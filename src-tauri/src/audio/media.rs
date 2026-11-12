#[cfg(target_os = "windows")]
mod windows;

use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex, Weak,
    },
    thread,
    time::Duration,
};

use playwire::{Capabilities, Event, MediaControls, PlaybackState, PlayerConfig, Repeat, Track};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};
use tempfile::TempPath;
use tokio::sync::watch;

use super::{library_track_path, Action, Engine, RepeatMode, Request, Snapshot};
use crate::{cover, library::Library};

pub(super) struct Updates {
    sender: watch::Sender<Option<Update>>,
    activated: bool,
}

#[derive(Clone)]
struct Update {
    snapshot: Snapshot,
    selection: u64,
    can_skip: bool,
}

impl Updates {
    pub(super) fn publish(&mut self, engine: &Engine) {
        // A paused queue restored at startup must not take over the media keys.
        self.activated |= engine.is_playing();
        if !self.activated || self.sender.is_closed() {
            return;
        }
        let _ = self.sender.send(Some(Update {
            snapshot: engine.snapshot.clone(),
            selection: engine.selection,
            can_skip: engine.order.len() > 1,
        }));
    }
}

pub(super) fn start(
    app: AppHandle,
    roots: Arc<Mutex<Vec<PathBuf>>>,
    commands: Weak<mpsc::Sender<Request>>,
) -> Option<Updates> {
    let config = PlayerConfig::new("YAP")
        .desktop_entry(
            app.config()
                .product_name
                .as_deref()
                .unwrap_or("yet-another-player"),
        )
        .supported_uri_schemes(Vec::new());
    #[cfg(target_os = "windows")]
    let config = {
        let hwnd = app
            .get_webview_window("main")
            .and_then(|window| window.hwnd().ok());
        let Some(hwnd) = hwnd else {
            eprintln!("System media controls: missing main window handle");
            return None;
        };
        config
            .hwnd(hwnd.0 as usize as u64)
            .app_media_id(&app.config().identifier)
    };

    let (updates, receiver) = watch::channel(None);
    match thread::Builder::new()
        .name("yap-media-controls".into())
        .spawn(move || run(app, config, &roots, commands, receiver))
    {
        Ok(_) => Some(Updates {
            sender: updates,
            activated: false,
        }),
        Err(error) => {
            eprintln!("System media controls: {error}");
            None
        }
    }
}

fn run(
    app: AppHandle,
    config: PlayerConfig,
    roots: &Mutex<Vec<PathBuf>>,
    commands: Weak<mpsc::Sender<Request>>,
    mut updates: watch::Receiver<Option<Update>>,
) {
    let Some(mut update) = receive(&mut updates) else {
        return;
    };
    #[cfg(target_os = "windows")]
    let _runtime = match windows::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("System media controls: {error}");
            return;
        }
    };
    #[cfg(target_os = "windows")]
    let hwnd = config.hwnd;

    let selection = Arc::new(AtomicU64::new(0));
    let callback_selection = Arc::clone(&selection);
    let Some(controls) = on_platform_thread(&app, move || {
        MediaControls::new(config, move |event| {
            if let Some(commands) = commands.upgrade() {
                let selection = callback_selection.load(Ordering::Acquire);
                let _ = commands.send(Request::Media { event, selection });
            }
        })
    }) else {
        return;
    };
    let mut controls = match controls {
        Ok(controls) => controls,
        Err(error) => {
            eprintln!("System media controls: {error}");
            return;
        }
    };
    #[cfg(target_os = "windows")]
    let thumbnail = hwnd.and_then(|hwnd| match windows::Thumbnail::new(hwnd) {
        Ok(thumbnail) => Some(thumbnail),
        Err(error) => {
            eprintln!("System media artwork: {error}");
            None
        }
    });

    let cache = app
        .path()
        .app_cache_dir()
        .ok()
        .map(|path| path.join("covers"));
    let library = app.state::<Library>();
    let mut loaded = None;
    let mut last_path = None;
    let mut last_state: Option<PlaybackState> = None;
    let mut publish_failed = false;
    loop {
        let path_changed = last_path != update.snapshot.path;
        let next_track = path_changed.then(|| {
            let path = library_track_path(update.snapshot.path.as_deref()?, roots).ok()?;
            Some(LoadedTrack::new(&path, &library, cache.as_deref()))
        });
        let track = next_track.as_ref().unwrap_or(&loaded);
        let state = update.playback_state(track.as_ref());
        if last_state.as_ref() != Some(&state) || publish_failed {
            #[cfg(target_os = "windows")]
            let thumbnail_changed = last_state.as_ref().is_none_or(|previous| {
                previous.track != state.track || previous.duration != state.duration
            });
            let Some((next_controls, state, result)) = on_platform_thread(&app, move || {
                let result = controls.set_state(&state);
                (controls, state, result)
            }) else {
                return;
            };
            controls = next_controls;
            if let Err(error) = &result {
                if !publish_failed {
                    eprintln!("System media controls: {error}");
                }
            }
            #[cfg(target_os = "windows")]
            if (thumbnail_changed || publish_failed) && result.is_ok() {
                if let (Some(thumbnail), Some(artwork)) = (
                    &thumbnail,
                    track
                        .as_ref()
                        .filter(|_| {
                            update
                                .snapshot
                                .preview
                                .as_ref()
                                .is_none_or(|preview| preview.loading)
                        })
                        .and_then(|track| track.artwork.as_ref()),
                ) {
                    if let Err(error) = thumbnail.publish(artwork) {
                        eprintln!("System media artwork: {error}");
                    }
                }
            }
            publish_failed = result.is_err();
            last_state = Some(state);
        }
        if let Some(next_track) = next_track {
            loaded = next_track;
            last_path = update.snapshot.path;
        }
        selection.store(update.selection, Ordering::Release);

        let Some(next) = receive(&mut updates) else {
            break;
        };
        update = next;
    }
    on_platform_thread(&app, move || drop(controls));
}

fn receive(updates: &mut watch::Receiver<Option<Update>>) -> Option<Update> {
    tauri::async_runtime::block_on(updates.changed()).ok()?;
    updates.borrow_and_update().clone()
}

#[cfg(target_os = "macos")]
fn on_platform_thread<T: Send + 'static>(
    app: &AppHandle,
    task: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (reply, result) = mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = reply.send(task());
    })
    .ok()?;
    result.recv().ok()
}

#[cfg(not(target_os = "macos"))]
fn on_platform_thread<T>(_app: &AppHandle, task: impl FnOnce() -> T) -> Option<T> {
    Some(task())
}

struct LoadedTrack {
    metadata: Track,
    duration: Option<f64>,
    // Keep the file available while the OS displays the track.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    artwork: Option<TempPath>,
}

impl LoadedTrack {
    fn new(path: &Path, library: &Library, cache: Option<&Path>) -> Self {
        let tags = library.playback_tags(path);
        let artwork = artwork_file(path, cache.unwrap_or_else(|| Path::new("")));
        let metadata = Track {
            id: format!("{:x}", Sha256::digest(path.as_os_str().as_encoded_bytes())),
            title: tags.title,
            artists: tags.artists,
            album: tags.album.unwrap_or_default(),
            #[cfg(not(target_os = "windows"))]
            artwork_url: artwork
                .as_ref()
                .and_then(|file| tauri::Url::from_file_path(&**file).ok())
                .map(String::from)
                .unwrap_or_default(),
            ..Track::default()
        };
        Self {
            metadata,
            duration: tags.duration,
            artwork,
        }
    }
}

fn artwork_file(path: &Path, cache: &Path) -> Option<TempPath> {
    let (bytes, mime) = cover::load_cover(path, cache)?;
    let suffix = match mime {
        "image/jpeg" => ".jpg",
        "image/png" => ".png",
        "image/webp" => ".webp",
        _ => return None,
    };
    let mut file = tempfile::Builder::new()
        .prefix("yap-now-playing-")
        .suffix(suffix)
        .tempfile()
        .ok()?;
    file.write_all(&bytes).ok()?;
    Some(file.into_temp_path())
}

impl Update {
    fn playback_state(&self, track: Option<&LoadedTrack>) -> PlaybackState {
        let snapshot = &self.snapshot;
        if let Some(preview) = snapshot.preview.as_ref().filter(|preview| !preview.loading) {
            let duration = preview
                .duration
                .filter(|seconds| *seconds > 0.0)
                .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok());
            return PlaybackState {
                track: Some(Track {
                    id: format!("preview_{}", preview.id),
                    title: preview.title.clone(),
                    artists: vec![preview.artist.clone()],
                    ..Track::default()
                }),
                playing: preview.playing,
                position: Duration::try_from_secs_f64(preview.position).unwrap_or_default(),
                duration,
                volume: f64::from(snapshot.volume),
                capabilities: Capabilities {
                    can_go_next: false,
                    can_go_previous: false,
                    can_seek: duration.is_some(),
                },
                ..PlaybackState::default()
            };
        }
        let duration = snapshot
            .duration
            .or_else(|| track.and_then(|track| track.duration))
            .filter(|seconds| *seconds > 0.0)
            .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok());
        PlaybackState {
            track: track.map(|track| track.metadata.clone()),
            playing: snapshot.playing,
            position: Duration::try_from_secs_f64(snapshot.position).unwrap_or_default(),
            duration,
            volume: f64::from(snapshot.volume),
            repeat: match snapshot.repeat {
                RepeatMode::Off => Repeat::Off,
                RepeatMode::One => Repeat::One,
                RepeatMode::All => Repeat::All,
            },
            shuffle: snapshot.shuffle,
            capabilities: Capabilities {
                can_go_next: self.can_skip,
                can_go_previous: self.can_skip,
                can_seek: duration.is_some(),
            },
        }
    }
}

impl Engine {
    pub(super) fn apply_media(&mut self, app: &AppHandle, event: Event, selection: u64) {
        if selection != self.selection
            && matches!(
                event,
                Event::Play
                    | Event::Pause
                    | Event::PlayPause
                    | Event::Stop
                    | Event::SeekTo(_)
                    | Event::SeekBy(_)
                    | Event::Next
                    | Event::Previous
            )
        {
            return;
        }
        if let Some(preview) = self.active_preview() {
            match event {
                Event::Play => return self.set_preview_playing(true),
                Event::Pause => return self.set_preview_playing(false),
                Event::PlayPause => return self.set_preview_playing(!preview.playing),
                Event::SeekTo(position) => return self.seek_preview(position.as_secs_f64()),
                Event::SeekBy(offset) => return self.seek_preview(preview.position + offset),
                Event::Stop => return self.close_preview(false),
                Event::Next | Event::Previous | Event::SetRepeat(_) | Event::SetShuffle(_) => {
                    return
                }
                _ => {}
            }
        }
        match event {
            Event::Stop => {
                self.apply(Action::Playing { playing: false });
                if let Some(action) = self.media_seek(0.0, selection) {
                    self.apply(action);
                }
            }
            Event::SetRepeat(mode) => {
                self.snapshot.repeat = match mode {
                    Repeat::Off => RepeatMode::Off,
                    Repeat::One => RepeatMode::One,
                    Repeat::All => RepeatMode::All,
                };
            }
            Event::Raise | Event::Quit => {
                let app_handle = app.clone();
                let _ = app.run_on_main_thread(move || match event {
                    Event::Raise => {
                        if let Some(window) = app_handle.get_webview_window("main") {
                            let _ = window.unminimize();
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    Event::Quit => {
                        if let Some(window) = app_handle.get_webview_window("main") {
                            let _ = window.close();
                        } else {
                            app_handle.exit(0);
                        }
                    }
                    _ => {}
                });
            }
            _ => {
                if let Some(action) = self.media_action(event, selection) {
                    self.apply(action);
                }
            }
        }
    }

    fn media_action(&self, event: Event, selection: u64) -> Option<Action> {
        match event {
            Event::Play => Some(Action::Playing { playing: true }),
            Event::Pause => Some(Action::Playing { playing: false }),
            Event::PlayPause => Some(Action::Playing {
                playing: !self.snapshot.playing,
            }),
            Event::Next if self.order.len() > 1 => Some(Action::Next),
            Event::Previous if self.order.len() > 1 => Some(Action::Previous),
            Event::SeekTo(position) => self.media_seek(position.as_secs_f64(), selection),
            Event::SeekBy(offset) => self.media_seek(self.snapshot.position + offset, selection),
            Event::SetVolume(volume) if volume.is_finite() => Some(Action::Volume {
                volume: volume.clamp(0.0, 1.0) as f32,
            }),
            Event::SetShuffle(shuffle) if self.snapshot.shuffle != shuffle => {
                Some(Action::ToggleShuffle)
            }
            _ => None,
        }
    }

    fn media_seek(&self, position: f64, selection: u64) -> Option<Action> {
        if selection != self.selection || !position.is_finite() {
            return None;
        }
        Some(Action::Seek {
            position: position.max(0.0),
            path: self.snapshot.path.clone()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_seek_uses_the_published_track_and_clamps_negative_offsets() {
        let mut engine = Engine::new(Arc::default());
        engine.selection = 2;
        engine.snapshot.path = Some("track.mp3".into());
        engine.snapshot.position = 4.0;

        assert!(engine.media_action(Event::SeekBy(3.0), 1).is_none());
        assert!(engine.media_action(Event::SeekBy(f64::NAN), 2).is_none());
        assert!(matches!(
            engine.media_action(Event::SeekBy(-10.0), 2),
            Some(Action::Seek { position, path }) if position == 0.0 && path == "track.mp3"
        ));
    }

    #[test]
    fn media_controls_activate_on_play_and_keep_receiving_paused_state() {
        let (sender, receiver) = watch::channel(None);
        let mut updates = Updates {
            sender,
            activated: false,
        };
        let mut engine = Engine::new(Arc::default());
        engine.snapshot.path = Some("restored.mp3".into());

        updates.publish(&engine);
        assert!(receiver.borrow().is_none());
        engine.snapshot.playing = true;
        updates.publish(&engine);
        engine.snapshot.playing = false;
        updates.publish(&engine);
        let update = receiver.borrow();
        assert!(!update.as_ref().unwrap().snapshot.playing);
    }
}
