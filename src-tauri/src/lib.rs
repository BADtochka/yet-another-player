use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tauri::Manager;
use tauri::Url;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

mod cover;
mod linux_audio;

struct RangeFile {
    file: File,
    remaining: u64,
}

impl Read for RangeFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.remaining == 0 {
            return Ok(0);
        }

        let to_read = std::cmp::min(buf.len() as u64, self.remaining) as usize;
        let read = self.file.read(&mut buf[..to_read])?;
        self.remaining -= read as u64;
        Ok(read)
    }
}

struct AudioServerState {
    base_url: String,
    roots: Arc<Mutex<Vec<PathBuf>>>,
    cover_cache: Arc<Mutex<PathBuf>>,
    resolved_paths: Arc<Mutex<HashMap<String, PathBuf>>>,
}

#[tauri::command]
fn audio_server_base(state: tauri::State<AudioServerState>) -> String {
    state.base_url.clone()
}

#[tauri::command]
fn get_cover_src(state: tauri::State<AudioServerState>, path: String) -> Option<String> {
    let resolved = resolve_track_path(&path, &state)?;
    let cover_cache = state.cover_cache.lock().ok()?;

    if cover::load_cover(&resolved, &cover_cache).is_none() {
        return None;
    }

    cover_url_for_path(&state.base_url, &path)
}

#[tauri::command]
fn get_audio_environment(state: tauri::State<linux_audio::AudioEnvironment>) -> linux_audio::AudioEnvironment {
    state.inner().clone()
}

#[tauri::command]
fn set_music_roots(state: tauri::State<AudioServerState>, paths: Vec<String>) -> Result<(), String> {
    let mut roots = Vec::new();

    for path in paths {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            continue;
        }

        let canonical =
            std::fs::canonicalize(PathBuf::from(trimmed)).map_err(|error| error.to_string())?;

        if !roots.iter().any(|existing| existing == &canonical) {
            roots.push(canonical);
        }
    }

    *state.roots.lock().map_err(|error| error.to_string())? = roots;
    state
        .resolved_paths
        .lock()
        .map_err(|error| error.to_string())?
        .clear();

    Ok(())
}

fn parse_range(header: &str, total: u64) -> Option<(u64, u64)> {
    if total == 0 {
        return None;
    }

    let spec = header.trim().strip_prefix("bytes=")?;
    let (raw_start, raw_end) = spec.split_once('-')?;

    let (start, end) = if raw_start.is_empty() {
        let suffix: u64 = raw_end.parse().ok()?;
        if suffix == 0 {
            return None;
        }
        (total.saturating_sub(suffix), total - 1)
    } else {
        let start: u64 = raw_start.parse().ok()?;
        let end = if raw_end.is_empty() {
            total - 1
        } else {
            raw_end.parse::<u64>().ok()?.min(total - 1)
        };
        (start, end)
    };

    if start > end || start >= total {
        return None;
    }

    Some((start, end))
}

fn audio_content_type(path: &Path) -> Option<&'static str> {
    let extension = path.extension().and_then(|extension| extension.to_str())?;

    if extension.eq_ignore_ascii_case("mp3") {
        return Some("audio/mpeg");
    }

    if extension.eq_ignore_ascii_case("m4a") {
        return Some("audio/mp4");
    }

    if extension.eq_ignore_ascii_case("webm") {
        return Some("audio/webm");
    }

    if extension.eq_ignore_ascii_case("ogg") {
        return Some("audio/ogg");
    }

    None
}

fn resolve_track_path(raw: &str, state: &AudioServerState) -> Option<PathBuf> {
    if let Ok(cache) = state.resolved_paths.lock() {
        if let Some(path) = cache.get(raw) {
            return Some(path.clone());
        }
    }

    let canonical = std::fs::canonicalize(PathBuf::from(raw)).ok()?;
    if audio_content_type(&canonical).is_none() {
        return None;
    }

    let roots = state.roots.lock().ok()?;
    if roots.is_empty() {
        return None;
    }

    if !roots
        .iter()
        .any(|root| canonical.starts_with(root))
    {
        return None;
    }

    if let Ok(mut cache) = state.resolved_paths.lock() {
        cache.insert(raw.to_string(), canonical.clone());
    }

    Some(canonical)
}

fn resolve_path(url: &str, state: &AudioServerState) -> Option<PathBuf> {
    let parsed = Url::parse(&format!("http://localhost{url}")).ok()?;
    let raw = parsed
        .query_pairs()
        .find(|(key, _)| key.as_ref() == "path")
        .map(|(_, value)| value.into_owned())?;

    resolve_track_path(&raw, state)
}

fn cover_url_for_path(base_url: &str, path: &str) -> Option<String> {
    let mut url = Url::parse(&format!("{base_url}/cover")).ok()?;
    url.query_pairs_mut().append_pair("path", path);
    Some(url.to_string())
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("valid header")
}

fn cors_headers() -> [Header; 4] {
    [
        header("Access-Control-Allow-Origin", "*"),
        header("Access-Control-Allow-Methods", "GET, HEAD, OPTIONS"),
        header("Access-Control-Allow-Headers", "Range, Content-Type"),
        header(
            "Access-Control-Expose-Headers",
            "Accept-Ranges, Content-Length, Content-Range",
        ),
    ]
}

fn with_cors<R: Read + Send + 'static>(response: Response<R>) -> Response<R> {
    let mut response = response;
    for cors_header in cors_headers() {
        response = response.with_header(cors_header);
    }
    response
}

fn respond_with_cors<R: Read + Send + 'static>(request: Request, response: Response<R>) {
    let _ = request.respond(with_cors(response));
}

fn handle_cover_request(request: Request, state: &AudioServerState) {
    if request.method() == &Method::Options {
        respond_with_cors(request, Response::empty(204));
        return;
    }

    let Some(path) = resolve_path(request.url(), state) else {
        respond_with_cors(
            request,
            Response::from_string("forbidden").with_status_code(403),
        );
        return;
    };

    let cover_cache = match state.cover_cache.lock() {
        Ok(cache) => cache.clone(),
        Err(_) => {
            respond_with_cors(
                request,
                Response::from_string("error").with_status_code(500),
            );
            return;
        }
    };

    let Some((image, content_type)) = cover::load_cover(&path, &cover_cache) else {
        respond_with_cors(
            request,
            Response::from_string("not found").with_status_code(404),
        );
        return;
    };

    let is_head = request.method() == &Method::Head;

    if is_head {
        let response = Response::empty(200)
            .with_header(header("Content-Type", content_type))
            .with_header(header("Cache-Control", "public, max-age=31536000, immutable"));
        respond_with_cors(request, response);
        return;
    }

    let response = Response::from_data(image)
        .with_header(header("Content-Type", content_type))
        .with_header(header("Cache-Control", "public, max-age=31536000, immutable"));
    respond_with_cors(request, response);
}

fn handle_stream_request(request: Request, state: &AudioServerState) {
    if request.method() == &Method::Options {
        respond_with_cors(request, Response::empty(204));
        return;
    }

    let Some(path) = resolve_path(request.url(), state) else {
        respond_with_cors(
            request,
            Response::from_string("forbidden").with_status_code(403),
        );
        return;
    };

    let mut file = match File::open(&path) {
        Ok(file) => file,
        Err(_) => {
            respond_with_cors(
                request,
                Response::from_string("not found").with_status_code(404),
            );
            return;
        }
    };

    let total = match file.metadata() {
        Ok(metadata) => metadata.len(),
        Err(_) => {
            respond_with_cors(
                request,
                Response::from_string("error").with_status_code(500),
            );
            return;
        }
    };

    let range = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Range"))
        .map(|header| header.value.as_str().to_string());

    if let Some((start, end)) = range.and_then(|range| parse_range(&range, total)) {
        let length = end - start + 1;

        if file.seek(SeekFrom::Start(start)).is_err() {
            respond_with_cors(
                request,
                Response::from_string("error").with_status_code(500),
            );
            return;
        }

        let response = Response::new(
            StatusCode(206),
            vec![
                header("Content-Type", audio_content_type(&path).unwrap_or("application/octet-stream")),
                header("Accept-Ranges", "bytes"),
                header("Content-Range", &format!("bytes {start}-{end}/{total}")),
            ],
            RangeFile {
                file,
                remaining: length,
            },
            Some(length as usize),
            None,
        );
        respond_with_cors(request, response);
    } else {
        let response = Response::from_file(file)
            .with_header(header("Content-Type", audio_content_type(&path).unwrap_or("application/octet-stream")))
            .with_header(header("Accept-Ranges", "bytes"));
        respond_with_cors(request, response);
    }
}

fn handle_warmup_request(request: Request) {
    if request.method() == &Method::Options {
        respond_with_cors(request, Response::empty(204));
        return;
    }

    respond_with_cors(request, Response::empty(204));
}

fn handle_request(request: Request, state: &AudioServerState) {
    let url = request.url();

    if url.starts_with("/warmup") {
        handle_warmup_request(request);
        return;
    }

    if url.starts_with("/cover") {
        handle_cover_request(request, state);
        return;
    }

    handle_stream_request(request, state);
}

fn start_audio_server(state: Arc<AudioServerState>, server: Arc<Server>) {
    for _ in 0..8 {
        let server = Arc::clone(&server);
        let state = Arc::clone(&state);
        std::thread::spawn(move || loop {
            match server.recv() {
                Ok(request) => handle_request(request, &state),
                Err(_) => break,
            }
        });
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let audio_environment = linux_audio::configure();

    let roots: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
    let cover_cache = Arc::new(Mutex::new(PathBuf::new()));
    let resolved_paths = Arc::new(Mutex::new(HashMap::new()));

    let server = Server::http("127.0.0.1:0")
        .map_err(|error| std::io::Error::other(error.to_string()))
        .expect("failed to start audio server");
    let port = server
        .server_addr()
        .to_ip()
        .map(|addr| addr.port())
        .expect("failed to resolve audio server port");
    let base_url = format!("http://127.0.0.1:{port}");

    let server = Arc::new(server);
    let server_state = Arc::new(AudioServerState {
        base_url: base_url.clone(),
        roots: Arc::clone(&roots),
        cover_cache: Arc::clone(&cover_cache),
        resolved_paths: Arc::clone(&resolved_paths),
    });

    let server_for_setup = Arc::clone(&server);
    let server_state_for_setup = Arc::clone(&server_state);

    tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_shell::init())
        .manage(audio_environment)
        .manage(AudioServerState {
            base_url,
            roots,
            cover_cache,
            resolved_paths,
        })
        .plugin(tauri_plugin_zustand::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            let cache_dir = app
                .path()
                .app_cache_dir()
                .map_err(|error| error.to_string())?
                .join("covers");

            std::fs::create_dir_all(&cache_dir).map_err(|error| error.to_string())?;

            if let Some(state) = app.try_state::<AudioServerState>() {
                *state
                    .cover_cache
                    .lock()
                    .map_err(|error| error.to_string())? = cache_dir;
            }

            start_audio_server(server_state_for_setup, server_for_setup);

            #[cfg(debug_assertions)]
            if let Some(window) = app.get_webview_window("main") {
                window.open_devtools();
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![audio_server_base, get_audio_environment, get_cover_src, set_music_roots])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
