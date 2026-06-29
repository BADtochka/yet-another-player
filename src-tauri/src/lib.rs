use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tauri::Manager;
use tauri::Url;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

mod wsl_audio;

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
    root: Arc<Mutex<Option<PathBuf>>>,
    resolved_paths: Arc<Mutex<HashMap<String, PathBuf>>>,
}

#[tauri::command]
fn audio_server_base(state: tauri::State<AudioServerState>) -> String {
    state.base_url.clone()
}

#[tauri::command]
fn set_music_root(state: tauri::State<AudioServerState>, path: String) -> Result<(), String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Ok(());
    }

    let canonical =
        std::fs::canonicalize(PathBuf::from(trimmed)).map_err(|error| error.to_string())?;

    *state.root.lock().map_err(|error| error.to_string())? = Some(canonical);
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

fn is_mp3(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp3"))
}

fn resolve_path(url: &str, state: &AudioServerState) -> Option<PathBuf> {
    let parsed = Url::parse(&format!("http://localhost{url}")).ok()?;
    let raw = parsed
        .query_pairs()
        .find(|(key, _)| key.as_ref() == "path")
        .map(|(_, value)| value.into_owned())?;

    if let Ok(cache) = state.resolved_paths.lock() {
        if let Some(path) = cache.get(&raw) {
            return Some(path.clone());
        }
    }

    let canonical = std::fs::canonicalize(PathBuf::from(&raw)).ok()?;
    if !is_mp3(&canonical) {
        return None;
    }

    let root = state.root.lock().ok()?.clone()?;
    if !canonical.starts_with(&root) {
        return None;
    }

    if let Ok(mut cache) = state.resolved_paths.lock() {
        cache.insert(raw, canonical.clone());
    }

    Some(canonical)
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

fn handle_request(request: Request, state: &AudioServerState) {
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
                header("Content-Type", "audio/mpeg"),
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
            .with_header(header("Content-Type", "audio/mpeg"))
            .with_header(header("Accept-Ranges", "bytes"));
        respond_with_cors(request, response);
    }
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
    wsl_audio::configure();

    let root: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
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

    let server_state = Arc::new(AudioServerState {
        base_url: base_url.clone(),
        root: Arc::clone(&root),
        resolved_paths: Arc::clone(&resolved_paths),
    });

    start_audio_server(Arc::clone(&server_state), Arc::new(server));

    tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AudioServerState {
            base_url,
            root,
            resolved_paths,
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            #[cfg(debug_assertions)]
            if let Some(window) = app.get_webview_window("main") {
                window.open_devtools();
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![audio_server_base, set_music_root])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
