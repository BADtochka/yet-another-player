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
        app.state::<library::Library>()
            .youtube_cover(&path)
            .or_else(|| cover::load_cover(&path, &cache).map(|(bytes, _)| bytes))
    })
    .await
    .map_err(|error| error.to_string())?;
    Ok(Response::new(bytes.unwrap_or_default()))
}

#[tauri::command]
async fn get_lyrics(
    folders: State<'_, MusicFolders>,
    path: String,
) -> Result<Option<String>, String> {
    let roots = Arc::clone(&folders.canonical);
    tauri::async_runtime::spawn_blocking(move || {
        let track = std::fs::canonicalize(path).ok()?;
        let lyrics = std::fs::canonicalize(track.with_extension("lrc")).ok()?;
        let roots = roots.lock().ok()?;
        if !roots
            .iter()
            .any(|root| track.starts_with(root) && lyrics.starts_with(root))
        {
            return None;
        }
        drop(roots);
        let file = std::fs::File::open(lyrics).ok()?;
        if !file.metadata().ok()?.is_file() {
            return None;
        }
        let mut text = String::new();
        use std::io::Read;
        file.take(256 * 1024 + 1).read_to_string(&mut text).ok()?;
        (text.len() <= 256 * 1024).then_some(text)
    })
    .await
    .map_err(|error| error.to_string())
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
            external_tools::ffmpeg::check_ffmpeg,
            external_tools::ffmpeg::install_ffmpeg,
            external_tools::install::cancel_tool_install,
            external_tools::install::remove_external_tool,
            external_tools::open_macos_privacy_settings,
            external_tools::preview::preview_yt_dlp,
            external_tools::search::search_yt_dlp,
            external_tools::downloads::download_yt_dlp,
            external_tools::downloads::cancel_yt_dlp_download,
            get_audio_environment,
            get_cover,
            get_lyrics,
            lyrica::lookup_lyrica_lyrics,
            genius::genius_is_configured,
            genius::genius_save_token,
            genius::genius_disconnect,
            genius::genius_search,
            library::library_catalog_status,
            library::queries::library_tracks_page,
            library::queries::library_play_tracks,
            library::queries::library_tracks_by_paths,
            library::youtube::library_youtube_tracks,
            library::queries::library_collections,
            library::queries::library_collection_tracks,
            library::playlists::library_playlists,
            library::library_scan,
            library::library_cancel,
            library::library_resolve_path,
            set_music_roots,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
