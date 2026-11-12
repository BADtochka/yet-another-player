use std::{
    fs::{File, OpenOptions},
    io::{self, Read},
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use reqwest::{redirect, Client, Response};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{ipc::Channel, Manager};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::{RwLock, RwLockReadGuard, RwLockWriteGuard},
    time::timeout,
};

use super::{
    bundle,
    install::{Installation, Installer},
    InstallProgress,
};

const BTBN: &str = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest";
const MAC_BUILDS: &str = "https://ffmpeg.martin-riedl.de/download/macos";
const BINARIES: [&str; 2] = if cfg!(windows) {
    ["ffmpeg.exe", "ffprobe.exe"]
} else {
    ["ffmpeg", "ffprobe"]
};
const ARCHIVE_LIMIT: u64 = 384 * 1024 * 1024;
const BINARY_LIMIT: u64 = 256 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;
const CURRENT: &str = "ffmpeg";
const PREVIOUS: &str = "ffmpeg-previous";

pub(super) static INSTALLER: Installer = Installer::new();
static SWITCH: RwLock<()> = RwLock::const_new(());

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Status {
    installed: bool,
    version: Option<String>,
    path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    launch_error: Option<String>,
}

pub(super) struct Installed {
    pub(super) path: PathBuf,
    _in_use: RwLockReadGuard<'static, ()>,
}

pub(super) async fn installed(app: &tauri::AppHandle) -> io::Result<Option<Installed>> {
    let data = app.path().app_data_dir().map_err(io::Error::other)?;
    let Some(tools) = bundle::tools_directory(&data, false).await? else {
        return Ok(None);
    };
    let path = tools.join(CURRENT);
    let previous = tools.join(PREVIOUS);
    let mut in_use = SWITCH.read().await;
    if !bundle::directory_exists(&path).await? && bundle::directory_exists(&previous).await? {
        drop(in_use);
        let switch = SWITCH.write().await;
        bundle::restore(&path, &previous).await?;
        in_use = RwLockWriteGuard::downgrade(switch);
    }
    for name in BINARIES {
        match tokio::fs::symlink_metadata(path.join(name)).await {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => return Err(io::Error::other("неверный файл FFmpeg")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        }
    }
    Ok(Some(Installed {
        path,
        _in_use: in_use,
    }))
}

#[tauri::command]
pub(crate) async fn check_ffmpeg(app: tauri::AppHandle) -> Result<Status, String> {
    let Some(installed) = installed(&app).await.map_err(install_error)? else {
        return Ok(Status {
            installed: false,
            version: None,
            path: None,
            launch_error: None,
        });
    };
    let path = installed.path.join(BINARIES[0]);
    let (version, launch_error) = match probe_bundle(&installed.path).await {
        Ok(version) => (Some(version), None),
        Err(error) => (None, Some(error)),
    };
    Ok(Status {
        installed: true,
        version,
        path: Some(path),
        launch_error,
    })
}

#[tauri::command]
pub(crate) async fn install_ffmpeg(
    app: tauri::AppHandle,
    id: String,
    on_progress: Channel<InstallProgress>,
) -> Result<Status, String> {
    let install = INSTALLER.start(id)?;
    let data = app.path().app_data_dir().map_err(install_error)?;
    let tools = bundle::tools_directory(&data, true)
        .await
        .map_err(install_error)?
        .ok_or("Папка инструментов YAP недоступна.")?;
    let staging = tempfile::tempdir_in(&tools).map_err(install_error)?;
    let files = staging.path().join("bundle");
    tokio::fs::create_dir(&files).await.map_err(install_error)?;
    let client = release_client()?;
    let archives = install
        .wait(archives(
            &client,
            std::env::consts::OS,
            std::env::consts::ARCH,
        ))
        .await?;
    let archive_path = staging.path().join("release");
    let mut progress = InstallProgress {
        status: "downloading",
        downloaded: 0,
        total: None,
    };
    for (index, archive) in archives.iter().enumerate() {
        let mut response = install.wait(response(&client, &archive.url)).await?;
        let length = response.content_length();
        if length.is_some_and(|length| length > ARCHIVE_LIMIT) {
            return Err("Архив FFmpeg слишком большой.".into());
        }
        progress.total = if index + 1 == archives.len() {
            length.map(|length| length + progress.downloaded)
        } else {
            None
        };
        let _ = on_progress.send(progress);
        progress.downloaded += download_archive(
            &mut response,
            &archive_path,
            &archive.sha256,
            progress,
            &on_progress,
            &install,
        )
        .await?;
        // Keep the byte count cumulative when a platform has two archives.
        let _ = on_progress.send(InstallProgress {
            status: "extracting",
            ..progress
        });
        let archive_path = archive_path.clone();
        let destination = files.clone();
        let names = archive.names;
        let tar_xz = archive.url.ends_with(".tar.xz");
        tauri::async_runtime::spawn_blocking(move || {
            unpack(&archive_path, &destination, names, tar_xz)
        })
        .await
        .map_err(install_error)?
        .map_err(install_error)?;
    }
    let _ = on_progress.send(InstallProgress {
        status: "checking",
        ..progress
    });
    let version = install.wait(probe_bundle(&files)).await?;

    let current = tools.join(CURRENT);
    let previous = tools.join(PREVIOUS);
    let _switch = match SWITCH.try_write() {
        Ok(guard) => guard,
        Err(_) => {
            let _ = on_progress.send(InstallProgress {
                status: "waitingForDownload",
                ..progress
            });
            install.wait(async { Ok(SWITCH.write().await) }).await?
        }
    };
    install.commit()?;
    let _ = on_progress.send(InstallProgress {
        status: "publishing",
        ..progress
    });
    bundle::publish(&files, &current, &previous)
        .await
        .map_err(install_error)?;
    let _ = tokio::fs::remove_dir_all(previous).await;
    let _ = on_progress.send(InstallProgress {
        status: "installed",
        ..progress
    });
    Ok(Status {
        installed: true,
        version: Some(version),
        path: Some(current.join(BINARIES[0])),
        launch_error: None,
    })
}

pub(super) async fn remove(app: &tauri::AppHandle) -> Result<(), String> {
    let install = INSTALLER.start("remove".into())?;
    install.commit()?;
    let data = app.path().app_data_dir().map_err(install_error)?;
    let Some(tools) = bundle::tools_directory(&data, false)
        .await
        .map_err(install_error)?
    else {
        return Ok(());
    };
    let _switch = SWITCH
        .try_write()
        .map_err(|_| "FFmpeg используется. Дождитесь окончания операции.")?;
    bundle::remove_directory(&tools.join(PREVIOUS))
        .await
        .map_err(install_error)?;
    bundle::remove_directory(&tools.join(CURRENT))
        .await
        .map_err(install_error)
}

struct Archive {
    url: String,
    sha256: String,
    names: &'static [&'static str],
}

async fn archives(client: &Client, os: &str, arch: &str) -> Result<Vec<Archive>, String> {
    // The publisher provides separate, versioned ZIPs and SHA-256 files for macOS.
    let mac = match (os, arch) {
        ("macos", "aarch64") => Some((
            "arm64/1789931890_9.0.2",
            [
                "c8ed4c4e6978a03c485edbfe4e0a5dc2380f8a30bba5150531b31b094492d924",
                "fcbe839537485eaee7a7a8bc5cbc0f90d53617e80943e8a5b2e31cb851197ea6",
            ],
        )),
        ("macos", "x86_64") => Some((
            "amd64/1789931006_9.0.2",
            [
                "7c6b4125b191cbf773832dc51f424cf2b6bb7da43007d1e066f95909e47cacd4",
                "2322438ed2f6319a691291b247d09c69dcaa3a982460d1f269a7e1af335cfdfd",
            ],
        )),
        _ => None,
    };
    if let Some((release, checksums)) = mac {
        return Ok([&["ffmpeg"][..], &["ffprobe"][..]]
            .into_iter()
            .zip(checksums)
            .map(|(names, sha256)| Archive {
                url: format!("{MAC_BUILDS}/{release}/{}.zip", names[0]),
                sha256: sha256.into(),
                names,
            })
            .collect());
    }
    let asset = match (os, arch) {
        ("windows", "x86_64") => "ffmpeg-master-latest-win64-gpl.zip",
        ("windows", "aarch64") => "ffmpeg-master-latest-winarm64-gpl.zip",
        ("linux", "x86_64") => "ffmpeg-master-latest-linux64-gpl.tar.xz",
        ("linux", "aarch64") => "ffmpeg-master-latest-linuxarm64-gpl.tar.xz",
        _ => return Err("Для этой платформы нет архива FFmpeg.".into()),
    };
    let mut response = response(client, &format!("{BTBN}/checksums.sha256")).await?;
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        if data.len() + chunk.len() > 64 * 1024 {
            return Err("Список контрольных сумм FFmpeg слишком большой.".into());
        }
        data.extend_from_slice(&chunk);
    }
    let sha256 = bundle::archive_checksum(&String::from_utf8_lossy(&data), asset)
        .ok_or("Для архива FFmpeg не найдена контрольная сумма SHA-256.")?;
    Ok(vec![Archive {
        url: format!("{BTBN}/{asset}"),
        sha256,
        names: &BINARIES,
    }])
}

fn release_client() -> Result<Client, String> {
    Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(10 * 60))
        .redirect(redirect::Policy::custom(|redirect| {
            let url = redirect.url();
            if redirect.previous().len() < 6
                && url.scheme() == "https"
                && matches!(
                    url.host_str(),
                    Some(
                        "ffmpeg.martin-riedl.de"
                            | "github.com"
                            | "objects.githubusercontent.com"
                            | "release-assets.githubusercontent.com"
                    )
                )
            {
                redirect.follow()
            } else {
                redirect.error("unexpected FFmpeg release redirect")
            }
        }))
        .build()
        .map_err(network_error)
}

async fn response(client: &Client, url: &str) -> Result<Response, String> {
    client
        .get(url)
        .send()
        .await
        .and_then(Response::error_for_status)
        .map_err(network_error)
}

async fn download_archive(
    response: &mut Response,
    path: &Path,
    sha256: &str,
    mut progress: InstallProgress,
    on_progress: &Channel<InstallProgress>,
    install: &Installation<'_>,
) -> Result<u64, String> {
    let mut file = tokio::fs::File::create(path).await.map_err(install_error)?;
    let mut hash = Sha256::new();
    let mut bytes = 0;
    let mut reported = 0;
    let previous = progress.downloaded;
    while let Some(chunk) = install
        .wait(async { response.chunk().await.map_err(network_error) })
        .await?
    {
        bytes += chunk.len() as u64;
        if bytes > ARCHIVE_LIMIT {
            return Err("Архив FFmpeg слишком большой.".into());
        }
        hash.update(&chunk);
        file.write_all(&chunk).await.map_err(install_error)?;
        if bytes - reported >= 256 * 1024 {
            progress.downloaded = previous + bytes;
            let _ = on_progress.send(progress);
            reported = bytes;
        }
    }
    file.flush().await.map_err(install_error)?;
    if format!("{:x}", hash.finalize()) != sha256 {
        return Err("Контрольная сумма архива FFmpeg не совпала. Повторите установку.".into());
    }
    Ok(bytes)
}

fn unpack(archive: &Path, destination: &Path, names: &[&str], tar_xz: bool) -> io::Result<()> {
    let input = File::open(archive)?;
    if tar_xz {
        let input =
            lzma_rust2::XzReader::new_mem_limit(io::BufReader::new(input), false, 128 * 1024);
        let mut tar = tar::Archive::new(input.take(768 * 1024 * 1024));
        for (index, entry) in tar.entries()?.enumerate() {
            if index >= MAX_ENTRIES {
                return Err(invalid_archive());
            }
            let mut entry = entry?;
            let relative = entry.path()?;
            if let Some(name) = binary_name(&relative, names) {
                if !entry.header().entry_type().is_file() {
                    return Err(invalid_archive());
                }
                let size = entry.size();
                copy_binary(&mut entry, size, &destination.join(name))?;
            }
        }
    } else {
        let mut zip = zip::ZipArchive::new(input)?;
        if zip.len() > MAX_ENTRIES {
            return Err(invalid_archive());
        }
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index)?;
            let relative = entry.enclosed_name().ok_or_else(invalid_archive)?;
            if let Some(name) = binary_name(&relative, names) {
                if !entry.is_file() || entry.is_symlink() {
                    return Err(invalid_archive());
                }
                let size = entry.size();
                copy_binary(&mut entry, size, &destination.join(name))?;
            }
        }
    }
    if names.iter().all(|name| destination.join(name).is_file()) {
        Ok(())
    } else {
        Err(invalid_archive())
    }
}

fn binary_name<'a>(path: &Path, names: &'a [&'a str]) -> Option<&'a str> {
    let mut parts = path.components();
    let first = parts.next()?;
    let candidate = match (first, parts.next(), parts.next(), parts.next()) {
        (Component::Normal(name), None, None, None) => name,
        (
            Component::Normal(_),
            Some(Component::Normal(bin)),
            Some(Component::Normal(name)),
            None,
        ) if bin == "bin" => name,
        _ => return None,
    };
    names.iter().copied().find(|name| candidate == *name)
}

fn copy_binary(input: &mut impl Read, size: u64, path: &Path) -> io::Result<()> {
    if size == 0 || size > BINARY_LIMIT {
        return Err(invalid_archive());
    }
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    if io::copy(&mut input.take(size + 1), &mut file)? != size {
        return Err(invalid_archive());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

async fn probe_bundle(directory: &Path) -> Result<String, String> {
    let ffmpeg_path = directory.join(BINARIES[0]);
    let ffprobe_path = directory.join(BINARIES[1]);
    let (ffmpeg, ffprobe) = tokio::join!(
        probe(&ffmpeg_path, "ffmpeg"),
        probe(&ffprobe_path, "ffprobe"),
    );
    let ffmpeg = ffmpeg?;
    let ffprobe = ffprobe?;
    if ffmpeg != ffprobe {
        return Err("Версии FFmpeg и FFprobe не совпадают. Переустановите инструменты.".into());
    }
    Ok(ffmpeg)
}

async fn probe(path: &Path, name: &str) -> Result<String, String> {
    let failure = || format!("Не удалось запустить {name}. Переустановите FFmpeg.");
    let mut command = Command::new(path);
    command
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let mut child = command.spawn().map_err(|_| failure())?;
    let mut stdout = child.stdout.take().ok_or_else(failure)?.take(16 * 1024);
    let mut bytes = Vec::new();
    let (status, _) = timeout(Duration::from_secs(12), async {
        tokio::try_join!(child.wait(), stdout.read_to_end(&mut bytes))
    })
    .await
    .map_err(|_| format!("{name} не ответил за 12 секунд."))?
    .map_err(|_| failure())?;
    if !status.success() {
        return Err(failure());
    }
    let output = String::from_utf8_lossy(&bytes);
    output
        .lines()
        .next()
        .and_then(|line| line.strip_prefix(name))
        .and_then(|line| line.strip_prefix(" version "))
        .and_then(|line| line.split_whitespace().next())
        .map(str::to_owned)
        .ok_or_else(failure)
}

fn invalid_archive() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "неверная структура архива FFmpeg",
    )
}

fn install_error(error: impl std::fmt::Display) -> String {
    format!("Не удалось подготовить FFmpeg: {error}")
}

fn network_error(error: reqwest::Error) -> String {
    format!("Не удалось скачать FFmpeg: {}", error.without_url())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn archive_extracts_only_the_two_expected_binaries() {
        let staging = tempfile::tempdir().unwrap();
        let archive = staging.path().join("ffmpeg.zip");
        let destination = staging.path().join("bundle");
        std::fs::create_dir(&destination).unwrap();
        let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
        for (path, bytes) in [
            ("release/bin/ffmpeg", b"ffmpeg".as_slice()),
            ("release/bin/ffprobe", b"ffprobe"),
            ("release/bin/ffplay", b"ffplay"),
        ] {
            zip.start_file(path, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();

        unpack(&archive, &destination, &["ffmpeg", "ffprobe"], false).unwrap();

        assert_eq!(
            std::fs::read(destination.join("ffmpeg")).unwrap(),
            b"ffmpeg"
        );
        assert_eq!(
            std::fs::read(destination.join("ffprobe")).unwrap(),
            b"ffprobe"
        );
        assert!(!destination.join("ffplay").exists());
    }

    #[test]
    fn archive_rejects_a_link_in_place_of_a_binary() {
        let staging = tempfile::tempdir().unwrap();
        let archive = staging.path().join("ffmpeg.zip");
        let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
        zip.add_symlink(
            "ffmpeg",
            "../other",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.finish().unwrap();

        assert!(unpack(&archive, staging.path(), &["ffmpeg"], false).is_err());
        assert!(!staging.path().join("ffmpeg").exists());
    }
}
