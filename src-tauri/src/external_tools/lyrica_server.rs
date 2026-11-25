use std::{
    env, fs, io,
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use flate2::read::GzDecoder;
use reqwest::{redirect::Policy, Client};
use serde::Serialize;
use tauri::{ipc::Channel, AppHandle, Manager, State};
use tokio::{
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

const VERSION: &str = "1.5.0";
const SOURCE_URL: &str =
    "https://codeload.github.com/Wilooper/Lyrica/tar.gz/d70c11c466d54789aca6585530a533e0160b7298";
const SOURCE_HASH: &str = "8a4a180be751eac19415e471da296411d2022a7b5a9ce7429dd4d81efdb23795";
const PYTHON_RELEASE: &str = "20261110";
const PYTHON_VERSION: &str = "3.12.16";
const DIRECTORY: &str = "lyrica";
const PREVIOUS: &str = "lyrica-previous";
const LIMIT: u64 = 128 * 1024 * 1024;
const LAUNCHER: &str = "yap_lyrica.py";
const KEY_HEADER: &str = "X-YAP-Lyrica-Key";
pub(super) static INSTALLER: Installer = Installer::new();

#[derive(Default)]
pub(crate) struct LocalLyrica(Mutex<Option<Running>>);

#[derive(Clone)]
pub(crate) struct Endpoint {
    port: u16,
    key: String,
}

impl Endpoint {
    pub(crate) fn get(&self, client: &Client, path: &str) -> reqwest::RequestBuilder {
        client
            .get(format!("http://127.0.0.1:{}{path}", self.port))
            .header(KEY_HEADER, &self.key)
    }
}

struct Running {
    child: Child,
    endpoint: Endpoint,
}

#[derive(Serialize)]
pub(crate) struct Status {
    installed: bool,
    version: Option<String>,
    path: Option<PathBuf>,
}

fn python_asset(os: &str, arch: &str) -> Option<(&'static str, &'static str)> {
    match (os, arch) {
        ("macos", "aarch64") => Some((
            "aarch64-apple-darwin",
            "e956e60bc9465d577ef8062059aa05cd3a2665b4b8678a2ed24e432368330b8f",
        )),
        ("macos", "x86_64") => Some((
            "x86_64-apple-darwin",
            "83f2922437e6d546d0fcdb8c11f150a8e52787e8d9ea8651b1974e20908eb1bc",
        )),
        ("linux", "aarch64") => Some((
            "aarch64-unknown-linux-gnu",
            "4792f81750282597fb2a3389fe18b945ed2ba6aad5e45ad406d122c58cb33081",
        )),
        ("linux", "x86_64") => Some((
            "x86_64-unknown-linux-gnu",
            "c2f87d4b4e8db3abddb0f116c6ccfc503d0cbdbb9ee8faacddad1b679ec12e99",
        )),
        ("windows", "aarch64") => Some((
            "aarch64-pc-windows-msvc",
            "7122bf62a0cf601527ba62e19ca715d64db86b269765da23937e9ac7f968f39c",
        )),
        ("windows", "x86_64") => Some((
            "x86_64-pc-windows-msvc",
            "e23fe5b16926de1d81661b1a97352e89fb167549c55a6825796ccd0f32ec7f1b",
        )),
        _ => None,
    }
}

fn failure(error: impl std::fmt::Display) -> String {
    eprintln!("Lyrica: {error}");
    "Не удалось выполнить операцию с локальной Lyrica.".into()
}

async fn tools(app: &AppHandle, create: bool) -> Result<Option<PathBuf>, String> {
    let data = app.path().app_data_dir().map_err(failure)?;
    bundle::tools_directory(&data, create)
        .await
        .map_err(failure)
}

fn python(root: &Path) -> PathBuf {
    root.join(if cfg!(windows) {
        "python/python.exe"
    } else {
        "python/bin/python3.12"
    })
}

async fn status(tools: &Path) -> Result<Status, String> {
    let root = tools.join(DIRECTORY);
    bundle::restore(&root, &tools.join(PREVIOUS))
        .await
        .map_err(failure)?;
    let installed = bundle::directory_exists(&root).await.map_err(failure)?
        && tokio::fs::symlink_metadata(python(&root))
            .await
            .is_ok_and(|meta| meta.is_file())
        && tokio::fs::symlink_metadata(root.join(LAUNCHER))
            .await
            .is_ok_and(|meta| meta.is_file());
    Ok(Status {
        installed,
        version: installed.then(|| VERSION.to_owned()),
        path: installed.then_some(root),
    })
}

#[tauri::command]
pub(crate) async fn check_lyrica(
    app: AppHandle,
    server: State<'_, LocalLyrica>,
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
pub(crate) async fn install_lyrica(
    app: AppHandle,
    server: State<'_, LocalLyrica>,
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
    server: &LocalLyrica,
    install: &Installation<'_>,
    progress: &Channel<InstallProgress>,
) -> Result<Status, String> {
    let (target, checksum) = python_asset(env::consts::OS, env::consts::ARCH)
        .ok_or("Для этой платформы локальная Lyrica недоступна.")?;
    let stage = tempfile::tempdir_in(tools).map_err(failure)?;
    let python_archive = stage.path().join("python.tar.gz");
    let source_archive = stage.path().join("source.tar.gz");
    let root = stage.path().join("install");
    let url = format!("https://github.com/astral-sh/python-build-standalone/releases/download/{PYTHON_RELEASE}/cpython-{PYTHON_VERSION}%2B{PYTHON_RELEASE}-{target}-install_only.tar.gz");
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(240))
        .build()
        .map_err(failure)?;
    for (url, checksum, path) in [
        (url.as_str(), checksum, &python_archive),
        (SOURCE_URL, SOURCE_HASH, &source_archive),
    ] {
        install
            .wait(async {
                bundle::download_verified(&client, url, checksum, path, LIMIT, progress)
                    .await
                    .map_err(failure)
            })
            .await?;
    }

    phase(progress, "extracting");
    let destination = root.clone();
    tauri::async_runtime::spawn_blocking(move || -> io::Result<()> {
        fs::create_dir(&destination)?;
        unpack(&python_archive, &destination)?;
        let source = destination.join("app");
        fs::create_dir(&source)?;
        unpack_source(&source_archive, &source)?;
        fs::write(
            destination.join(LAUNCHER),
            include_bytes!("lyrica_launcher.py"),
        )?;
        fs::write(destination.join("lyrica.ini"), "")?;
        Ok(())
    })
    .await
    .map_err(failure)?
    .map_err(failure)?;

    phase(progress, "building");
    install.wait(install_dependencies(&root)).await?;
    phase(progress, "checking");
    let probe = install.wait(start(&root)).await?;
    stop(&mut Some(probe)).await?;

    let mut running = install.wait(async { Ok(server.0.lock().await) }).await?;
    install.commit()?;
    phase(progress, "publishing");
    stop(&mut running).await?;
    bundle::publish(&root, &tools.join(DIRECTORY), &tools.join(PREVIOUS))
        .await
        .map_err(failure)?;
    let _ = bundle::remove_directory(&tools.join(PREVIOUS)).await;
    status(tools).await
}

fn phase(progress: &Channel<InstallProgress>, status: &'static str) {
    let _ = progress.send(InstallProgress {
        status,
        downloaded: 0,
        total: None,
    });
}

fn unpack(archive: &Path, destination: &Path) -> io::Result<()> {
    let mut tar = tar::Archive::new(GzDecoder::new(fs::File::open(archive)?));
    let mut remaining = 512 * 1024 * 1024_u64;
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        if !path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        {
            return Err(io::Error::other("Invalid archive path"));
        }
        if let Some(target) = entry.link_name()? {
            if !target
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
            {
                return Err(io::Error::other("Invalid archive link"));
            }
        }
        remaining = remaining
            .checked_sub(entry.size())
            .ok_or_else(|| io::Error::other("Extracted archive is too large"))?;
        if !entry.unpack_in(destination)? {
            return Err(io::Error::other(
                "Archive entry left installation directory",
            ));
        }
    }
    Ok(())
}

fn unpack_source(archive: &Path, destination: &Path) -> io::Result<()> {
    unpack(archive, destination)?;
    let mut folders = fs::read_dir(destination)?;
    let source = folders
        .next()
        .transpose()?
        .ok_or_else(|| io::Error::other("Missing Lyrica source"))?;
    if !source.file_type()?.is_dir() || folders.next().is_some() {
        return Err(io::Error::other("Invalid Lyrica source archive"));
    }
    for entry in fs::read_dir(source.path())? {
        let entry = entry?;
        fs::rename(entry.path(), destination.join(entry.file_name()))?;
    }
    fs::remove_dir(source.path())
}

fn command(root: &Path) -> Command {
    let mut command = Command::new(python(root));
    command
        .current_dir(root)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .env("TEMP", env::temp_dir())
        .env("TMP", env::temp_dir());
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(windows)]
    {
        command.creation_flags(0x08000000);
        for name in ["SystemRoot", "USERPROFILE"] {
            if let Some(value) = env::var_os(name) {
                command.env(name, value);
            }
        }
    }
    command
}

async fn install_dependencies(root: &Path) -> Result<(), String> {
    let requirements = tokio::fs::read_to_string(root.join("app/requirements.txt"))
        .await
        .map_err(failure)?;
    let requirements = requirements
        .lines()
        .filter(|line| {
            !["gunicorn", "gevent", "hiredis"]
                .iter()
                .any(|name| line.starts_with(name))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let file = root.join("requirements.txt");
    tokio::fs::write(&file, requirements)
        .await
        .map_err(failure)?;
    let log = tempfile::NamedTempFile::new().map_err(failure)?;
    let mut child = command(root)
        .args([
            "-I",
            "-m",
            "pip",
            "--isolated",
            "install",
            "--disable-pip-version-check",
            "--no-cache-dir",
            "--no-compile",
            "--index-url",
            "https://pypi.org/simple",
            "--only-binary",
            ":all:",
            "--target",
        ])
        .env(
            "PIP_CONFIG_FILE",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .arg(root.join("site"))
        .arg("-r")
        .arg(file)
        .stderr(Stdio::from(log.reopen().map_err(failure)?))
        .spawn()
        .map_err(failure)?;
    let mut tree = ProcessTree::attach(&child).map_err(failure)?;
    let status = timeout(Duration::from_secs(300), child.wait())
        .await
        .map_err(failure)?
        .map_err(failure)?;
    tree.completed();
    if status.success() {
        Ok(())
    } else {
        let log = tokio::fs::read_to_string(log.path())
            .await
            .unwrap_or_default();
        eprintln!(
            "Lyrica dependencies ({status}): {}",
            log.lines().rev().take(6).collect::<Vec<_>>().join("\n")
        );
        Err("Не удалось установить зависимости локальной Lyrica.".into())
    }
}

async fn start(root: &Path) -> Result<Running, String> {
    let ready = tempfile::NamedTempFile::new_in(root)
        .map_err(failure)?
        .into_temp_path();
    let key: String = rand::random::<[u8; 32]>()
        .iter()
        .map(|value| format!("{value:02x}"))
        .collect();
    let log = tempfile::NamedTempFile::new().map_err(failure)?;
    let mut child = command(root)
        .args(["-I", "-B"])
        .arg(root.join(LAUNCHER))
        .stdin(Stdio::piped())
        .stderr(Stdio::from(log.reopen().map_err(failure)?))
        .env("YAP_LYRICA_READY", &*ready)
        .env("YAP_LYRICA_KEY", &key)
        .env("LYRICA_CONFIG", root.join("lyrica.ini"))
        .env("CACHE_DIR", root.join("cache"))
        .env("LOG_LEVEL", "ERROR")
        .spawn()
        .map_err(failure)?;
    let client = Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(failure)?;
    timeout(Duration::from_secs(35), async {
        loop {
            if child.try_wait().map_err(failure)?.is_some() {
                let log = tokio::fs::read_to_string(log.path())
                    .await
                    .unwrap_or_default();
                eprintln!(
                    "Lyrica startup: {}",
                    log.lines().rev().take(6).collect::<Vec<_>>().join("\n")
                );
                return Err("Локальная Lyrica завершилась при запуске.".into());
            }
            if let Ok(port) = tokio::fs::read_to_string(&ready)
                .await
                .map_err(failure)?
                .parse::<u16>()
            {
                if port != 0 {
                    let endpoint = Endpoint {
                        port,
                        key: key.clone(),
                    };
                    if endpoint
                        .get(&client, "/health")
                        .send()
                        .await
                        .is_ok_and(|result| result.status().is_success())
                    {
                        return Ok(Running { child, endpoint });
                    }
                }
            }
            sleep(Duration::from_millis(120)).await;
        }
    })
    .await
    .map_err(|_| "Локальная Lyrica не запустилась вовремя.".to_string())?
}

async fn stop(running: &mut Option<Running>) -> Result<(), String> {
    if let Some(mut running) = running.take() {
        if running.child.try_wait().map_err(failure)?.is_none() {
            running.child.kill().await.map_err(failure)?;
        }
    }
    Ok(())
}

impl LocalLyrica {
    pub(crate) async fn stop_if_disabled(&self, app: &AppHandle) -> Result<(), String> {
        let mut running = self.0.lock().await;
        if !super::enabled::tool_enabled(app, super::install::Tool::Lyrica) {
            stop(&mut running).await?;
        }
        Ok(())
    }

    pub(crate) async fn endpoint(&self, app: &AppHandle) -> Result<Option<Endpoint>, String> {
        let mut running = self.0.lock().await;
        if !super::enabled::tool_enabled(app, super::install::Tool::Lyrica) {
            return Ok(None);
        }
        if let Some(server) = running.as_mut() {
            if server.child.try_wait().map_err(failure)?.is_none() {
                return Ok(Some(server.endpoint.clone()));
            }
            *running = None;
        }
        let Some(tools) = tools(app, false).await? else {
            return Ok(None);
        };
        let Some(root) = status(&tools).await?.path else {
            return Ok(None);
        };
        let server = start(&root).await?;
        let endpoint = server.endpoint.clone();
        *running = Some(server);
        Ok(Some(endpoint))
    }

    pub(crate) fn shutdown(&self) {
        if let Ok(mut running) = self.0.try_lock() {
            running.take();
        }
    }
}

pub(super) async fn remove(app: &AppHandle) -> Result<(), String> {
    let _install = INSTALLER.start("remove".into())?;
    let server = app.state::<LocalLyrica>();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "downloads the pinned Python and Lyrica distributions and installs Python packages"]
    async fn installs_and_serves_lyrics_on_an_authenticated_loopback_port() {
        let temp = tempfile::tempdir().unwrap();
        let guard = INSTALLER.start("test-install".into()).unwrap();
        let server = LocalLyrica::default();
        let status = install_into(temp.path(), &server, &guard, &Channel::new(|_| Ok(())))
            .await
            .unwrap();
        let running = start(&status.path.unwrap()).await.unwrap();
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(35))
            .build()
            .unwrap();
        let unauthorized = client
            .get(format!("http://127.0.0.1:{}/health", running.endpoint.port))
            .send()
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), reqwest::StatusCode::NOT_FOUND);
        let response = running
            .endpoint
            .get(&client, "/lyrics/")
            .query(&[
                ("artist", "Linkin Park"),
                ("song", "Numb"),
                ("sequence", "2"),
                ("pass", "true"),
                ("timestamps", "true"),
                ("metadata", "false"),
                ("mood", "false"),
            ])
            .send()
            .await
            .unwrap();
        assert!(
            response.status().is_success(),
            "status {}",
            response.status()
        );
        let data: serde_json::Value =
            serde_json::from_str(&response.text().await.unwrap()).unwrap();
        assert_eq!(
            data["data"]["source"], "lrclib",
            "status {}",
            data["status"]
        );
        assert!(data["data"]["timed_lyrics"]
            .as_array()
            .is_some_and(|lines| !lines.is_empty()));
        stop(&mut Some(running)).await.unwrap();
    }
}
