use std::{path::Path, process::Stdio, time::Duration};

use tauri::State;
use tempfile::TempPath;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    time::timeout,
};

use super::{
    downloads::ProcessTree, installed_yt_dlp, interrupted_launch, youtube_url, yt_dlp_command,
    YtDlpError,
};
use crate::audio::{Action, AudioEngine, PreviewStartError};

const AUDIO_LIMIT: u64 = 32 * 1024 * 1024;
const PREVIEW_TIMEOUT: Duration = Duration::from_secs(40);

#[tauri::command]
pub(crate) async fn preview_yt_dlp(
    app: tauri::AppHandle,
    audio: State<'_, AudioEngine>,
    id: u64,
    video_id: String,
) -> Result<(), YtDlpError> {
    let mut cancelled = audio.claim_preview(id).await.map_err(start_error)?;
    let prepared = tokio::select! {
        biased;
        _ = &mut cancelled => return Err(YtDlpError::PreviewCancelled),
        result = async {
            let url = youtube_url(&video_id)?;
            let path = installed_yt_dlp(&app).await?;
            download_preview(&path.path, &url).await
        } => result,
    };
    let result = match prepared {
        Ok(file) => audio.start_preview(id, file).await.map_err(start_error),
        Err(error) => Err(error),
    };
    if result.is_err() {
        let _ = audio
            .control(Action::ClosePreview {
                id,
                keep_paused: false,
            })
            .await;
    }
    result
}

async fn download_preview(executable: &Path, url: &str) -> Result<TempPath, YtDlpError> {
    let (file, path) = tempfile::Builder::new()
        .prefix("yap-preview-")
        .tempfile()
        .map_err(|_| YtDlpError::PreviewFailed)?
        .into_parts();
    let mut file = tokio::fs::File::from_std(file);
    let mut child = yt_dlp_command(executable)
        .args([
            "--ignore-config",
            "--no-plugin-dirs",
            "--no-cache-dir",
            "--no-playlist",
            "--quiet",
            "--no-warnings",
            "--no-progress",
            "--socket-timeout",
            "10",
            "--retries",
            "1",
            "--fragment-retries",
            "1",
            "--format",
            "bestaudio[ext=m4a]/bestaudio[ext=webm]",
            "--output",
            "-",
            "--",
            url,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| YtDlpError::LaunchFailed)?;
    let pid = child.id();
    let mut tree = ProcessTree::attach(&child).map_err(|_| YtDlpError::LaunchFailed)?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or(YtDlpError::LaunchFailed)?
        .take(AUDIO_LIMIT + 1);
    let (status, size) = timeout(PREVIEW_TIMEOUT, async {
        tokio::try_join!(
            async { child.wait().await.map_err(|_| YtDlpError::LaunchFailed) },
            async {
                let size = tokio::io::copy(&mut stdout, &mut file)
                    .await
                    .map_err(|_| YtDlpError::PreviewFailed)?;
                if size > AUDIO_LIMIT {
                    return Err(YtDlpError::PreviewTooLarge);
                }
                file.flush().await.map_err(|_| YtDlpError::PreviewFailed)?;
                Ok(size)
            },
        )
    })
    .await
    .map_err(|_| YtDlpError::TimedOut)??;
    tree.completed();
    if let Some(error) = interrupted_launch(&status, pid).await {
        return Err(error);
    }
    if !status.success() || size == 0 {
        return Err(YtDlpError::PreviewFailed);
    }
    Ok(path)
}

fn start_error(error: PreviewStartError) -> YtDlpError {
    match error {
        PreviewStartError::Cancelled => YtDlpError::PreviewCancelled,
        PreviewStartError::Failed => YtDlpError::PreviewFailed,
    }
}
