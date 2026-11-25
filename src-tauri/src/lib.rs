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
#[cfg(target_os = "macos")]
mod page_swipe;
mod track_lyrics;
mod updates;
mod volume_gesture;

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
        let path = audio::library_track_path(&path, &roots).ok()?;
        let library = app.state::<library::Library>();
        let mut candidates = library.album_cover_paths(path.to_str()?).ok()?;
        if let Some(first) = candidates.first() {
            if let Ok(track) = audio::library_track_path(first, &roots) {
                if let Some(bytes) = cover::folder_cover(&track) {
                    return Some(bytes);
                }
            }
        } else {
            candidates.push(path.to_str()?.to_owned());
        }
        for candidate in candidates {
            let Ok(candidate) = audio::library_track_path(&candidate, &roots) else {
                continue;
            };
            if let Some(bytes) = cover::load_cover(&candidate, &cache).map(|(bytes, _)| bytes) {
                return Some(bytes);
            }
            if let Some(bytes) = library.resolver_cover(&candidate) {
                return Some(bytes);
            }
            if let Some(bytes) = library
                .external_cover(&candidate)
                .filter(|bytes| !bytes.is_empty())
            {
                return Some(bytes);
            }
        }
        library.release_cover(&path)
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

#[tauri::command]
async fn start_track_clip(
    app: tauri::AppHandle,
    audio: State<'_, audio::AudioEngine>,
    folders: State<'_, MusicFolders>,
    path: String,
    video_id: Option<String>,
    id: u64,
    autoplay: bool,
) -> Result<audio::Snapshot, String> {
    let roots = Arc::clone(&folders.canonical);
    let track_path = path.clone();
    let file = tauri::async_runtime::spawn_blocking(move || {
        let track = audio::library_track_path(&track_path, &roots)?;
        let library = app.state::<library::Library>();
        let unavailable = "Файл клипа недоступен";
        if let Some(video_id) = video_id {
            if !library
                .owns_clip(&track, &video_id)
                .map_err(|_| unavailable)?
            {
                return Err(unavailable);
            }
            let file =
                external_tools::downloads::clip_path(&app, &video_id).map_err(|_| unavailable)?;
            let metadata = std::fs::symlink_metadata(&file).map_err(|_| unavailable)?;
            if metadata.is_file() && metadata.len() > 0 {
                return Ok(file);
            }
            return Err(unavailable);
        }
        let file = library
            .local_video(&track)
            .map_err(|_| unavailable)?
            .ok_or(unavailable)?;
        audio::library_track_path(&file, &roots)
    })
    .await
    .map_err(|error| error.to_string())??;
    audio
        .control(audio::Action::StartClip {
            id,
            track: path,
            file,
            autoplay,
        })
        .await
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
async fn get_lyrics(
    folders: State<'_, MusicFolders>,
    app: tauri::AppHandle,
    path: String,
) -> Result<Option<LocalLyrics>, String> {
    let roots = Arc::clone(&folders.canonical);
    tauri::async_runtime::spawn_blocking(move || {
        let track = audio::library_track_path(&path, &roots).ok()?;
        let local = track_lyrics::read_sidecar(&track, &roots);
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

    let builder = tauri::Builder::default();
    #[cfg(target_os = "macos")]
    let builder =
        builder.append_invoke_initialization_script("window.__YAP_NATIVE_PAGE_SWIPE__ = true;");

    builder
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(audio_environment)
        .manage(volume_gesture::VolumeGesture::default())
        .manage(external_tools::ommr::LocalServer::default())
        .manage(external_tools::sync_server::LocalServer::default())
        .manage(external_tools::lyrica_server::LocalLyrica::default())
        .manage(external_tools::downloads::Downloads::default())
        .manage(MusicFolders {
            canonical: Arc::clone(&roots),
            configured: Mutex::default(),
        })
        .plugin(tauri_plugin_zustand::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                page_swipe::install(&window)?;
            }
            let data = app.path().app_data_dir()?;
            app.manage(external_tools::enabled::DisabledTools::load(&data));
            let catalog = data.join("library.sqlite3");
            app.manage(
                library::Library::new(catalog)
                    .map_err(|error| std::io::Error::other(error.to_string()))?,
            );
            app.manage(audio::AudioEngine::start(app.handle().clone(), roots)?);
            volume_gesture::start(app.handle());
            library::trash::start_maintenance(app.handle());
            external_tools::sync_server::LocalServer::supervise(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            append_app_log,
            updates::uses_arch_package,
            updates::check_arch_update,
            updates::download_arch_update,
            audio::audio_control,
            volume_gesture::configure_volume_gesture,
            volume_gesture::volume_gesture_scroll,
            #[cfg(not(target_os = "macos"))]
            volume_gesture::volume_overlay_ready,
            audio::info::track_audio_info,
            external_tools::check_yt_dlp,
            external_tools::get_yt_dlp_location,
            external_tools::yt_dlp_bundle::install_yt_dlp_bundle,
            external_tools::yt_dlp_bundle::latest_yt_dlp_version,
            external_tools::ommr::check_ommr,
            external_tools::ommr::install_ommr,
            external_tools::ommr::test_ommr,
            external_tools::sync_server::sync_server_status,
            external_tools::sync_server::install_sync_server,
            external_tools::sync_server::sync_server_set_enabled,
            external_tools::sync_server::sync_server_devices,
            external_tools::sync_server::sync_server_set_access,
            external_tools::lyrica_server::check_lyrica,
            external_tools::lyrica_server::install_lyrica,
            external_tools::ffmpeg::check_ffmpeg,
            external_tools::ffmpeg::install_ffmpeg,
            external_tools::enabled::disabled_external_tools,
            external_tools::enabled::set_external_tool_enabled,
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
            local_track_video,
            start_track_clip,
            get_lyrics,
            reveal_track,
            track_file_info,
            track_lyrics::save_track_lyrics,
            lyrica::lookup_lyrica_lyrics,
            genius::genius_is_configured,
            genius::genius_save_token,
            genius::genius_disconnect,
            genius::genius_lyrics,
            library::edits::library_edit_track,
            library::edits::library_set_lyrics_offset,
            library::trash::library_trash_track,
            library::trash::library_restore_track,
            library::trash::library_trash_list,
            library::trash::library_trash_cover,
            library::trash::library_purge_trash,
            library::trash::library_set_trash_retention,
            library::library_catalog_status,
            library::queries::library_tracks_page,
            library::queries::library_playback_history,
            library::queries::library_metadata_log,
            library::queries::library_play_tracks,
            library::queries::library_tracks_by_paths,
            library::external::library_external_tracks,
            library::catalog::library_artist_profile,
            library::catalog::library_artist_avatar,
            library::catalog::library_release_details,
            library::catalog::library_search_catalog,
            library::catalog::library_find_release,
            library::catalog::library_find_album_artwork,
            library::catalog::library_album_artwork_image,
            library::catalog::storage::library_catalog_storage,
            library::queries::library_collections,
            library::queries::library_collection_tracks,
            library::playlists::library_playlists,
            library::sync::library_sync_status,
            library::sync::library_sync_discover,
            library::sync::library_sync_check_access,
            library::sync::library_sync_configure,
            library::sync::library_sync_run,
            library::sync::library_sync_cancel,
            library::playlists::library_play_playlist,
            library::playlists::library_playlist_transition_analysis,
            library::playlists::library_preview_playlist_transition,
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
                app.state::<external_tools::sync_server::LocalServer>().shutdown();
                app.state::<external_tools::lyrica_server::LocalLyrica>()
                    .shutdown();
            }
        });
}
