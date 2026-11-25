use reqwest::Client;
use sha2::{Digest, Sha256};
use std::{
    io,
    path::{Path, PathBuf},
};
use tauri::ipc::Channel;
use tokio::io::AsyncWriteExt;

use super::InstallProgress;

pub(super) async fn download_verified(
    client: &Client,
    url: &str,
    expected: &str,
    path: &Path,
    limit: u64,
    channel: &Channel<InstallProgress>,
) -> io::Result<()> {
    let mut response = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(io::Error::other)?;
    let mut progress = InstallProgress {
        status: "downloading",
        downloaded: 0,
        total: response.content_length(),
    };
    let too_large = || io::Error::other("Archive exceeds size limit");
    if progress.total.is_some_and(|size| size > limit) {
        return Err(too_large());
    }
    let _ = channel.send(progress);
    let mut hash = Sha256::new();
    let mut file = tokio::fs::File::create(path).await?;
    let mut reported = 0;
    while let Some(chunk) = response.chunk().await.map_err(io::Error::other)? {
        progress.downloaded += chunk.len() as u64;
        if progress.downloaded > limit {
            return Err(too_large());
        }
        hash.update(&chunk);
        file.write_all(&chunk).await?;
        if progress.downloaded - reported >= 256 * 1024 {
            let _ = channel.send(progress);
            reported = progress.downloaded;
        }
    }
    file.flush().await?;
    if format!("{:x}", hash.finalize()) != expected {
        return Err(io::Error::other("Archive checksum mismatch"));
    }
    let _ = channel.send(progress);
    Ok(())
}

pub(super) async fn directory_exists(path: &Path) -> io::Result<bool> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Ok(_) => Err(io::Error::other("ожидалась обычная папка инструментов")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub(super) async fn restore(current: &Path, previous: &Path) -> io::Result<()> {
    if !directory_exists(current).await? && directory_exists(previous).await? {
        tokio::fs::rename(previous, current).await?;
    }
    Ok(())
}

// Hold the corresponding tool's switch lock across this operation.
pub(super) async fn publish(staging: &Path, current: &Path, previous: &Path) -> io::Result<()> {
    restore(current, previous).await?;
    if directory_exists(previous).await? {
        tokio::fs::remove_dir_all(previous).await?;
    }
    if directory_exists(current).await? {
        tokio::fs::rename(current, previous).await?;
    }
    if let Err(error) = tokio::fs::rename(staging, current).await {
        restore(current, previous).await?;
        return Err(error);
    }
    Ok(())
}

pub(super) fn archive_checksum(checksums: &str, asset: &str) -> Option<String> {
    checksums.lines().find_map(|line| {
        let (hash, name) = line.split_once(char::is_whitespace)?;
        (name.trim().trim_start_matches('*') == asset
            && hash.len() == 64
            && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| hash.to_ascii_lowercase())
    })
}

pub(super) async fn tools_directory(data: &Path, create: bool) -> io::Result<Option<PathBuf>> {
    let tools = data.join("tools");
    if create {
        tokio::fs::create_dir_all(&tools).await?;
    }
    if !directory_exists(&tools).await? {
        return Ok(None);
    }
    let canonical = tokio::fs::canonicalize(&tools).await?;
    if canonical != tokio::fs::canonicalize(data).await?.join("tools") {
        return Err(io::Error::other(
            "папка инструментов находится вне каталога YAP",
        ));
    }
    Ok(Some(canonical))
}

pub(super) async fn remove_directory(path: &Path) -> io::Result<()> {
    if directory_exists(path).await? {
        tokio::fs::remove_dir_all(path).await?;
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn removal_refuses_to_follow_a_link_at_the_bundle_path() {
        let tools = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let bundle = tools.path().join("ffmpeg");
        std::os::unix::fs::symlink(target.path(), &bundle).unwrap();

        assert!(remove_directory(&bundle).await.is_err());
        assert!(target.path().is_dir());

        std::fs::remove_file(&bundle).unwrap();
        std::fs::create_dir(&bundle).unwrap();
        std::fs::write(bundle.join("ffmpeg"), b"fixture").unwrap();
        remove_directory(&bundle).await.unwrap();
        assert!(!bundle.exists());
    }
}
