use std::{
    fs, io,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};

use image::{codecs::jpeg::JpegEncoder, ImageFormat, ImageReader, Limits};
use lofty::{
    config::WriteOptions,
    file::{AudioFile, TaggedFileExt},
    picture::{Picture, PictureType},
    probe::Probe,
    tag::{items::Timestamp, Accessor, ItemKey, Tag},
};
use serde::{Deserialize, Serialize};
use tauri::{ipc::Channel, Manager, State};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::{Child, ChildStdout},
    sync::watch,
    time::{sleep, timeout},
};

use super::{installed_yt_dlp, interrupted_launch, source_video_url, yt_dlp_command, YtDlpError};
use crate::{
    library::{self, external, Library, Track},
    MusicFolders,
};

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(20 * 60);
const INFO_FILE: &str = "source.info.json";
const MAX_COVER_SIZE: u64 = 2 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum JobPhase {
    Running,
    Cancelled,
    Importing,
}

struct Job {
    id: String,
    phase: watch::Sender<JobPhase>,
}

#[derive(Default)]
pub(crate) struct Downloads(Mutex<Option<Arc<Job>>>);

struct ActiveDownload<'a> {
    state: &'a Downloads,
    job: Arc<Job>,
}

impl Downloads {
    fn start(&self, id: String) -> Result<ActiveDownload<'_>, YtDlpError> {
        if id.is_empty()
            || id.len() > 64
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(YtDlpError::InvalidDownload);
        }
        let mut active = self.0.lock().map_err(|_| YtDlpError::DownloadFailed)?;
        if active.is_some() {
            return Err(YtDlpError::DownloadBusy);
        }
        let job = Arc::new(Job {
            id,
            phase: watch::Sender::new(JobPhase::Running),
        });
        *active = Some(Arc::clone(&job));
        Ok(ActiveDownload { state: self, job })
    }

    fn cancel(&self, id: &str) -> bool {
        let Ok(active) = self.0.lock() else {
            return false;
        };
        let Some(job) = active.as_ref().filter(|job| job.id == id) else {
            return false;
        };
        job.phase.send_if_modified(|phase| {
            if *phase == JobPhase::Running {
                *phase = JobPhase::Cancelled;
                true
            } else {
                false
            }
        });
        let cancelled = *job.phase.borrow() == JobPhase::Cancelled;
        cancelled
    }
}

impl Drop for ActiveDownload<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.state.0.lock() {
            *active = None;
        }
    }
}

impl Job {
    async fn cancelled(&self) {
        let mut phase = self.phase.subscribe();
        let _ = phase.wait_for(|phase| *phase == JobPhase::Cancelled).await;
    }

    fn begin_import(&self) -> Result<(), YtDlpError> {
        if self.phase.send_if_modified(|phase| {
            if *phase != JobPhase::Running {
                return false;
            }
            *phase = JobPhase::Importing;
            true
        }) {
            Ok(())
        } else {
            Err(YtDlpError::DownloadCancelled)
        }
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(tag = "phase", rename_all = "camelCase")]
pub(crate) enum DownloadProgress {
    Preparing,
    Downloading { percent: Option<f64> },
    Processing,
    WaitingForLibrary,
    Importing,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DownloadResult {
    track: Track,
    already_present: bool,
}

#[tauri::command]
pub(crate) fn cancel_yt_dlp_download(downloads: State<'_, Downloads>, id: String) -> bool {
    downloads.cancel(&id)
}

#[tauri::command]
pub(crate) async fn download_yt_dlp(
    app: tauri::AppHandle,
    downloads: State<'_, Downloads>,
    folders: State<'_, MusicFolders>,
    id: String,
    video_id: String,
    folder_index: usize,
    with_video: Option<bool>,
    progress: Channel<DownloadProgress>,
) -> Result<DownloadResult, YtDlpError> {
    let url = source_video_url(&video_id)?;
    let active = downloads.start(id)?;
    let _ = progress.send(DownloadProgress::Preparing);
    let existing = {
        let app = app.clone();
        let lookup_id = video_id.clone();
        let roots = folders
            .canonical
            .lock()
            .map_err(|_| YtDlpError::TargetUnavailable)?
            .clone();
        tauri::async_runtime::spawn_blocking(move || {
            let ids = [lookup_id];
            app.state::<Library>()
                .active_external_tracks(&ids, &roots)
                .map(|mut tracks| tracks.remove(&ids[0]))
        })
        .await
        .map_err(|_| YtDlpError::ImportFailed)?
        .map_err(|error| {
            eprintln!("yt-dlp catalog lookup: {error}");
            YtDlpError::ImportFailed
        })?
    };
    if let Some(track) = existing {
        active.job.begin_import()?;
        return Ok(DownloadResult {
            track,
            already_present: true,
        });
    }
    let configured_root = folders
        .configured
        .lock()
        .map_err(|_| YtDlpError::TargetUnavailable)?
        .get(folder_index)
        .cloned()
        .ok_or(YtDlpError::TargetUnavailable)?;
    let root = tokio::fs::canonicalize(&configured_root)
        .await
        .map_err(|_| YtDlpError::TargetUnavailable)?;
    if !folders
        .canonical
        .lock()
        .map_err(|_| YtDlpError::TargetUnavailable)?
        .contains(&root)
    {
        return Err(YtDlpError::TargetUnavailable);
    }

    let executable = installed_yt_dlp(&app).await?;
    let stage = tempfile::Builder::new()
        .prefix(".yap-download-")
        .tempdir_in(&root)
        .map_err(|error| {
            eprintln!("yt-dlp staging: {error}");
            YtDlpError::TargetUnavailable
        })?;
    let stage_path = stage.path().to_path_buf();
    let ffmpeg = if with_video == Some(true) {
        super::ffmpeg::installed(&app).await.ok().flatten()
    } else {
        None
    };
    let video_id_for_cache = video_id.clone();
    let video_app = app.clone();
    let result = async {
        let output = fetch_media(
            &executable.path,
            &stage_path,
            &url,
            if with_video == Some(true) {
                DownloadKind::Combined(ffmpeg.as_ref().map(|tool| tool.path.as_path()))
            } else {
                DownloadKind::Audio
            },
            &active.job,
            &progress,
        )
        .await;
        drop(executable);
        let output = output?;
        let _ = progress.send(DownloadProgress::WaitingForLibrary);
        let writer = {
            let library = app.state::<Library>();
            tokio::select! {
                biased;
                _ = active.job.cancelled() => return Err(YtDlpError::DownloadCancelled),
                writer = library.lock_catalog_writer() => writer,
            }
        };
        active.job.begin_import()?;
        let _ = progress.send(DownloadProgress::Importing);
        let stage_path = stage_path.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let _writer = writer;
            let folders = app.state::<MusicFolders>();
            let configured = folders
                .configured
                .lock()
                .map_err(|_| YtDlpError::TargetUnavailable)?
                .clone();
            if !configured.contains(&configured_root)
                || fs::canonicalize(&configured_root).ok().as_ref() != Some(&root)
            {
                return Err(YtDlpError::TargetUnavailable);
            }
            publish_audio(
                &app.state::<Library>(),
                &root,
                &configured,
                &stage_path,
                &output,
                &video_id,
            )
        })
        .await
        .map_err(|_| YtDlpError::ImportFailed)?
    }
    .await;

    if let Err(error) = remove_staging(&stage_path).await {
        eprintln!("yt-dlp cleanup: {error}");
        if result.is_err() {
            return Err(YtDlpError::CleanupFailed);
        }
    }
    if with_video == Some(true) {
        if let Ok(download) = &result {
            if !download.already_present {
                if Path::new(download.track.path())
                    .extension()
                    .is_some_and(|ext| ext == "mp4")
                {
                    if let Ok(destination) =
                        video_destination(&video_app, &video_id_for_cache).await
                    {
                        if let Err(error) =
                            tokio::fs::hard_link(download.track.path(), &destination).await
                        {
                            eprintln!("Video cache: {error}");
                        }
                    }
                }
            }
        }
    }
    result
}

const VIDEO_FORMAT: &str = "bestvideo[ext=mp4][vcodec^=avc1]/best[ext=mp4][vcodec^=avc1]";

#[tauri::command]
pub(crate) async fn external_video_available(
    app: tauri::AppHandle,
    video_id: String,
) -> Result<bool, YtDlpError> {
    let url = source_video_url(&video_id)?;
    let executable = installed_yt_dlp(&app).await?;
    super::search::is_playable_clip(&executable.path, &video_id, &url).await
}

async fn video_destination(app: &tauri::AppHandle, id: &str) -> Result<PathBuf, YtDlpError> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|_| YtDlpError::TargetUnavailable)?
        .join("videos");
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|_| YtDlpError::TargetUnavailable)?;
    Ok(directory.join(format!("{id}.mp4")))
}

#[derive(Clone, Copy)]
enum DownloadKind<'a> {
    Audio,
    Combined(Option<&'a Path>),
    Video,
}

#[tauri::command]
pub(crate) async fn download_yt_dlp_video(
    app: tauri::AppHandle,
    downloads: State<'_, Downloads>,
    id: String,
    video_id: String,
    progress: Channel<DownloadProgress>,
) -> Result<PathBuf, YtDlpError> {
    let url = source_video_url(&video_id)?;
    let destination = video_destination(&app, &video_id).await?;
    let directory = destination.parent().ok_or(YtDlpError::TargetUnavailable)?;
    if tokio::fs::symlink_metadata(&destination)
        .await
        .is_ok_and(|file| file.is_file() && file.len() > 0)
    {
        return Ok(destination);
    }

    let active = downloads.start(id)?;
    let _ = progress.send(DownloadProgress::Preparing);
    let executable = installed_yt_dlp(&app).await?;
    let stage = tempfile::Builder::new()
        .prefix(".yap-video-")
        .tempdir_in(directory)
        .map_err(|_| YtDlpError::TargetUnavailable)?;
    let output = fetch_media(
        &executable.path,
        stage.path(),
        &url,
        DownloadKind::Video,
        &active.job,
        &progress,
    )
    .await?;
    let output = stage.path().join(output);
    if output.parent() != Some(stage.path())
        || output.extension().and_then(|ext| ext.to_str()) != Some("mp4")
        || !fs::symlink_metadata(&output).is_ok_and(|file| file.is_file() && file.len() > 0)
    {
        return Err(YtDlpError::DownloadFailed);
    }
    active.job.begin_import()?;
    let _ = progress.send(DownloadProgress::Importing);
    tokio::fs::rename(output, &destination)
        .await
        .map_err(|_| YtDlpError::ImportFailed)?;
    Ok(destination)
}

async fn fetch_media(
    executable: &Path,
    stage: &Path,
    url: &str,
    kind: DownloadKind<'_>,
    job: &Job,
    progress: &Channel<DownloadProgress>,
) -> Result<PathBuf, YtDlpError> {
    if *job.phase.borrow() == JobPhase::Cancelled {
        return Err(YtDlpError::DownloadCancelled);
    }
    let mut command = yt_dlp_command(executable);
    command
        .current_dir(stage)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args([
            "--ignore-config",
            "--no-plugin-dirs",
            "--no-cache-dir",
            "--no-playlist",
            "--no-warnings",
            "--newline",
            "--color",
            "never",
            "--progress",
            "--progress-delta",
            "0.25",
            "--socket-timeout",
            "15",
            "--retries",
            "2",
            "--fragment-retries",
            "2",
            "--windows-filenames",
            "--no-simulate",
            "--progress-template",
            "download:YAP_DOWNLOAD:%(progress._percent_str)s",
            "--progress-template",
            "postprocess:YAP_PROCESSING",
            "--print",
            "after_move:YAP_FILE:%(filepath)j",
        ]);
    match kind {
        DownloadKind::Audio | DownloadKind::Combined(_) => command.args([
            "--format",
            match kind {
                DownloadKind::Combined(Some(_)) => "bestvideo[ext=mp4][vcodec^=avc1]+bestaudio[ext=m4a]/best[ext=mp4][vcodec^=avc1][acodec^=mp4a]/bestaudio[ext=m4a]/bestaudio[ext=webm]",
                DownloadKind::Combined(None) => "best[ext=mp4][vcodec^=avc1][acodec^=mp4a]/bestaudio[ext=m4a]/bestaudio[ext=webm]",
                _ => "bestaudio[ext~='^(m4a|webm|opus|ogg|mp3|flac|wav)$']/best[ext=mp4][acodec^=mp4a]",
            },
            "--write-info-json",
            "--no-embed-info-json",
            "--write-thumbnail",
            "--output",
            "infojson:source",
            "--output",
            "thumbnail:cover.%(ext)s",
            "--output",
            "%(title).160B [%(id)s].%(ext)s",
        ]),
        DownloadKind::Video => command.args([
            "--format",
            VIDEO_FORMAT,
            "--output",
            "video.%(ext)s",
        ]),
    };
    if let DownloadKind::Combined(Some(ffmpeg)) = kind {
        command
            .arg("--ffmpeg-location")
            .arg(ffmpeg)
            .args(["--merge-output-format", "mp4"]);
    }
    command.args(["--", url]);

    let mut child = command.spawn().map_err(|_| YtDlpError::LaunchFailed)?;
    let pid = child.id();
    let mut tree = ProcessTree::attach(&child).map_err(|error| {
        eprintln!("yt-dlp process group: {error}");
        YtDlpError::LaunchFailed
    })?;
    let stdout = child.stdout.take().ok_or(YtDlpError::LaunchFailed)?;
    let stderr = child.stderr.take().ok_or(YtDlpError::LaunchFailed)?;
    let outcome = tokio::select! {
        biased;
        _ = job.cancelled() => Err(YtDlpError::DownloadCancelled),
        result = timeout(DOWNLOAD_TIMEOUT, async {
            tokio::try_join!(child.wait(), read_progress(stdout, progress), read_stderr(stderr, progress))
        }) => match result {
            Ok(Ok(outcome)) => Ok(outcome),
            Ok(Err(_)) => Err(YtDlpError::DownloadFailed),
            Err(_) => Err(YtDlpError::DownloadTimedOut),
        },
    };
    let (status, path, age_restricted) = match outcome {
        Ok(outcome) => outcome,
        Err(error) => {
            drop(tree);
            let _ = child.start_kill();
            let _ = timeout(Duration::from_secs(4), child.wait()).await;
            return Err(error);
        }
    };
    tree.completed();
    if let Some(error) = interrupted_launch(&status, pid).await {
        return Err(error);
    }
    if !status.success() {
        return Err(if age_restricted {
            YtDlpError::AgeRestricted
        } else {
            YtDlpError::DownloadFailed
        });
    }
    path.ok_or(YtDlpError::DownloadFailed)
}

async fn read_progress(
    stdout: ChildStdout,
    progress: &Channel<DownloadProgress>,
) -> io::Result<Option<PathBuf>> {
    let mut lines = BufReader::new(stdout.take(1024 * 1024)).lines();
    let mut path = None;
    while let Some(line) = lines.next_line().await? {
        if let Some(value) = line.strip_prefix("YAP_DOWNLOAD:") {
            let percent = value
                .trim()
                .strip_suffix('%')
                .and_then(|value| value.trim().parse::<f64>().ok())
                .filter(|value| (0.0..=100.0).contains(value));
            let _ = progress.send(DownloadProgress::Downloading { percent });
        } else if line == "YAP_PROCESSING" {
            let _ = progress.send(DownloadProgress::Processing);
        } else if let Some(value) = line.strip_prefix("YAP_FILE:") {
            if path.is_some() {
                return Err(io::Error::other("unexpected multiple outputs"));
            }
            let name: String = serde_json::from_str(value).map_err(io::Error::other)?;
            path = Some(PathBuf::from(name));
        }
    }
    Ok(path)
}

async fn read_stderr(
    stderr: tokio::process::ChildStderr,
    progress: &Channel<DownloadProgress>,
) -> io::Result<bool> {
    let mut stderr = BufReader::new(stderr.take(256 * 1024));
    let mut bytes = Vec::new();
    let mut age_restricted = false;
    let mut processing = false;
    while stderr.read_until(b'\n', &mut bytes).await? != 0 {
        let line = String::from_utf8_lossy(&bytes);
        age_restricted |= line.contains("Sign in to confirm your age");
        if !processing && line.trim_end() == "YAP_PROCESSING" {
            processing = true;
            let _ = progress.send(DownloadProgress::Processing);
        }
        bytes.clear();
    }
    Ok(age_restricted)
}

fn publish_audio(
    library: &Library,
    root: &Path,
    roots: &[String],
    stage: &Path,
    output: &Path,
    video_id: &str,
) -> Result<DownloadResult, YtDlpError> {
    let output = stage.join(output);
    let metadata = fs::symlink_metadata(&output).map_err(|_| YtDlpError::DownloadFailed)?;
    let staged = fs::canonicalize(&output).map_err(|_| YtDlpError::DownloadFailed)?;
    if !metadata.is_file()
        || metadata.len() == 0
        || staged.parent() != Some(stage)
        || !library::supported(&staged)
    {
        return Err(YtDlpError::DownloadFailed);
    }
    let extension = staged
        .extension()
        .and_then(|ext| ext.to_str())
        .ok_or(YtDlpError::DownloadFailed)?;
    let stem = staged
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or(YtDlpError::DownloadFailed)?;
    let suffix = format!(" [{video_id}]");
    let fallback = stem.strip_suffix(&suffix).unwrap_or(stem);
    let tags = source_tags(stage, video_id, fallback);
    let cover = source_cover(stage);
    let legacy_path = stem
        .ends_with(&suffix)
        .then(|| root.join(format!("{stem}.{extension}")));
    let known = library.external_paths(video_id).map_err(|error| {
        eprintln!("yt-dlp catalog lookup: {error}");
        YtDlpError::ImportFailed
    })?;
    let existing = known.into_iter().chain(legacy_path).find(|path| {
        path.parent() == Some(root)
            && fs::symlink_metadata(path)
                .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
    });
    let (destination, already_present) = if let Some(path) = existing {
        (path, true)
    } else {
        write_file_tags(&staged, extension, &tags, cover.as_deref());
        let name = file_stem(&tags.title);
        (publish_unique(&staged, root, &name, extension)?, false)
    };
    match library.index_download(
        &destination,
        roots,
        external::Download {
            video_id,
            tags: &tags,
            cover: cover.as_deref(),
        },
    ) {
        Ok(track) => Ok(DownloadResult {
            track,
            already_present,
        }),
        Err(error) => {
            eprintln!("yt-dlp catalog import: {error}");
            if !already_present {
                fs::remove_file(&destination).map_err(|_| YtDlpError::CleanupFailed)?;
            }
            Err(YtDlpError::ImportFailed)
        }
    }
}

#[derive(Default, Deserialize)]
struct SourceInfo {
    id: String,
    track: Option<String>,
    title: Option<String>,
    artists: Option<Vec<String>>,
    artist: Option<String>,
    creator: Option<String>,
    uploader: Option<String>,
    channel: Option<String>,
    album: Option<String>,
    album_artist: Option<String>,
    release_year: Option<u16>,
    track_number: Option<u32>,
    disc_number: Option<u32>,
}

fn nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

fn read_sidecar(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > limit {
        return None;
    }
    let mut data = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(limit + 1)
        .read_to_end(&mut data)
        .ok()?;
    (data.len() as u64 <= limit).then_some(data)
}

fn source_tags(stage: &Path, video_id: &str, fallback: &str) -> external::DownloadTags {
    let info = read_sidecar(&stage.join(INFO_FILE), 4 * 1024 * 1024)
        .and_then(|bytes| serde_json::from_slice::<SourceInfo>(&bytes).ok())
        .filter(|info| info.id == video_id)
        .unwrap_or_default();
    let mut artists = info
        .artists
        .unwrap_or_default()
        .into_iter()
        .filter_map(|name| nonempty(Some(name)))
        .collect::<Vec<_>>();
    if artists.is_empty() {
        if let Some(credit) = nonempty(info.artist).or_else(|| nonempty(info.creator)) {
            artists.extend(super::music_metadata::split_credits(&credit).map(str::to_owned));
        }
    }
    let is_clip = info
        .title
        .as_deref()
        .is_some_and(super::music_metadata::has_clip_label);
    let title = nonempty(info.track)
        .or_else(|| nonempty(info.title))
        .unwrap_or_else(|| fallback.to_owned());
    let title = super::music_metadata::normalize(&title, &mut artists);
    if artists.is_empty() {
        if let Some(channel) = nonempty(info.uploader).or_else(|| nonempty(info.channel)) {
            artists.push(
                channel
                    .strip_suffix(" - Topic")
                    .unwrap_or(&channel)
                    .to_owned(),
            );
        }
    }
    external::DownloadTags {
        title,
        is_clip,
        artists,
        album: nonempty(info.album),
        album_artist: nonempty(info.album_artist),
        year: info.release_year,
        track_number: info.track_number,
        disc_number: info.disc_number,
    }
}

fn source_cover(stage: &Path) -> Option<Vec<u8>> {
    ["jpg", "jpeg", "png", "webp"]
        .into_iter()
        .filter_map(|ext| read_sidecar(&stage.join(format!("cover.{ext}")), MAX_COVER_SIZE))
        .find(|bytes| {
            bytes.starts_with(b"\xff\xd8\xff")
                || bytes.starts_with(b"\x89PNG\r\n\x1a\n")
                || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"))
        })
}

fn file_stem(title: &str) -> String {
    let mut name = String::new();
    for character in title.chars() {
        if name.len() + character.len_utf8() > 160 {
            break;
        }
        name.push(
            if character.is_control() || "<>:\"/\\|?*".contains(character) {
                '_'
            } else {
                character
            },
        );
    }
    let name = name.trim_matches([' ', '.']);
    if name.is_empty() {
        return "Аудиотрек".to_owned();
    }
    let prefix = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_uppercase();
    if matches!(
        prefix.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|tag| {
        prefix.strip_prefix(tag).is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    }) {
        return format!("_{name}");
    }
    name.to_owned()
}

fn publish_unique(
    staged: &Path,
    root: &Path,
    name: &str,
    extension: &str,
) -> Result<PathBuf, YtDlpError> {
    let mut temp =
        tempfile::TempPath::try_from_path(staged).map_err(|_| YtDlpError::ImportFailed)?;
    for index in 1..=1000 {
        let filename = if index == 1 {
            format!("{name}.{extension}")
        } else {
            format!("{name} ({index}).{extension}")
        };
        let path = root.join(filename);
        match temp.persist_noclobber(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => temp = error.path,
            Err(error) => {
                eprintln!("yt-dlp publish: {}", error.error);
                break;
            }
        }
    }
    Err(YtDlpError::ImportFailed)
}

fn write_file_tags(
    path: &Path,
    extension: &str,
    tags: &external::DownloadTags,
    cover: Option<&[u8]>,
) {
    // WebM and other files still get the same metadata and cover through the catalog.
    if !matches!(extension, "mp3" | "m4a") {
        return;
    }
    let result = (|| -> library::LibraryResult<()> {
        let mut file = Probe::open(path)?.read()?;
        let tag_type = file.primary_tag_type();
        let tag = if let Some(tag) = file.primary_tag_mut() {
            tag
        } else {
            file.insert_tag(Tag::new(tag_type));
            file.primary_tag_mut().ok_or("Cannot write primary tag")?
        };
        tag.set_title(tags.title.clone());
        if !tags.artists.is_empty() {
            tag.set_artist(tags.artists.join("; "));
        }
        if let Some(album) = &tags.album {
            tag.set_album(album.clone());
        }
        if let Some(artist) = &tags.album_artist {
            tag.insert_text(ItemKey::AlbumArtist, artist.clone());
        }
        if let Some(track) = tags.track_number {
            tag.set_track(track);
        }
        if let Some(disc) = tags.disc_number {
            tag.set_disk(disc);
        }
        if let Some(year) = tags.year {
            tag.set_date(Timestamp {
                year,
                ..Timestamp::default()
            });
        }
        if tag.pictures().is_empty() {
            if let Some(picture) = cover.and_then(embedded_cover) {
                tag.push_picture(picture);
            }
        }
        Ok(file.save_to_path(path, WriteOptions::default())?)
    })();
    if let Err(error) = result {
        eprintln!("yt-dlp could not write optional audio tags: {error}");
    }
}

fn embedded_cover(bytes: &[u8]) -> Option<Picture> {
    let mut jpeg = Vec::new();
    let mut picture_bytes = if bytes.starts_with(b"RIFF") {
        let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::WebP);
        let mut limits = Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(4096);
        limits.max_alloc = Some(96 * 1024 * 1024);
        reader.limits(limits);
        let rgb = reader.decode().ok()?.into_rgb8();
        JpegEncoder::new_with_quality(&mut jpeg, 90)
            .encode_image(&rgb)
            .ok()?;
        if jpeg.len() as u64 > MAX_COVER_SIZE {
            return None;
        }
        jpeg.as_slice()
    } else {
        bytes
    };
    let mut picture = Picture::from_reader(&mut picture_bytes).ok()?;
    picture.set_pic_type(PictureType::CoverFront);
    Some(picture)
}

async fn remove_staging(path: &Path) -> io::Result<()> {
    for attempt in 0..=20 {
        match tokio::fs::remove_dir_all(path).await {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error)
                if cfg!(windows)
                    && attempt < 20
                    && error.kind() == io::ErrorKind::PermissionDenied =>
            {
                sleep(Duration::from_millis(50)).await;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub(super) struct ProcessTree {
    #[cfg(unix)]
    group: Option<i32>,
    #[cfg(windows)]
    _job: std::os::windows::io::OwnedHandle,
}

impl ProcessTree {
    #[cfg(unix)]
    pub(super) fn attach(child: &Child) -> io::Result<Self> {
        let group = child
            .id()
            .and_then(|pid| i32::try_from(pid).ok())
            .filter(|pid| *pid > 1)
            .ok_or_else(|| io::Error::other("missing process group"))?;
        Ok(Self { group: Some(group) })
    }

    #[cfg(windows)]
    pub(super) fn attach(child: &Child) -> io::Result<Self> {
        use std::{
            os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
            ptr,
        };
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };

        let process = child
            .raw_handle()
            .ok_or_else(|| io::Error::other("missing process handle"))?;
        // SAFETY: Both null arguments request an unnamed job with default security.
        let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if job.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: CreateJobObjectW returned a new handle owned by this call.
        let job = unsafe { OwnedHandle::from_raw_handle(job) };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: The job and child handles are live and the buffer matches the information class.
        unsafe {
            if SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                ptr::from_ref(&limits).cast(),
                std::mem::size_of_val(&limits) as u32,
            ) == 0
                || AssignProcessToJobObject(job.as_raw_handle(), process) == 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(Self { _job: job })
    }

    pub(super) fn completed(&mut self) {
        #[cfg(unix)]
        {
            self.group = None;
        }
    }
}

#[cfg(unix)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        if let Some(group) = self.group {
            // SAFETY: The child was spawned in its own positive process group.
            unsafe { libc::kill(-group, libc::SIGKILL) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WAV: &[u8] = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x40\x1f\0\0\x80\x3e\0\0\x02\0\x10\0data\x02\0\0\0\0\0";

    #[test]
    fn external_webp_can_be_embedded_as_a_jpeg_cover() {
        let mut webp = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut webp)
            .encode(&[16, 64, 128, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        let picture = embedded_cover(&webp).unwrap();
        assert_eq!(picture.pic_type(), PictureType::CoverFront);
        assert!(picture.data().starts_with(b"\xff\xd8\xff"));
    }

    #[test]
    fn publish_indexes_a_completed_track_and_keeps_an_existing_file_on_retry() {
        let directory = tempfile::tempdir().unwrap();
        let library = Library::new(directory.path().join("library.sqlite3")).unwrap();
        let root = directory.path().join("music");
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let roots = vec![root.to_string_lossy().into_owned()];
        let stage = tempfile::Builder::new()
            .prefix(".yap-download-")
            .tempdir_in(&root)
            .unwrap();
        let video_id = "abcdefghijkl";
        let name = Path::new("Video title [abcdefghijkl].wav");
        fs::write(stage.path().join(INFO_FILE), r#"{"id":"abcdefghijkl","title":"Video title","track":"Song","artists":["First","Second"],"uploader":"Channel","album":"Album","release_year":2025}"#).unwrap();
        let artwork = b"\x89PNG\r\n\x1a\nfixture";
        fs::write(stage.path().join("cover.png"), artwork).unwrap();
        fs::write(root.join("Song.wav"), b"unrelated local file").unwrap();
        fs::write(stage.path().join(name), WAV).unwrap();
        let first = publish_audio(&library, &root, &roots, stage.path(), name, video_id).unwrap();
        assert!(!first.already_present);
        assert!(!stage.path().join(name).exists());
        let destination = root.join("Song (2).wav");
        let first = serde_json::to_value(first.track).unwrap();
        assert_eq!(first["path"], destination.to_str().unwrap());
        assert_eq!(first["name"], "Song");
        assert_eq!(first["artists"], serde_json::json!(["First", "Second"]));
        assert_eq!(first["album"], "Album");
        assert_eq!(first["year"], 2025);
        assert_eq!(
            library.external_cover(&destination).as_deref(),
            Some(artwork.as_slice())
        );

        fs::write(stage.path().join(name), b"second copy").unwrap();
        fs::remove_file(stage.path().join("cover.png")).unwrap();
        let second = publish_audio(&library, &root, &roots, stage.path(), name, video_id).unwrap();
        assert!(second.already_present);
        assert_eq!(first, serde_json::to_value(second.track).unwrap());
        assert_eq!(fs::read(&destination).unwrap(), WAV);
        assert_eq!(
            fs::read(root.join("Song.wav")).unwrap(),
            b"unrelated local file"
        );
        assert_eq!(
            library.external_cover(&destination).as_deref(),
            Some(artwork.as_slice())
        );
        let ids = [video_id.to_owned()];
        let linked = library
            .active_external_tracks(&ids, std::slice::from_ref(&root))
            .unwrap();
        assert_eq!(serde_json::to_value(&linked[video_id]).unwrap(), first);
        assert!(library
            .active_external_tracks(&ids, &[])
            .unwrap()
            .is_empty());
        fs::remove_file(&destination).unwrap();
        assert!(library
            .active_external_tracks(&ids, &[root])
            .unwrap()
            .is_empty());
    }

    #[test]
    fn publish_rejects_a_path_outside_the_staging_directory() {
        let directory = tempfile::tempdir().unwrap();
        let library = Library::new(directory.path().join("library.sqlite3")).unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        let stage = tempfile::tempdir_in(&root).unwrap();
        let outside = root.join("other.wav");
        fs::write(&outside, WAV).unwrap();

        let result = publish_audio(&library, &root, &[], stage.path(), &outside, "abcdefghijk");
        assert!(matches!(result, Err(YtDlpError::DownloadFailed)));
        assert_eq!(fs::read(outside).unwrap(), WAV);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn cancelling_stops_the_download_process_and_leaves_no_importable_file() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("test-dlp");
        fs::write(&executable, "#!/bin/sh\nprintf partial > track.wav.part\nprintf 'YAP_DOWNLOAD: 20%%\\n'\n/bin/sleep 60 &\nwait\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let stage = tempfile::Builder::new()
            .prefix(".yap-download-")
            .tempdir_in(directory.path())
            .unwrap();
        let state = Downloads::default();
        let active = state.start("test-job".into()).unwrap();
        let (started, mut signal) = tokio::sync::watch::channel(false);
        let progress = Channel::new(move |_| {
            let _ = started.send(true);
            Ok(())
        });
        let cancel = async {
            timeout(Duration::from_secs(5), signal.wait_for(|started| *started))
                .await
                .unwrap()
                .unwrap();
            assert!(state.cancel("test-job"));
        };
        let (result, ()) = tokio::join!(
            fetch_media(
                &executable,
                stage.path(),
                "https://example.invalid",
                DownloadKind::Audio,
                &active.job,
                &progress
            ),
            cancel,
        );

        assert!(matches!(result, Err(YtDlpError::DownloadCancelled)));
        assert!(active.job.begin_import().is_err());
        remove_staging(stage.path()).await.unwrap();
        assert!(!stage.path().exists());
    }
}
