use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read},
    path::{Component, Path},
    time::Duration,
};

use reqwest::{self, Client, Response};
use sha2::{Digest, Sha256};
use tauri::{ipc::Channel, Manager};
use tokio::{io::AsyncWriteExt, sync::RwLock};

use super::{
    bundle,
    install::{Installation, Installer},
    InstallProgress, YT_DLP_BINARY,
};

pub(super) const BUNDLE_DIR: &str = "yt-dlp-bundle";
pub(super) const PREVIOUS_DIR: &str = "yt-dlp-previous";
const RELEASE_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest";
const ARCHIVE_LIMIT: u64 = 128 * 1024 * 1024;
const EXTRACTED_LIMIT: u64 = 512 * 1024 * 1024;
const CHECKSUM_LIMIT: usize = 64 * 1024;
const MAX_ENTRIES: usize = 4096;

pub(super) static INSTALLER: Installer = Installer::new();
pub(super) static BUNDLE_SWITCH: RwLock<()> = RwLock::const_new(());

#[tauri::command]
pub(crate) async fn latest_yt_dlp_version() -> Result<String, String> {
    let response = release_client()?
        .head(RELEASE_URL)
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .and_then(Response::error_for_status)
        .map_err(download_error)?;
    let tag = response
        .url()
        .path()
        .strip_prefix("/yt-dlp/yt-dlp/releases/tag/")
        .filter(|tag| {
            !tag.is_empty()
                && tag
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'.')
        });
    if response.url().host_str() != Some("github.com") {
        return Err("Не удалось определить версию yt-dlp.".into());
    }
    tag.map(str::to_owned)
        .ok_or_else(|| "Не удалось определить версию yt-dlp.".into())
}

#[tauri::command]
pub(crate) async fn install_yt_dlp_bundle(
    app: tauri::AppHandle,
    id: String,
    on_progress: Channel<InstallProgress>,
) -> Result<InstallProgress, String> {
    let install = INSTALLER.start(id)?;
    let (asset, executable) = release_asset(
        std::env::consts::OS,
        std::env::consts::ARCH,
        cfg!(target_env = "musl"),
    )
    .ok_or("Для этой платформы нет подходящего архива yt-dlp.")?;
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let tools = bundle::tools_directory(&app_data, true)
        .await
        .map_err(install_io_error)?
        .ok_or("Папка инструментов YAP недоступна.")?;

    let staging = tempfile::tempdir_in(&tools).map_err(install_io_error)?;
    let archive = staging.path().join("release.zip");
    let bundle = staging.path().join("bundle");
    let client = release_client()?;
    let expected = install.wait(release_checksum(&client, asset)).await?;
    let progress =
        download_archive(&client, asset, &expected, &archive, &on_progress, &install).await?;
    let _ = on_progress.send(progress);

    let extracted = bundle.clone();
    tauri::async_runtime::spawn_blocking(move || unpack_archive(&archive, &extracted, executable))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("Не удалось распаковать yt-dlp: {error}"))?;
    let previous = tools.join(PREVIOUS_DIR);
    let _switch = match BUNDLE_SWITCH.try_write() {
        Ok(guard) => guard,
        Err(_) => {
            let _ = on_progress.send(InstallProgress {
                status: "waitingForDownload",
                ..progress
            });
            install
                .wait(async { Ok(BUNDLE_SWITCH.write().await) })
                .await?
        }
    };
    install.commit()?;
    let _ = on_progress.send(InstallProgress {
        status: "publishing",
        ..progress
    });
    bundle::publish(&bundle, &tools.join(BUNDLE_DIR), &previous)
        .await
        .map_err(install_io_error)?;
    let _ = tokio::fs::remove_dir_all(previous).await;
    Ok(progress)
}

fn release_asset(os: &str, arch: &str, musl: bool) -> Option<(&'static str, &'static str)> {
    match (os, arch, musl) {
        ("macos", "x86_64" | "aarch64", _) => Some(("yt-dlp_macos.zip", "yt-dlp_macos")),
        ("windows", "x86_64", _) => Some(("yt-dlp_win.zip", "yt-dlp.exe")),
        ("windows", "x86", _) => Some(("yt-dlp_win_x86.zip", "yt-dlp_x86.exe")),
        ("windows", "aarch64", _) => Some(("yt-dlp_win_arm64.zip", "yt-dlp_arm64.exe")),
        ("linux", "x86_64", false) => Some(("yt-dlp_linux.zip", "yt-dlp_linux")),
        ("linux", "aarch64", false) => Some(("yt-dlp_linux_aarch64.zip", "yt-dlp_linux_aarch64")),
        ("linux", "arm", false) => Some(("yt-dlp_linux_armv7l.zip", "yt-dlp_linux_armv7l")),
        ("linux", "x86_64", true) => Some(("yt-dlp_musllinux.zip", "yt-dlp_musllinux")),
        ("linux", "aarch64", true) => {
            Some(("yt-dlp_musllinux_aarch64.zip", "yt-dlp_musllinux_aarch64"))
        }
        _ => None,
    }
}

fn release_client() -> Result<Client, String> {
    Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(240))
        .redirect(reqwest::redirect::Policy::custom(|redirect| {
            let url = redirect.url();
            if redirect.previous().len() < 8
                && url.scheme() == "https"
                && matches!(
                    url.host_str(),
                    Some(
                        "github.com"
                            | "release-assets.githubusercontent.com"
                            | "objects.githubusercontent.com"
                    )
                )
            {
                redirect.follow()
            } else {
                redirect.error("unexpected yt-dlp release redirect")
            }
        }))
        .build()
        .map_err(download_error)
}

async fn release_response(client: &Client, asset: &str) -> Result<Response, String> {
    client
        .get(format!("{RELEASE_URL}/download/{asset}"))
        .send()
        .await
        .and_then(Response::error_for_status)
        .map_err(download_error)
}

async fn release_checksum(client: &Client, asset: &str) -> Result<String, String> {
    let mut response = release_response(client, "SHA2-256SUMS").await?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(download_error)? {
        if bytes.len() + chunk.len() > CHECKSUM_LIMIT {
            return Err("Список контрольных сумм yt-dlp слишком большой.".into());
        }
        bytes.extend_from_slice(&chunk);
    }

    bundle::archive_checksum(&String::from_utf8_lossy(&bytes), asset)
        .ok_or_else(|| "Для архива yt-dlp не найдена контрольная сумма SHA-256.".into())
}

async fn download_archive(
    client: &Client,
    asset: &str,
    expected: &str,
    archive: &Path,
    on_progress: &Channel<InstallProgress>,
    install: &Installation<'_>,
) -> Result<InstallProgress, String> {
    let mut response = install.wait(release_response(client, asset)).await?;
    let mut progress = InstallProgress {
        status: "downloading",
        downloaded: 0,
        total: response.content_length(),
    };
    let archive_too_big = "Архив yt-dlp превышает допустимый размер.";
    if progress.total.is_some_and(|total| total > ARCHIVE_LIMIT) {
        return Err(archive_too_big.into());
    }
    let _ = on_progress.send(progress);

    let mut file = tokio::fs::File::create(archive)
        .await
        .map_err(install_io_error)?;
    let mut hash = Sha256::new();
    let mut reported = 0;
    while let Some(chunk) = install
        .wait(async { response.chunk().await.map_err(download_error) })
        .await?
    {
        progress.downloaded += chunk.len() as u64;
        if progress.downloaded > ARCHIVE_LIMIT {
            return Err(archive_too_big.into());
        }
        hash.update(&chunk);
        file.write_all(&chunk).await.map_err(install_io_error)?;
        if progress.downloaded - reported >= 256 * 1024 {
            let _ = on_progress.send(progress);
            reported = progress.downloaded;
        }
    }
    file.flush().await.map_err(install_io_error)?;
    if format!("{:x}", hash.finalize()) != expected {
        return Err("Контрольная сумма архива yt-dlp не совпала. Повторите установку.".into());
    }

    Ok(InstallProgress {
        status: "extracting",
        ..progress
    })
}

fn unpack_archive(archive: &Path, bundle: &Path, executable: &str) -> io::Result<()> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    if zip.len() > MAX_ENTRIES {
        return Err(invalid_archive());
    }
    fs::create_dir(bundle)?;
    let mut remaining = EXTRACTED_LIMIT;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let relative = entry.enclosed_name().ok_or_else(invalid_archive)?;
        if entry.is_symlink()
            || !relative
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
            || entry.name().contains(['\\', ':'])
        {
            return Err(invalid_archive());
        }
        let is_executable = relative == Path::new(executable) && !entry.is_dir();
        let path = if is_executable {
            bundle.join(YT_DLP_BINARY)
        } else if relative.starts_with("_internal") {
            bundle.join(relative)
        } else {
            return Err(invalid_archive());
        };

        if entry.is_dir() {
            fs::create_dir_all(path)?;
            continue;
        }
        if entry.size() > remaining {
            return Err(invalid_archive());
        }
        let parent = path.parent().ok_or_else(invalid_archive)?;
        fs::create_dir_all(parent)?;
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        let copied = io::copy(&mut entry.by_ref().take(remaining + 1), &mut file)?;
        if copied != entry.size() || copied > remaining {
            return Err(invalid_archive());
        }
        remaining -= copied;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mode = if is_executable {
                0o755
            } else {
                entry.unix_mode().unwrap_or(0o644) & 0o755
            };
            file.set_permissions(fs::Permissions::from_mode(mode))?;
        }
    }
    if !bundle.join(YT_DLP_BINARY).is_file()
        || fs::read_dir(bundle.join("_internal"))?.next().is_none()
    {
        return Err(invalid_archive());
    }
    Ok(())
}

pub(super) async fn restore_previous(tools: &Path) -> io::Result<()> {
    bundle::restore(&tools.join(BUNDLE_DIR), &tools.join(PREVIOUS_DIR)).await
}

pub(super) async fn remove(app: &tauri::AppHandle) -> Result<(), String> {
    let install = INSTALLER.start("remove".into())?;
    install.commit()?;
    let data = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let Some(tools) = bundle::tools_directory(&data, false)
        .await
        .map_err(install_io_error)?
    else {
        return Ok(());
    };
    let _switch = BUNDLE_SWITCH
        .try_write()
        .map_err(|_| "yt-dlp используется. Дождитесь окончания операции.")?;
    bundle::remove_directory(&tools.join(PREVIOUS_DIR))
        .await
        .map_err(install_io_error)?;
    let legacy = tools.join(YT_DLP_BINARY);
    match tokio::fs::symlink_metadata(&legacy).await {
        Ok(metadata) if metadata.is_file() => tokio::fs::remove_file(legacy)
            .await
            .map_err(install_io_error)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(install_io_error(error)),
        Ok(_) => return Err("Ожидался обычный файл yt-dlp.".into()),
    }
    bundle::remove_directory(&tools.join(BUNDLE_DIR))
        .await
        .map_err(install_io_error)
}

fn invalid_archive() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "неверная структура архива yt-dlp",
    )
}

fn download_error(error: reqwest::Error) -> String {
    format!("Не удалось скачать yt-dlp: {}", error.without_url())
}

fn install_io_error(error: io::Error) -> String {
    format!("Не удалось установить yt-dlp: {error}")
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use zip::write::SimpleFileOptions;

    fn create_archive(path: &Path, files: &[(&str, &[u8])]) {
        let mut archive = zip::ZipWriter::new(File::create(path).unwrap());
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, content) in files {
            archive.start_file(*name, options).unwrap();
            archive.write_all(content).unwrap();
        }
        archive.finish().unwrap();
    }

    #[test]
    fn unpack_preserves_the_runtime_next_to_the_normalized_executable() {
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("release.zip");
        let bundle = root.path().join("bundle");
        create_archive(
            &archive,
            &[
                ("release-exe", b"executable"),
                ("_internal/runtime/library", b"dependency"),
            ],
        );

        unpack_archive(&archive, &bundle, "release-exe").unwrap();

        assert_eq!(fs::read(bundle.join(YT_DLP_BINARY)).unwrap(), b"executable");
        assert_eq!(
            fs::read(bundle.join("_internal/runtime/library")).unwrap(),
            b"dependency"
        );
    }

    #[test]
    fn unpack_rejects_paths_outside_the_bundle() {
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("release.zip");
        create_archive(&archive, &[("_internal/../../escaped", b"unexpected")]);

        let result = unpack_archive(&archive, &root.path().join("bundle"), "release-exe");

        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert!(!root.path().join("escaped").exists());
    }

    #[tokio::test]
    async fn publish_restores_the_previous_bundle_when_the_new_one_cannot_move() {
        let tools = tempfile::tempdir().unwrap();
        let current = tools.path().join(BUNDLE_DIR);
        fs::create_dir(&current).unwrap();
        fs::write(current.join(YT_DLP_BINARY), "old executable").unwrap();

        assert!(bundle::publish(
            &tools.path().join("missing"),
            &current,
            &tools.path().join(PREVIOUS_DIR),
        )
        .await
        .is_err());
        assert_eq!(
            fs::read_to_string(current.join(YT_DLP_BINARY)).unwrap(),
            "old executable"
        );
    }
}
