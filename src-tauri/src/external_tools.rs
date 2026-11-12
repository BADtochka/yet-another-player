use std::{
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    time::Duration,
};

use serde::Serialize;
use tauri::Manager;
use tokio::{io::AsyncReadExt, process::Command, time::timeout};

mod bundle;
pub(crate) mod downloads;
pub(crate) mod ffmpeg;
pub(crate) mod install;
pub(crate) mod preview;
pub(crate) mod search;
pub(crate) mod yt_dlp_bundle;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstallProgress {
    status: &'static str,
    downloaded: u64,
    total: Option<u64>,
}

const VERSION_TIMEOUT: Duration = Duration::from_secs(45);
const YT_DLP_BINARY: &str = if cfg!(windows) {
    "yt-dlp.exe"
} else {
    "yt-dlp"
};
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum YtDlpError {
    Missing,
    TimedOut,
    #[cfg(target_os = "macos")]
    GatekeeperBlocked,
    LaunchFailed,
    RequestFailed,
    InvalidQuery,
    InvalidVideo,
    PreviewFailed,
    PreviewCancelled,
    PreviewTooLarge,
    DownloadBusy,
    InvalidDownload,
    DownloadCancelled,
    DownloadTimedOut,
    DownloadFailed,
    AgeRestricted,
    TargetUnavailable,
    ImportFailed,
    CleanupFailed,
}

#[tauri::command]
pub(crate) async fn check_yt_dlp(app: tauri::AppHandle) -> Result<String, YtDlpError> {
    let path = installed_yt_dlp(&app).await?;
    probe_version(&path.path, VERSION_TIMEOUT).await
}

#[tauri::command]
pub(crate) async fn get_yt_dlp_location(app: tauri::AppHandle) -> Result<PathBuf, YtDlpError> {
    find_yt_dlp(&app).await.map(|installed| installed.path)
}

fn youtube_url(video_id: &str) -> Result<String, YtDlpError> {
    if !valid_youtube_id(video_id) {
        return Err(YtDlpError::InvalidVideo);
    }
    Ok(format!("https://www.youtube.com/watch?v={video_id}"))
}

pub(crate) fn valid_youtube_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

struct InstalledYtDlp {
    path: PathBuf,
    _in_use: tokio::sync::RwLockReadGuard<'static, ()>,
}

async fn find_yt_dlp(app: &tauri::AppHandle) -> Result<InstalledYtDlp, YtDlpError> {
    let data = app
        .path()
        .app_data_dir()
        .map_err(|_| YtDlpError::LaunchFailed)?;
    let tools = bundle::tools_directory(&data, false)
        .await
        .map_err(|_| YtDlpError::LaunchFailed)?
        .ok_or(YtDlpError::Missing)?;
    let mut in_use = yt_dlp_bundle::BUNDLE_SWITCH.read().await;
    if !bundle::directory_exists(&tools.join(yt_dlp_bundle::BUNDLE_DIR))
        .await
        .map_err(|_| YtDlpError::LaunchFailed)?
        && bundle::directory_exists(&tools.join(yt_dlp_bundle::PREVIOUS_DIR))
            .await
            .map_err(|_| YtDlpError::LaunchFailed)?
    {
        drop(in_use);
        let switch = yt_dlp_bundle::BUNDLE_SWITCH.write().await;
        yt_dlp_bundle::restore_previous(&tools)
            .await
            .map_err(|_| YtDlpError::LaunchFailed)?;
        in_use = tokio::sync::RwLockWriteGuard::downgrade(switch);
    }
    let bundle_dir = tools.join(yt_dlp_bundle::BUNDLE_DIR);
    let directory = match tokio::fs::symlink_metadata(&bundle_dir).await {
        Ok(metadata) if metadata.is_dir() => bundle_dir,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => tools,
        _ => return Err(YtDlpError::LaunchFailed),
    };
    let path = directory.join(YT_DLP_BINARY);
    let metadata = match tokio::fs::symlink_metadata(&path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(YtDlpError::Missing)
        }
        Err(_) => return Err(YtDlpError::LaunchFailed),
    };
    if !metadata.is_file() {
        return Err(YtDlpError::LaunchFailed);
    }
    Ok(InstalledYtDlp {
        path,
        _in_use: in_use,
    })
}

async fn installed_yt_dlp(app: &tauri::AppHandle) -> Result<InstalledYtDlp, YtDlpError> {
    let installed = find_yt_dlp(app).await?;
    #[cfg(unix)]
    let path = &installed.path;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let metadata = tokio::fs::symlink_metadata(path)
            .await
            .map_err(|_| YtDlpError::LaunchFailed)?;
        if !metadata.is_file() {
            return Err(YtDlpError::LaunchFailed);
        }
        let mut permissions = metadata.permissions();
        if permissions.mode() & 0o100 == 0 {
            permissions.set_mode(permissions.mode() | 0o100);
            tokio::fs::set_permissions(path, permissions)
                .await
                .map_err(|_| YtDlpError::LaunchFailed)?;
        }
    }
    Ok(installed)
}

async fn probe_version(path: &Path, deadline: Duration) -> Result<String, YtDlpError> {
    let (status, stdout) = run_yt_dlp(path, &["--version"], deadline, 1024).await?;
    if !status.success() {
        return Err(YtDlpError::LaunchFailed);
    }
    stdout
        .lines()
        .next()
        .map(str::trim)
        .filter(|version| !version.is_empty())
        .map(str::to_owned)
        .ok_or(YtDlpError::LaunchFailed)
}

async fn run_yt_dlp(
    path: &Path,
    args: &[&str],
    deadline: Duration,
    output_limit: u64,
) -> Result<(ExitStatus, String), YtDlpError> {
    let mut command = yt_dlp_command(path);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let mut child = command.spawn().map_err(|_| YtDlpError::LaunchFailed)?;
    let pid = child.id();
    let stdout = child.stdout.take().ok_or(YtDlpError::LaunchFailed)?;
    let mut bytes = Vec::new();
    let exit = timeout(deadline, async {
        let (status, _) = tokio::try_join!(child.wait(), async {
            stdout.take(output_limit + 1).read_to_end(&mut bytes).await
        })?;
        Ok::<_, std::io::Error>(status)
    })
    .await;
    match exit {
        Ok(Ok(status)) => {
            if let Some(error) = interrupted_launch(&status, pid).await {
                return Err(error);
            }
            if bytes.len() as u64 > output_limit {
                return Err(YtDlpError::RequestFailed);
            }
            Ok((status, String::from_utf8_lossy(&bytes).into_owned()))
        }
        Ok(Err(_)) => Err(YtDlpError::LaunchFailed),
        Err(_) => {
            #[cfg(unix)]
            if let Some(group) = pid.and_then(|pid| i32::try_from(pid).ok()) {
                // PyInstaller can spawn a child. The probe has its own process group.
                // SAFETY: A negative PID targets only the process group created above.
                unsafe { libc::kill(-group, libc::SIGKILL) };
            }
            let _ = child.start_kill();
            Err(YtDlpError::TimedOut)
        }
    }
}

fn yt_dlp_command(path: &Path) -> Command {
    let mut command = Command::new(path);
    command
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    command
}

async fn interrupted_launch(_status: &ExitStatus, _pid: Option<u32>) -> Option<YtDlpError> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::process::ExitStatusExt;

        if _status.signal() == Some(libc::SIGKILL) {
            if let Some(pid) = _pid {
                if gatekeeper_rejected(pid).await {
                    return Some(YtDlpError::GatekeeperBlocked);
                }
            }
            return Some(YtDlpError::LaunchFailed);
        }
    }
    None
}

#[cfg(target_os = "macos")]
async fn gatekeeper_rejected(pid: u32) -> bool {
    // syspolicyd can write the rejection shortly after the child has exited.
    tokio::time::sleep(Duration::from_millis(400)).await;
    let mut log = Command::new("/usr/bin/log");
    let prefix = format!("Terminating process due to Gatekeeper rejection: {pid},");
    let predicate = format!("process == \"syspolicyd\" AND eventMessage BEGINSWITH \"{prefix}\"");
    log.args([
        "show",
        "--last",
        "8s",
        "--style",
        "json",
        "--predicate",
        &predicate,
    ])
    .kill_on_drop(true);
    let Ok(Ok(output)) = timeout(Duration::from_secs(3), log.output()).await else {
        return false;
    };
    output.status.success() && log_has_rejection(&output.stdout, &prefix)
}

#[cfg(target_os = "macos")]
fn log_has_rejection(log: &[u8], prefix: &str) -> bool {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Entry {
        event_message: Option<String>,
    }

    serde_json::from_slice::<Vec<Entry>>(log).is_ok_and(|entries| {
        entries.iter().any(|entry| {
            entry
                .event_message
                .as_deref()
                .is_some_and(|message| message.starts_with(prefix))
        })
    })
}

#[tauri::command]
pub(crate) async fn open_macos_privacy_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        for pane in [
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension",
            "x-apple.systempreferences:com.apple.preference.security",
        ] {
            let status = Command::new("/usr/bin/open")
                .arg(pane)
                .status()
                .await
                .map_err(|err| err.to_string())?;
            if status.success() {
                return Ok(());
            }
        }
        Err("Не удалось открыть «Конфиденциальность и безопасность»".into())
    }
    #[cfg(not(target_os = "macos"))]
    Err("Системные настройки macOS здесь недоступны".into())
}

#[cfg(all(test, unix))]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt};

    use super::*;

    fn fixture(script: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(YT_DLP_BINARY);
        fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        (dir, path)
    }

    #[tokio::test]
    async fn probe_reads_version_from_executable() {
        let (_dir, path) = fixture("test \"$1\" = --version && printf '2026.10.26\\n'");
        let result = probe_version(&path, Duration::from_secs(5)).await;
        assert_eq!(result.as_deref(), Ok("2026.10.26"));
    }

    #[tokio::test]
    async fn probe_distinguishes_timeout_from_launch_failure() {
        let (_slow, slow) = fixture("/bin/sleep 10");
        let (_failed, failed) = fixture("exit 1");
        let (timed_out, failed) = tokio::join!(
            probe_version(&slow, Duration::from_millis(150)),
            probe_version(&failed, Duration::from_secs(5))
        );
        assert_eq!(timed_out, Err(YtDlpError::TimedOut));
        assert_eq!(failed, Err(YtDlpError::LaunchFailed));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn gatekeeper_log_requires_the_exact_probe_pid() {
        let entries = br#"[
            {"eventMessage":"Terminating process due to Gatekeeper rejection: 24444, <private>"},
            {"eventMessage":"Tracking process with attributes: 244, TA"}
        ]"#;
        assert!(log_has_rejection(
            entries,
            "Terminating process due to Gatekeeper rejection: 24444,"
        ));
        assert!(!log_has_rejection(
            entries,
            "Terminating process due to Gatekeeper rejection: 244,"
        ));
    }
}
