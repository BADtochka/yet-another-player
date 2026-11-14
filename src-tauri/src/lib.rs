use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use tauri::{ipc::Response, Manager, State};
use tokio::io::AsyncWriteExt;

mod audio;
mod cover;
mod external_tools;
mod genius;
mod library;
mod linux_audio;
mod lyrica;
mod track_lyrics;
mod updates;

pub struct MusicFolders {
    canonical: Arc<Mutex<Vec<PathBuf>>>,
    configured: Mutex<Vec<String>>,
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> &str {
    panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("unknown panic")
}

#[tauri::command]
async fn append_app_log(app: tauri::AppHandle, line: String) -> Result<(), String> {
    if line.len() > 128 * 1024 {
        return Err("Запись журнала слишком длинная.".into());
    }
    let logs = app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?
        .join("logs");
    tokio::fs::create_dir_all(&logs)
        .await
        .map_err(|error| error.to_string())?;
    let mut file = tokio::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(logs.join("yap.log"))
        .await
        .map_err(|error| error.to_string())?;
    file.write_all(line.as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    file.flush().await.map_err(|error| error.to_string())
}

#[tauri::command]
async fn get_cover(
    app: tauri::AppHandle,
    folders: State<'_, MusicFolders>,
    path: String,
) -> Result<Response, String> {
    let cache = app
        .path()
        .app_cache_dir()
        .map_err(|error| error.to_string())?
        .join("covers");
    let roots = Arc::clone(&folders.canonical);
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        let path = std::fs::canonicalize(path).ok()?;
        if !path.is_file() || !roots.lock().ok()?.iter().any(|root| path.starts_with(root)) {
            return None;
        }
        cover::load_cover(&path, &cache)
            .map(|(bytes, _)| bytes)
            .or_else(|| {
                let library = app.state::<library::Library>();
                library
                    .resolver_cover(&path)
                    .is_none()
                    .then(|| library.external_cover(&path))
                    .flatten()
            })
    })
    .await
    .map_err(|error| error.to_string())?;
    Ok(Response::new(bytes.unwrap_or_default()))
}

#[tauri::command]
async fn reveal_track(folders: State<'_, MusicFolders>, path: String) -> Result<(), String> {
    let roots = Arc::clone(&folders.canonical);
    tauri::async_runtime::spawn_blocking(move || {
        let path = audio::library_track_path(&path, &roots)?;
        tauri_plugin_opener::reveal_item_in_dir(path).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn local_track_video(
    app: tauri::AppHandle,
    folders: State<'_, MusicFolders>,
    path: String,
) -> Result<Option<PathBuf>, String> {
    let roots = Arc::clone(&folders.canonical);
    tauri::async_runtime::spawn_blocking(move || {
        let audio_path = audio::library_track_path(&path, &roots)?;
        let library = app.state::<library::Library>();
        let Some(path) = library
            .local_video(&audio_path)
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        let path = audio::library_track_path(&path, &roots)?;
        if !library::versions::has_video(&path) {
            return Ok(None);
        }
        let scope = app.asset_protocol_scope();
        scope.allow_file(&path).map_err(|error| error.to_string())?;
        Ok(Some(path))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TrackFileInfo {
    size: u64,
    format: String,
}

#[tauri::command]
async fn track_file_info(
    folders: State<'_, MusicFolders>,
    path: String,
) -> Result<TrackFileInfo, String> {
    let roots = Arc::clone(&folders.canonical);
    tauri::async_runtime::spawn_blocking(move || {
        let path = audio::library_track_path(&path, &roots)?;
        let size = path.metadata().map_err(|error| error.to_string())?.len();
        let format = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_uppercase();
        Ok(TrackFileInfo { size, format })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn get_resolver_cover(
    app: tauri::AppHandle,
    folders: State<'_, MusicFolders>,
    path: String,
) -> Result<Option<String>, String> {
    let roots = Arc::clone(&folders.canonical);
    tauri::async_runtime::spawn_blocking(move || {
        let path = audio::library_track_path(&path, &roots)?;
        Ok(app.state::<library::Library>().resolver_cover(&path))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn get_lyrics(
    folders: State<'_, MusicFolders>,
    app: tauri::AppHandle,
    path: String,
) -> Result<Option<LocalLyrics>, String> {
    let roots = Arc::clone(&folders.canonical);
    tauri::async_runtime::spawn_blocking(move || {
        let track = audio::library_track_path(&path, &roots).ok()?;
        let local = (|| {
            let lyrics = std::fs::canonicalize(track.with_extension("lrc")).ok()?;
            if !lyrics.is_file()
                || !roots
                    .lock()
                    .ok()?
                    .iter()
                    .any(|root| track.starts_with(root) && lyrics.starts_with(root))
            {
                return None;
            }
            let file = std::fs::File::open(lyrics).ok()?;
            let mut text = String::new();
            use std::io::Read;
            file.take(256 * 1024 + 1).read_to_string(&mut text).ok()?;
            (text.len() <= 256 * 1024 && !text.trim().is_empty()).then_some(text)
        })();
        let fetched = app.state::<library::Library>().fetched_lyrics(&track);
        Some(LocalLyrics {
            sidecar: local,
            fetched,
        })
    })
    .await
    .map_err(|error| error.to_string())
}

#[derive(serde::Serialize)]
struct LocalLyrics {
    sidecar: Option<String>,
    fetched: Option<String>,
}

#[tauri::command]
fn get_audio_environment(
    state: State<'_, linux_audio::AudioEnvironment>,
) -> linux_audio::AudioEnvironment {
    state.inner().clone()
}

#[tauri::command]
fn set_music_roots(folders: State<'_, MusicFolders>, paths: Vec<String>) -> Result<(), String> {
    let mut roots = Vec::new();
    for path in &paths {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(path) = std::fs::canonicalize(trimmed) {
            if path.is_dir() && !roots.contains(&path) {
                roots.push(path);
            }
        }
    }
    *folders
        .canonical
        .lock()
        .map_err(|_| "Медиатека недоступна")? = roots;
    *folders
        .configured
        .lock()
        .map_err(|_| "Медиатека недоступна")? = paths;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let audio_environment = linux_audio::configure();
    let roots = Arc::new(Mutex::new(Vec::new()));

    tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(audio_environment)
        .manage(external_tools::ommr::LocalServer::default())
        .manage(external_tools::downloads::Downloads::default())
        .manage(MusicFolders {
            canonical: Arc::clone(&roots),
            configured: Mutex::default(),
        })
        .plugin(tauri_plugin_zustand::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            let catalog = app.path().app_data_dir()?.join("library.sqlite3");
            app.manage(
                library::Library::new(catalog)
                    .map_err(|error| std::io::Error::other(error.to_string()))?,
            );
            app.manage(audio::AudioEngine::start(app.handle().clone(), roots)?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            append_app_log,
            updates::uses_arch_package,
            updates::check_arch_update,
            updates::download_arch_update,
            audio::audio_control,
            audio::info::track_audio_info,
            external_tools::check_yt_dlp,
            external_tools::get_yt_dlp_location,
            external_tools::yt_dlp_bundle::install_yt_dlp_bundle,
            external_tools::yt_dlp_bundle::latest_yt_dlp_version,
            external_tools::ommr::check_ommr,
            external_tools::ommr::install_ommr,
            external_tools::ommr::test_ommr,
            external_tools::ffmpeg::check_ffmpeg,
            external_tools::ffmpeg::install_ffmpeg,
            external_tools::install::cancel_tool_install,
            external_tools::install::remove_external_tool,
            external_tools::open_macos_privacy_settings,
            external_tools::preview::preview_yt_dlp,
            external_tools::search::search_yt_dlp,
            external_tools::downloads::download_yt_dlp,
            external_tools::downloads::download_yt_dlp_video,
            external_tools::downloads::external_video_available,
            external_tools::downloads::cancel_yt_dlp_download,
            get_audio_environment,
            get_cover,
            get_resolver_cover,
            local_track_video,
            get_lyrics,
            reveal_track,
            track_file_info,
            track_lyrics::save_track_lyrics,
            lyrica::lookup_lyrica_lyrics,
            genius::genius_is_configured,
            genius::genius_save_token,
            genius::genius_disconnect,
            genius::genius_search,
            library::library_catalog_status,
            library::queries::library_tracks_page,
            library::queries::library_play_tracks,
            library::queries::library_tracks_by_paths,
            library::external::library_external_tracks,
            library::queries::library_collections,
            library::queries::library_collection_tracks,
            library::playlists::library_playlists,
            library::library_scan,
            library::library_enrich_download,
            library::library_cancel,
            library::library_resolve_path,
            set_music_roots,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                app.state::<external_tools::ommr::LocalServer>().shutdown();
            }
        });
}
