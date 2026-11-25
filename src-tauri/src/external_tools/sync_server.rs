use std::{
    fs,
    io::{self, Read, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
    process::Stdio,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{ipc::Channel, AppHandle, Manager, State};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::Mutex,
    time::timeout,
};

use super::{bundle, install::Installer, InstallProgress};
use crate::library::{
    sync::{device_auth, discovery::primary_ipv4, transport::Invite},
    Library,
};

const DIRECTORY: &str = "yap-sync-server";
const PREVIOUS: &str = "yap-sync-server-previous";
const RELEASE: &str = "https://github.com/BADtochka/yap-sync-server/releases/download/v0.2.0";
const RELEASE_CHECKSUMS: &str = include_str!("sync_server_release.sha256");
const BINARY: &str = if cfg!(windows) {
    "yap-sync-server.exe"
} else {
    "yap-sync-server"
};
const MAX_BINARY: u64 = 64 * 1024 * 1024;
const PORT: u16 = 38737;
const PROTOCOL_MARKER: &str = "allow-list-v2";
const ENABLED_FILE: &str = "sync-server-enabled";
static INSTALLER: Installer = Installer::new();

#[derive(Default)]
pub(crate) struct LocalServer {
    managed: Mutex<Managed>,
    exiting: AtomicBool,
}

#[derive(Default)]
struct Managed {
    running: Option<Running>,
    retry_at: Option<Instant>,
    error: Option<String>,
}

struct Running {
    child: Child,
    invite: String,
    address: SocketAddr,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceAccess {
    id: String,
    name: String,
    status: String,
    local: bool,
    seen_at: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Status {
    installed: bool,
    address: Option<SocketAddr>,
    asset: Option<String>,
    enabled: bool,
    needs_update: bool,
    error: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct InstallSource {
    location: String,
    checksum: String,
    local: bool,
}

fn official_source() -> Result<InstallSource, String> {
    let asset = asset().ok_or("Для этой платформы нет готового файла сервера.")?;
    let checksum = bundle::archive_checksum(RELEASE_CHECKSUMS, &asset)
        .ok_or("Отсутствует контрольная сумма официального файла сервера.")?;
    Ok(InstallSource {
        location: format!("{RELEASE}/{asset}"),
        checksum,
        local: false,
    })
}

fn tool_error(error: impl std::fmt::Display) -> String {
    eprintln!("YAP Sync server: {error}");
    "Операция с сервером синхронизации не завершилась.".into()
}

fn asset() -> Option<String> {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "linux" => "linux",
        "windows" => "windows",
        _ => return None,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        _ => return None,
    };
    let suffix = if os == "windows" { ".exe" } else { "" };
    Some(format!("{DIRECTORY}-{os}-{arch}{suffix}"))
}

fn primary_address() -> SocketAddr {
    primary_ipv4()
        .map(|ip| SocketAddr::from((ip, PORT)))
        .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], PORT)))
}

fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_data_dir().map_err(tool_error)
}

async fn enabled(app: &AppHandle) -> Result<Option<bool>, String> {
    match tokio::fs::read(data_dir(app)?.join(ENABLED_FILE)).await {
        Ok(value) if value == b"1" => Ok(Some(true)),
        Ok(value) if value == b"0" => Ok(Some(false)),
        Ok(_) => Err("Настройки запуска сервера повреждены.".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(tool_error(error)),
    }
}

fn save_enabled(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let data = data_dir(app)?;
    let mut file = tempfile::NamedTempFile::new_in(&data).map_err(tool_error)?;
    file.write_all(if enabled { b"1" } else { b"0" })
        .map_err(tool_error)?;
    file.persist(data.join(ENABLED_FILE)).map_err(tool_error)?;
    Ok(())
}

async fn up_to_date(binary: &Path) -> bool {
    let Some(directory) = binary.parent() else {
        return false;
    };
    tokio::fs::read(directory.join(PROTOCOL_MARKER))
        .await
        .is_ok_and(|value| value == b"2")
}

async fn binary(app: &AppHandle) -> Result<Option<PathBuf>, String> {
    let data = app.path().app_data_dir().map_err(tool_error)?;
    let Some(tools) = bundle::tools_directory(&data, false)
        .await
        .map_err(tool_error)?
    else {
        return Ok(None);
    };
    let directory = tools.join(DIRECTORY);
    bundle::restore(&directory, &tools.join(PREVIOUS))
        .await
        .map_err(tool_error)?;
    if !bundle::directory_exists(&directory)
        .await
        .map_err(tool_error)?
    {
        return Ok(None);
    }
    let file = directory.join(BINARY);
    match tokio::fs::symlink_metadata(&file).await {
        Ok(meta) if meta.is_file() && meta.len() <= MAX_BINARY => Ok(Some(file)),
        Ok(_) => Err("Файл сервера в папке инструментов повреждён.".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(tool_error(error)),
    }
}

async fn status(app: &AppHandle, managed: &mut Managed) -> Result<Status, String> {
    if let Some(server) = managed.running.as_mut() {
        if server.child.try_wait().map_err(tool_error)?.is_some() {
            managed.running = None;
        }
    }
    let binary = binary(app).await?;
    let needs_update = match binary.as_deref() {
        Some(path) => !up_to_date(path).await,
        None => false,
    };
    Ok(Status {
        installed: binary.is_some(),
        address: managed.running.as_ref().map(|server| server.address),
        enabled: enabled(app).await?.unwrap_or(false),
        needs_update,
        error: managed.error.clone(),
        asset: asset(),
    })
}

#[tauri::command]
pub(crate) async fn sync_server_status(
    app: AppHandle,
    server: State<'_, LocalServer>,
) -> Result<Status, String> {
    let mut managed = server.managed.lock().await;
    status(&app, &mut managed).await
}

fn release_url(input: &str, asset: &str) -> Result<Url, String> {
    let invalid = || format!("Укажите ссылку на файл {asset} из GitHub Releases.");
    let url = Url::parse(input).map_err(|_| invalid())?;
    let base_valid = url.scheme() == "https"
        && url.host_str() == Some("github.com")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.query().is_none()
        && url.fragment().is_none();
    let parts: Vec<_> = url.path_segments().ok_or_else(invalid)?.collect();
    if !base_valid
        || !matches!(&parts[..], [owner, repo, "releases", "download", tag, filename]
        if !owner.is_empty() && !repo.is_empty() && !tag.is_empty() && *filename == asset)
    {
        return Err(invalid());
    }
    Ok(url)
}

fn command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    command
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command
}

fn copy_local(source: &Path, output: &Path, expected: &str) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_BINARY {
        return Err(io::Error::other("Invalid server binary"));
    }
    let mut source = fs::File::open(source)?.take(MAX_BINARY + 1);
    let mut temporary = fs::File::create(output)?;
    let mut hash = Sha256::new();
    let mut total = 0;
    let mut buffer = vec![0; 128 * 1024];
    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > MAX_BINARY {
            return Err(io::Error::other("Server binary is too large"));
        }
        hash.update(&buffer[..read]);
        temporary.write_all(&buffer[..read])?;
    }
    if format!("{:x}", hash.finalize()) != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Server binary checksum mismatch",
        ));
    }
    temporary.sync_all()
}

#[tauri::command]
pub(crate) async fn install_sync_server(
    app: AppHandle,
    server: State<'_, LocalServer>,
    source: Option<InstallSource>,
    progress: Channel<InstallProgress>,
) -> Result<Status, String> {
    let source = match source {
        Some(source) => source,
        None => official_source()?,
    };
    let install = INSTALLER.start("yap-sync-server".into())?;
    let checksum = source.checksum.trim().to_ascii_lowercase();
    if checksum.len() != 64 || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Укажите SHA-256 скачанного файла.".into());
    }
    let data = app.path().app_data_dir().map_err(tool_error)?;
    let tools = bundle::tools_directory(&data, true)
        .await
        .map_err(tool_error)?
        .ok_or("Папка инструментов недоступна.")?;
    let staging = tempfile::tempdir_in(&tools).map_err(tool_error)?;
    let file = staging.path().join(BINARY);
    let send = |phase| {
        let _ = progress.send(InstallProgress {
            status: phase,
            downloaded: 0,
            total: None,
        });
    };
    if source.local {
        let source = PathBuf::from(source.location);
        if !source.is_absolute() {
            return Err("Выберите файл сервера.".into());
        }
        send("checking");
        let output = file.clone();
        tauri::async_runtime::spawn_blocking(move || copy_local(&source, &output, &checksum))
            .await
            .map_err(tool_error)?
            .map_err(|error| {
                if error.kind() == io::ErrorKind::InvalidData {
                    "SHA-256 файла не совпадает с указанным.".into()
                } else {
                    tool_error(error)
                }
            })?;
    } else {
        let asset = asset().ok_or("Для этой платформы нет готового файла сервера.")?;
        let url = release_url(source.location.trim(), &asset)?;
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .https_only(true)
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() > 5
                    || attempt.url().scheme() != "https"
                    || !matches!(
                        attempt.url().host_str(),
                        Some(
                            "github.com"
                                | "release-assets.githubusercontent.com"
                                | "objects.githubusercontent.com"
                        )
                    )
                {
                    attempt.stop()
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(tool_error)?;
        bundle::download_verified(
            &client,
            url.as_str(),
            &checksum,
            &file,
            MAX_BINARY,
            &progress,
        )
        .await
        .map_err(|error| {
            if error.to_string().contains("checksum mismatch") {
                "SHA-256 скачанного файла не совпадает с указанным.".into()
            } else {
                tool_error(error)
            }
        })?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&file, fs::Permissions::from_mode(0o700))
            .await
            .map_err(tool_error)?;
    }
    send("checking");
    let version = timeout(Duration::from_secs(5), async {
        let mut child = command(&file)
            .arg("--version")
            .stdout(Stdio::piped())
            .spawn()?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("No version output"))?;
        let mut version = Vec::new();
        output.take(128).read_to_end(&mut version).await?;
        if child.wait().await?.success() && version.trim_ascii() == b"yap-sync-server 2" {
            Ok(())
        } else {
            Err(io::Error::other("Unsupported server version"))
        }
    })
    .await;
    if !matches!(version, Ok(Ok(()))) {
        return Err("Файл не запускается или использует несовместимый протокол YAP Sync.".into());
    }
    tokio::fs::write(staging.path().join(PROTOCOL_MARKER), b"2")
        .await
        .map_err(tool_error)?;
    let mut managed = server.managed.lock().await;
    let previous_enabled = enabled(&app).await?;
    install.commit()?;
    stop(&mut managed).await?;
    bundle::publish(
        staging.path(),
        &tools.join(DIRECTORY),
        &tools.join(PREVIOUS),
    )
    .await
    .map_err(tool_error)?;
    if previous_enabled.is_none() {
        save_enabled(&app, true)?;
    }
    managed.retry_at = None;
    server.reconcile(&app, &mut managed).await?;
    status(&app, &mut managed).await
}

async fn start_sync_server(
    app: &AppHandle,
    socket: SocketAddr,
    executable: &Path,
) -> Result<Running, String> {
    let data = data_dir(app)?.join("sync-server-data");
    match tokio::fs::symlink_metadata(&data).await {
        Ok(meta) if !meta.is_dir() => return Err("Папка данных сервера повреждена.".into()),
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(tool_error(error)),
        _ => {}
    }
    let database = app.state::<Library>().database_path().to_owned();
    let profile = tauri::async_runtime::spawn_blocking(move || device_auth(&database))
        .await
        .map_err(tool_error)?
        .map_err(tool_error)?;
    let profile = serde_json::to_vec(&profile).map_err(tool_error)?;
    let mut child = command(executable)
        .arg("--listen")
        .arg(socket.to_string())
        .arg("--data")
        .arg(&data)
        .arg("--managed")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(tool_error)?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or("Не удалось передать ключ компьютера серверу.")?;
    stdin.write_all(&profile).await.map_err(tool_error)?;
    drop(stdin);
    let stdout = child
        .stdout
        .take()
        .ok_or("Не удалось прочитать код подключения.")?;
    let code = timeout(Duration::from_secs(6), async {
        let mut code = String::new();
        BufReader::new(stdout.take(8193))
            .read_line(&mut code)
            .await?;
        Ok::<_, io::Error>(code)
    })
    .await
    .map_err(|_| "Сервер не ответил при запуске.")?
    .map_err(tool_error)?;
    let invite = Invite::parse(&code)
        .map_err(|_| "Сервер не запустился. Проверьте, что порт 38737 свободен.")?;
    if invite.address != socket || !invite.modern() {
        return Err("Сервер вернул несовместимые параметры подключения.".into());
    }
    Ok(Running {
        child,
        invite: code.trim().to_owned(),
        address: socket,
    })
}

async fn stop(managed: &mut Managed) -> Result<(), String> {
    if let Some(mut running) = managed.running.take() {
        if running.child.try_wait().map_err(tool_error)?.is_none() {
            running.child.kill().await.map_err(tool_error)?;
        }
    }
    Ok(())
}

impl LocalServer {
    async fn reconcile(&self, app: &AppHandle, managed: &mut Managed) -> Result<(), String> {
        if self.exiting.load(Ordering::Relaxed) || !enabled(app).await?.unwrap_or(false) {
            managed.error = None;
            return stop(managed).await;
        }
        if let Some(running) = managed.running.as_mut() {
            if running.child.try_wait().map_err(tool_error)?.is_some() {
                managed.running = None;
            }
        }
        let address = primary_address();
        if managed
            .running
            .as_ref()
            .is_some_and(|running| running.address == address)
        {
            return Ok(());
        }
        if managed.retry_at.is_some_and(|retry| retry > Instant::now()) {
            return Ok(());
        }
        let Some(executable) = binary(app).await? else {
            return Ok(());
        };
        if !up_to_date(&executable).await {
            return stop(managed).await;
        }
        stop(managed).await?;
        match start_sync_server(app, address, &executable).await {
            Ok(running) if !self.exiting.load(Ordering::Relaxed) => {
                managed.running = Some(running);
                managed.error = None;
                managed.retry_at = None;
            }
            Ok(_) => {}
            Err(error) => {
                managed.error = Some(error);
                managed.retry_at = Some(Instant::now() + Duration::from_secs(5));
            }
        }
        Ok(())
    }

    pub(crate) fn supervise(app: AppHandle) {
        tauri::async_runtime::spawn(async move {
            let server = app.state::<LocalServer>();
            while !server.exiting.load(Ordering::Relaxed) {
                {
                    let mut managed = server.managed.lock().await;
                    if let Err(error) = server.reconcile(&app, &mut managed).await {
                        managed.error = Some(error);
                    }
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
    }

    pub(crate) fn shutdown(&self) {
        self.exiting.store(true, Ordering::Relaxed);
        if let Ok(mut managed) = self.managed.try_lock() {
            managed.running.take();
        }
    }
}

#[tauri::command]
pub(crate) async fn sync_server_set_enabled(
    app: AppHandle,
    server: State<'_, LocalServer>,
    enabled: bool,
) -> Result<Status, String> {
    let mut managed = server.managed.lock().await;
    save_enabled(&app, enabled)?;
    managed.retry_at = None;
    server.reconcile(&app, &mut managed).await?;
    status(&app, &mut managed).await
}

async fn admin_client(
    app: &AppHandle,
    server: &LocalServer,
) -> Result<Option<(Client, Invite, String)>, String> {
    #[derive(Deserialize)]
    struct Identity {
        admin: String,
    }
    let Some(code) = server
        .managed
        .lock()
        .await
        .running
        .as_ref()
        .map(|running| running.invite.clone())
    else {
        return Ok(None);
    };
    let stored = tokio::fs::read(data_dir(app)?.join("sync-server-data/identity.json"))
        .await
        .map_err(tool_error)?;
    let identity: Identity = serde_json::from_slice(&stored).map_err(tool_error)?;
    if identity.admin.len() != 64 || !identity.admin.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Ключ управления сервером повреждён.".into());
    }
    let invite = Invite::parse(&code).map_err(tool_error)?;
    let client = Client::builder()
        .timeout(Duration::from_secs(3))
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .tls_built_in_root_certs(false)
        .add_root_certificate(invite.certificate().map_err(tool_error)?)
        .build()
        .map_err(tool_error)?;
    Ok(Some((client, invite, identity.admin)))
}

#[tauri::command]
pub(crate) async fn sync_server_devices(
    app: AppHandle,
    server: State<'_, LocalServer>,
) -> Result<Vec<DeviceAccess>, String> {
    let Some((client, invite, admin)) = admin_client(&app, &server).await? else {
        return Ok(Vec::new());
    };
    let reply = client
        .get(invite.url("/admin/devices"))
        .bearer_auth(admin)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(tool_error)?;
    if reply
        .content_length()
        .is_some_and(|bytes| bytes > 128 * 1024)
    {
        return Err("Список устройств слишком большой.".into());
    }
    let data = reply.bytes().await.map_err(tool_error)?;
    if data.len() > 128 * 1024 {
        return Err("Список устройств слишком большой.".into());
    }
    serde_json::from_slice(&data).map_err(tool_error)
}

#[tauri::command]
pub(crate) async fn sync_server_set_access(
    app: AppHandle,
    server: State<'_, LocalServer>,
    id: String,
    allowed: bool,
) -> Result<(), String> {
    if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Недопустимый идентификатор устройства.".into());
    }
    let (client, invite, admin) = admin_client(&app, &server)
        .await?
        .ok_or("Сервер выключен.")?;
    let body =
        serde_json::json!({ "status": if allowed { "allowed" } else { "blocked" } }).to_string();
    client
        .put(invite.url(&format!("/admin/devices/{id}")))
        .bearer_auth(admin)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(tool_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_release_pins_the_asset_for_each_supported_platform() {
        let assets = [
            "yap-sync-server-darwin-amd64",
            "yap-sync-server-darwin-arm64",
            "yap-sync-server-linux-amd64",
            "yap-sync-server-linux-arm64",
            "yap-sync-server-windows-amd64.exe",
            "yap-sync-server-windows-arm64.exe",
        ];
        for name in assets {
            assert!(bundle::archive_checksum(RELEASE_CHECKSUMS, name).is_some());
        }
        if let Some(name) = asset() {
            let source = official_source().unwrap();
            assert_eq!(source.location, format!("{RELEASE}/{name}"));
            assert!(release_url(&source.location, &name).is_ok());
        }
    }

    #[test]
    fn accepts_only_the_expected_github_release_asset() {
        let asset = "yap-sync-server-darwin-arm64";
        assert!(release_url(
            "https://github.com/owner/project/releases/download/v1/yap-sync-server-darwin-arm64",
            asset
        )
        .is_ok());
        for invalid in [
            "https://example.org/owner/project/releases/download/v1/yap-sync-server-darwin-arm64",
            "https://github.com/owner/project/releases/download/v1/yap-sync-server-linux-amd64",
            "http://github.com/owner/project/releases/download/v1/yap-sync-server-darwin-arm64",
        ] {
            assert!(release_url(invalid, asset).is_err());
        }
    }
}
