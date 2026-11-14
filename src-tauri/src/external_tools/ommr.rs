use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use flate2::read::GzDecoder;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{ipc::Channel, AppHandle, Manager, State};
use tokio::{
    io::AsyncWriteExt,
    process::{Child, Command},
    sync::Mutex,
    time::{sleep, timeout},
};

use super::{
    bundle,
    downloads::ProcessTree,
    install::{Installation, Installer},
    InstallProgress,
};

const COMMIT: &str = "1d96b704625b4a64d55243131e72678fe0022ebd";
const SOURCE_HASH: &str = "3f1107685b6ecb4b31493ef37eed3419d482da5f4a2d25f0ff39800d88ac284d";
const GO: &str = "go1.26.11";
const DIRECTORY: &str = "ommr";
const PREVIOUS: &str = "ommr-previous";
const BINARY: &str = if cfg!(windows) {
    "ommr-server.exe"
} else {
    "ommr-server"
};
const LIMIT: u64 = 128 * 1024 * 1024;
const EXTRACTED_LIMIT: u64 = 768 * 1024 * 1024;
pub(super) static INSTALLER: Installer = Installer::new();

#[derive(Default)]
pub(crate) struct LocalServer(Mutex<Option<Running>>);

struct Running {
    child: Child,
    port: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Status {
    installed: bool,
    version: Option<String>,
    path: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Configuration {
    pub(crate) mode: Mode,
    pub(crate) url: Option<String>,
    pub(crate) sources: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Mode {
    Off,
    Local,
    Remote,
}

impl Configuration {
    pub(crate) fn sources(&self) -> Result<String, String> {
        const PROVIDERS: &[&str] = &[
            "applemusic",
            "deezer",
            "musicbrainz",
            concat!("yt", "music"),
        ];
        if self.sources.len() > PROVIDERS.len()
            || self
                .sources
                .iter()
                .any(|name| !PROVIDERS.contains(&name.as_str()))
        {
            return Err("Неизвестный источник метаданных OMMR.".into());
        }
        Ok(PROVIDERS
            .iter()
            .filter(|name| self.sources.iter().any(|source| source == **name))
            .copied()
            .collect::<Vec<_>>()
            .join(","))
    }
}

fn go_archive(os: &str, arch: &str) -> Option<(&'static str, &'static str)> {
    match (os, arch) {
        ("macos", "x86_64") => Some((
            "darwin-amd64.tar.gz",
            "394baae971332f2bc783d6e2004ae57101479bd3b06ed8209a3fe3570d4ce498",
        )),
        ("macos", "aarch64") => Some((
            "darwin-arm64.tar.gz",
            "e3a42e67a7a8acf450ebada2b074c7ca863000ca514907b7185b45e8f5cdff57",
        )),
        ("linux", "x86_64") => Some((
            "linux-amd64.tar.gz",
            "9849f4e04ee35303714e34fbc3ac8ab44ea3e448a95aa182f91e3b9b79c710ec",
        )),
        ("linux", "aarch64") => Some((
            "linux-arm64.tar.gz",
            "6c2f29b458b5add0835f18b776f69c8881a6d88428f1484a6da55755e620757e",
        )),
        ("windows", "x86_64") => Some((
            "windows-amd64.zip",
            "71ae086f0137e665f855a811cc119152b6a783e0bbc1b906e709f46d80141eb0",
        )),
        ("windows", "aarch64") => Some((
            "windows-arm64.zip",
            "a4c7e96d431ad186392a25e00db525839c6ee3840e80385e3c187e41e4f1f8d2",
        )),
        _ => None,
    }
}

fn failure(error: impl std::fmt::Display) -> String {
    eprintln!("OMMR tool: {error}");
    "Не удалось выполнить операцию с локальным OMMR.".into()
}

async fn tools(app: &AppHandle, create: bool) -> Result<Option<PathBuf>, String> {
    let data = app.path().app_data_dir().map_err(failure)?;
    bundle::tools_directory(&data, create)
        .await
        .map_err(failure)
}

async fn binary(tools: &Path) -> Result<Option<PathBuf>, String> {
    let folder = tools.join(DIRECTORY);
    bundle::restore(&folder, &tools.join(PREVIOUS))
        .await
        .map_err(failure)?;
    if !bundle::directory_exists(&folder).await.map_err(failure)? {
        return Ok(None);
    }
    let path = folder.join(BINARY);
    match tokio::fs::symlink_metadata(&path).await {
        Ok(metadata) if metadata.is_file() => Ok(Some(path)),
        Ok(_) => Err("Некорректная установка OMMR.".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(failure(error)),
    }
}

async fn status(tools: &Path) -> Result<Status, String> {
    let path = binary(tools).await?;
    let version = if let Some(path) = &path {
        let marker = path.with_file_name("revision");
        match tokio::fs::symlink_metadata(&marker).await {
            Ok(metadata) if metadata.is_file() && metadata.len() <= 40 => {
                tokio::fs::read_to_string(marker)
                    .await
                    .ok()
                    .map(|version| version.chars().take(12).collect())
            }
            _ => None,
        }
    } else {
        None
    };
    Ok(Status {
        installed: path.is_some(),
        path,
        version,
    })
}

#[tauri::command]
pub(crate) async fn check_ommr(
    app: AppHandle,
    server: State<'_, LocalServer>,
) -> Result<Status, String> {
    let _guard = server.0.lock().await;
    match tools(&app, false).await? {
        Some(tools) => status(&tools).await,
        None => Ok(Status {
            installed: false,
            version: None,
            path: None,
        }),
    }
}

#[tauri::command]
pub(crate) async fn install_ommr(
    app: AppHandle,
    server: State<'_, LocalServer>,
    id: String,
    on_progress: Channel<InstallProgress>,
) -> Result<Status, String> {
    let install = INSTALLER.start(id)?;
    let tools = tools(&app, true)
        .await?
        .ok_or("Папка инструментов YAP недоступна.")?;
    install_into(&tools, &server, &install, &on_progress).await
}

async fn install_into(
    tools: &Path,
    server: &LocalServer,
    install: &Installation<'_>,
    progress: &Channel<InstallProgress>,
) -> Result<Status, String> {
    let (asset, hash) = go_archive(std::env::consts::OS, std::env::consts::ARCH)
        .ok_or("Для этой платформы недоступна локальная сборка OMMR.")?;
    let stage = tempfile::tempdir_in(tools).map_err(failure)?;
    let go = stage.path().join("go.archive");
    let source = stage.path().join("source.tar.gz");
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(failure)?;
    download(
        &client,
        &format!("https://go.dev/dl/{GO}.{asset}"),
        hash,
        &go,
        install,
        progress,
    )
    .await?;
    download(
        &client,
        &format!(
            "https://codeload.github.com/Wilooper/OpenMusicMetadataResolver-OMMR/tar.gz/{COMMIT}"
        ),
        SOURCE_HASH,
        &source,
        install,
        progress,
    )
    .await?;
    phase(progress, "extracting");
    let workspace = stage.path().to_owned();
    let extracted = tauri::async_runtime::spawn_blocking(move || -> io::Result<()> {
        unpack(&go, &workspace, asset.ends_with(".zip"))?;
        unpack(&source, &workspace, false)?;
        patch_source(&workspace.join(format!(
            "OpenMusicMetadataResolver-OMMR-{COMMIT}/cmd/server/main.go"
        )))
    });
    extracted.await.map_err(failure)?.map_err(failure)?;
    phase(progress, "building");
    let root = stage.path();
    let published = root.join("install");
    tokio::fs::create_dir(&published).await.map_err(failure)?;
    let output = published.join(BINARY);
    let go_binary = root.join(if cfg!(windows) {
        "go/bin/go.exe"
    } else {
        "go/bin/go"
    });
    let mut go = command(&go_binary);
    go.current_dir(root.join(format!("OpenMusicMetadataResolver-OMMR-{COMMIT}")))
        .env("GOROOT", root.join("go"))
        .env("GOCACHE", root.join("build-cache"))
        .env("GOMODCACHE", root.join("module-cache"))
        .env("GO111MODULE", "on")
        .env("GOENV", "off")
        .env("GOWORK", "off")
        .env("GOFLAGS", "")
        .env("GOTOOLCHAIN", "local")
        .env("CGO_ENABLED", "0")
        .env_remove("GOOS")
        .env_remove("GOARCH")
        .env_remove("GOEXPERIMENT")
        .env_remove("GODEBUG")
        .env_remove("GOAMD64")
        .env_remove("GOARM64")
        .env_remove("GOTMPDIR")
        .env("GOPROXY", "https://proxy.golang.org")
        .env("GOSUMDB", "sum.golang.org")
        .env_remove("GOPRIVATE")
        .env_remove("GONOPROXY")
        .env_remove("GONOSUMDB")
        .env_remove("GOINSECURE")
        .args([
            "build",
            "-mod=readonly",
            "-trimpath",
            "-buildvcs=false",
            "-o",
        ])
        .arg(&output)
        .arg("./cmd/server");
    let build_log = root.join("build.log");
    go.stderr(Stdio::from(File::create(&build_log).map_err(failure)?));
    install
        .wait(async {
            let mut child = go.spawn().map_err(failure)?;
            let mut process = ProcessTree::attach(&child).map_err(failure)?;
            let exit = timeout(Duration::from_secs(600), child.wait())
                .await
                .map_err(failure)?
                .map_err(failure)?;
            process.completed();
            if exit.success() {
                return Ok(());
            }
            let log = tokio::fs::read_to_string(&build_log)
                .await
                .unwrap_or_default();
            eprintln!(
                "OMMR build ({exit}): {}",
                log.lines().rev().take(6).collect::<Vec<_>>().join("\n")
            );
            Err(build_failure(&log, exit))
        })
        .await?;
    tokio::fs::write(published.join("revision"), COMMIT)
        .await
        .map_err(failure)?;

    phase(progress, "checking");
    let probe = install.wait(start(&output, root)).await?;
    let mut probe = Some(probe);
    stop(&mut probe).await?;
    let mut running = install.wait(async { Ok(server.0.lock().await) }).await?;
    install.commit()?;
    phase(progress, "publishing");
    stop(&mut running).await?;
    bundle::publish(&published, &tools.join(DIRECTORY), &tools.join(PREVIOUS))
        .await
        .map_err(failure)?;
    let _ = bundle::remove_directory(&tools.join(PREVIOUS)).await;
    status(tools).await
}

fn build_failure(log: &str, exit: std::process::ExitStatus) -> String {
    let log = log.to_ascii_lowercase();
    let reason = if log.contains("no space left") || log.contains("not enough space") {
        "На диске закончилось свободное место."
    } else if log.contains("permission denied") || log.contains("access is denied") {
        "Системе не удалось получить доступ к файлам сборки."
    } else if log.contains("checksum mismatch") || log.contains("security error") {
        "Не удалось подтвердить контрольные суммы зависимостей."
    } else if log.lines().any(|line| {
        !line.starts_with("go: downloading")
            && (line.contains("proxy.golang.org") || line.contains("sum.golang.org"))
    }) {
        "Не удалось загрузить или проверить зависимости Go. Проверьте доступ к proxy.golang.org и sum.golang.org."
    } else {
        return format!("Не удалось собрать OMMR. Go завершился с ошибкой ({exit}).");
    };
    format!("Не удалось собрать OMMR. {reason}")
}

fn phase(progress: &Channel<InstallProgress>, status: &'static str) {
    let _ = progress.send(InstallProgress {
        status,
        downloaded: 0,
        total: None,
    });
}

async fn download(
    client: &Client,
    url: &str,
    expected: &str,
    path: &Path,
    install: &Installation<'_>,
    channel: &Channel<InstallProgress>,
) -> Result<(), String> {
    install
        .wait(async {
            let mut response = client
                .get(url)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
                .map_err(failure)?;
            let mut progress = InstallProgress {
                status: "downloading",
                downloaded: 0,
                total: response.content_length(),
            };
            if progress.total.is_some_and(|total| total > LIMIT) {
                return Err("Архив OMMR превышает допустимый размер.".into());
            }
            let _ = channel.send(progress);
            let mut hash = Sha256::new();
            let mut file = tokio::fs::File::create(path).await.map_err(failure)?;
            let mut reported = 0;
            while let Some(chunk) = response.chunk().await.map_err(failure)? {
                progress.downloaded += chunk.len() as u64;
                if progress.downloaded > LIMIT {
                    return Err("Архив OMMR превышает допустимый размер.".into());
                }
                hash.update(&chunk);
                file.write_all(&chunk).await.map_err(failure)?;
                if progress.downloaded - reported >= 256 * 1024 {
                    let _ = channel.send(progress);
                    reported = progress.downloaded;
                }
            }
            file.flush().await.map_err(failure)?;
            if format!("{:x}", hash.finalize()) != expected {
                return Err("Контрольная сумма архива OMMR или Go не совпала.".into());
            }
            let _ = channel.send(progress);
            Ok(())
        })
        .await
}

fn patch_source(main: &Path) -> io::Result<()> {
    let text = fs::read_to_string(main)?;
    let listener = "serverErrors <- srv.ListenAndServe()";
    let start = "flag.Parse()";
    if text.matches(listener).count() != 1 || text.matches(start).count() != 1 {
        return Err(io::Error::other(
            "Unexpected OMMR listener in pinned source",
        ));
    }
    // The upstream binary listens on all interfaces. Tell YAP the port only after binding it.
    let managed_listener = r#"listener, err := net.Listen("tcp4", "127.0.0.1:0")
        if err != nil {
            serverErrors <- err
            return
        }
        if err := os.WriteFile(os.Getenv("YAP_OMMR_READY"), []byte(listener.Addr().String()), 0600); err != nil {
            _ = listener.Close()
            serverErrors <- err
            return
        }
        serverErrors <- srv.Serve(listener)"#;
    let parent_watch = r#"flag.Parse()
    go func() {
        _, _ = io.Copy(io.Discard, os.Stdin)
        os.Exit(0)
    }()"#;
    fs::write(
        main,
        text.replace(r#""net/http""#, "\"io\"\n\t\"net\"\n\t\"net/http\"")
            .replace(start, parent_watch)
            .replace(listener, managed_listener),
    )
}

fn safe_path(path: &Path) -> io::Result<&Path> {
    if path
        .components()
        .all(|part| matches!(part, Component::Normal(_)))
    {
        Ok(path)
    } else {
        Err(io::Error::other("Invalid archive path"))
    }
}

fn unpack(path: &Path, root: &Path, zip: bool) -> io::Result<()> {
    let mut remaining = EXTRACTED_LIMIT;
    let mut write = |path: &Path, mode: u32, size: u64, input: &mut dyn Read| -> io::Result<()> {
        remaining = remaining
            .checked_sub(size)
            .ok_or_else(|| io::Error::other("Archive too large"))?;
        let path = root.join(safe_path(path)?);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        io::copy(&mut input.take(size), &mut file)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600 | mode & 0o100))?;
        }
        #[cfg(not(unix))]
        let _ = mode;
        Ok(())
    };
    if zip {
        let mut archive = zip::ZipArchive::new(File::open(path)?)?;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            if entry.is_symlink() {
                return Err(io::Error::other("Unexpected archive link"));
            }
            if !entry.is_dir() {
                let path = entry
                    .enclosed_name()
                    .ok_or_else(|| io::Error::other("Invalid archive path"))?;
                write(
                    &path,
                    entry.unix_mode().unwrap_or_default(),
                    entry.size(),
                    &mut entry,
                )?;
            }
        }
    } else {
        let mut archive = tar::Archive::new(GzDecoder::new(File::open(path)?));
        for entry in archive.entries()? {
            let mut entry = entry?;
            let kind = entry.header().entry_type();
            if kind.is_file() {
                let path = entry.path()?.into_owned();
                write(&path, entry.header().mode()?, entry.size(), &mut entry)?;
            } else if !kind.is_dir() && !kind.is_pax_global_extensions() {
                return Err(io::Error::other(format!(
                    "Unexpected archive entry type: {kind:?}"
                )));
            }
        }
    }
    Ok(())
}

fn command(path: &Path) -> Command {
    let mut cmd = Command::new(path);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    cmd
}

async fn start(binary: &Path, work: &Path) -> Result<Running, String> {
    let ready = tempfile::NamedTempFile::new_in(work)
        .map_err(failure)?
        .into_temp_path();
    let mut command = command(binary);
    for (key, _) in std::env::vars_os().filter(|(key, _)| {
        key.to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("OMMR_")
    }) {
        command.env_remove(key);
    }
    command
        .current_dir(work)
        .stdin(Stdio::piped())
        .env("YAP_OMMR_READY", &*ready);
    let mut child = command.spawn().map_err(failure)?;
    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(failure)?;
    timeout(Duration::from_secs(55), async {
        loop {
            if child.try_wait().map_err(failure)?.is_some() {
                return Err("Локальный OMMR завершился при запуске.".into());
            }
            let address = tokio::fs::read_to_string(&ready).await.map_err(failure)?;
            if let Ok(address) = address.parse::<std::net::SocketAddrV4>() {
                if address.ip().is_loopback()
                    && address.port() > 0
                    && health(&client, &format!("http://{address}")).await.is_ok()
                    && child.try_wait().map_err(failure)?.is_none()
                {
                    return Ok(Running {
                        child,
                        port: address.port(),
                    });
                }
            }
            sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .map_err(|_| "Локальный OMMR не запустился вовремя.".to_string())?
}

async fn stop(running: &mut Option<Running>) -> Result<(), String> {
    if let Some(mut running) = running.take() {
        if running.child.try_wait().map_err(failure)?.is_none() {
            running.child.kill().await.map_err(failure)?;
        }
    }
    Ok(())
}

impl LocalServer {
    pub(crate) async fn endpoint(&self, app: &AppHandle) -> Result<Option<String>, String> {
        let mut state = self.0.lock().await;
        if let Some(running) = state.as_mut() {
            if running.child.try_wait().map_err(failure)?.is_none() {
                return Ok(Some(format!("http://127.0.0.1:{}", running.port)));
            }
            *state = None;
        }
        let Some(tools) = tools(app, false).await? else {
            return Ok(None);
        };
        let Some(binary) = binary(&tools).await? else {
            return Ok(None);
        };
        let work = tools.join("ommr-data");
        tokio::fs::create_dir_all(&work).await.map_err(failure)?;
        bundle::directory_exists(&work).await.map_err(failure)?;
        let running = start(&binary, &work).await?;
        let endpoint = format!("http://127.0.0.1:{}", running.port);
        *state = Some(running);
        Ok(Some(endpoint))
    }

    pub(crate) fn shutdown(&self) {
        if let Ok(mut running) = self.0.try_lock() {
            if let Some(mut server) = running.take() {
                let _ = server.child.start_kill();
            }
        }
    }
}

pub(super) async fn remove(app: &AppHandle) -> Result<(), String> {
    let _installation = INSTALLER.start("remove".into())?;
    let server = app.state::<LocalServer>();
    let mut running = server.0.lock().await;
    stop(&mut running).await?;
    if let Some(tools) = tools(app, false).await? {
        bundle::remove_directory(&tools.join(PREVIOUS))
            .await
            .map_err(failure)?;
        bundle::remove_directory(&tools.join(DIRECTORY))
            .await
            .map_err(failure)?;
    }
    Ok(())
}

pub(crate) fn normalize_url(value: &str) -> Result<String, String> {
    let parsed = Url::parse(value.trim()).map_err(|_| "Введите корректный адрес OMMR.")?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("Введите корректный адрес OMMR.".into());
    }
    Ok(parsed.as_str().trim_end_matches('/').to_owned())
}

async fn health(client: &Client, endpoint: &str) -> Result<(), String> {
    let mut response = client
        .get(format!("{endpoint}/v1/health"))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| {
            eprintln!("OMMR health: {error}");
            "Не удалось подключиться к серверу OMMR."
        })?;
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(failure)? {
        if body.len() + chunk.len() > 64 * 1024 {
            return Err("Некорректный ответ OMMR.".into());
        }
        body.extend_from_slice(&chunk);
    }
    let health: serde_json::Value = serde_json::from_slice(&body).map_err(failure)?;
    if matches!(health["status"].as_str(), Some("healthy" | "degraded"))
        && health["version"].is_string()
        && health["cache_provider"].is_string()
    {
        Ok(())
    } else {
        Err("Адрес не отвечает как сервер OMMR.".into())
    }
}

#[tauri::command]
pub(crate) async fn test_ommr(
    app: AppHandle,
    server: State<'_, LocalServer>,
    config: Configuration,
) -> Result<(), String> {
    config.sources()?;
    match config.mode {
        Mode::Off => Err("Выберите сервер OMMR.".into()),
        Mode::Local => server
            .endpoint(&app)
            .await?
            .map(|_| ())
            .ok_or("Сначала установите OMMR.".into()),
        Mode::Remote => {
            let endpoint = normalize_url(config.url.as_deref().unwrap_or_default())?;
            let client = Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(10))
                .build()
                .map_err(failure)?;
            health(&client, &endpoint).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_server_paths_and_rejects_credentials_and_query_parameters() {
        assert_eq!(
            normalize_url(" https://example.com/ommr/ ").unwrap(),
            "https://example.com/ommr"
        );
        assert!(normalize_url("https://name:password@example.com/").is_err());
        assert!(normalize_url("https://example.com/?key=value").is_err());
        assert!(normalize_url("file:///tmp/ommr").is_err());
    }

    #[test]
    fn metadata_sources_use_provider_order_and_reject_unknown_values() {
        let mut config = Configuration {
            mode: Mode::Local,
            url: None,
            sources: vec![concat!("yt", "music").into(), "applemusic".into()],
        };
        assert_eq!(config.sources().unwrap(), concat!("applemusic,yt", "music"));
        config.sources.push("invalid".into());
        assert!(config.sources().is_err());
    }

    #[tokio::test]
    #[ignore = "downloads Go and builds the pinned OMMR server"]
    async fn installs_and_checks_real_local_server_in_a_temporary_directory() {
        let directory = tempfile::tempdir().unwrap();
        let installer = Installer::new();
        let install = installer.start("integration".into()).unwrap();
        let server = LocalServer::default();
        let progress = Channel::new(|_| Ok(()));
        let status = install_into(directory.path(), &server, &install, &progress)
            .await
            .unwrap();
        assert!(status.installed);
        assert_eq!(status.version.as_deref(), Some(&COMMIT[..12]));
        let mut running = Some(
            start(&status.path.unwrap(), directory.path())
                .await
                .unwrap(),
        );
        let port = running.as_ref().unwrap().port;
        assert!(port > 0);
        stop(&mut running).await.unwrap();
        assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err());
    }
}
