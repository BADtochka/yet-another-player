mod binding;
#[cfg(target_os = "macos")]
#[path = "volume_gesture/macos_overlay.rs"]
mod macos_overlay;
#[cfg(target_os = "linux")]
#[path = "volume_gesture/linux.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "volume_gesture/macos.rs"]
mod platform;
#[cfg(target_os = "windows")]
#[path = "volume_gesture/windows.rs"]
mod platform;

use std::{
    sync::{
        atomic::{AtomicBool, AtomicI64, Ordering::Relaxed},
        mpsc, Arc, Mutex,
    },
    thread,
    time::Duration,
};

use serde::Serialize;
use tauri::{AppHandle, Manager, State, WebviewWindow, WindowEvent};
#[cfg(not(target_os = "macos"))]
use tauri::{Emitter, EventTarget, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};
use tokio::{sync::Notify, time::Instant};

use crate::audio::AudioEngine;
use binding::{Binding, Selection};

#[cfg(not(target_os = "macos"))]
const OVERLAY: &str = "volume-overlay";
#[cfg(not(target_os = "macos"))]
const OVERLAY_EVENT: &str = "volume-gesture-level";
const OVERLAY_WIDTH: f64 = 236.0;
const OVERLAY_HEIGHT: f64 = 74.0;
const SHOW_FOR: Duration = Duration::from_millis(1050);

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Status {
    #[default]
    Off,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    Global,
    #[cfg(target_os = "linux")]
    X11,
    #[cfg(target_os = "linux")]
    Wayland,
    Local,
    #[cfg(target_os = "macos")]
    PermissionRequired,
    #[cfg(target_os = "macos")]
    MacInterceptionUnavailable,
}

impl Status {
    fn global(self) -> bool {
        match self {
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            Self::Global => true,
            #[cfg(target_os = "linux")]
            Self::X11 => true,
            _ => false,
        }
    }

    fn shows_overlay(self) -> bool {
        #[cfg(target_os = "linux")]
        if self == Self::Wayland {
            return false;
        }
        self != Self::Off
    }

    fn local(self) -> bool {
        self != Self::Off && !self.global()
    }
}

#[derive(Default)]
struct Pending {
    delta: AtomicI64,
    input: Notify,
}

impl Pending {
    fn add(&self, delta: f64) -> bool {
        if !delta.is_finite() || delta == 0.0 {
            return false;
        }
        let units = (delta.clamp(-0.1, 0.1) * 1_000_000.0).round() as i64;
        self.delta.fetch_add(units, Relaxed);
        self.input.notify_one();
        true
    }
}

#[derive(Clone)]
struct Input {
    stop: Arc<AtomicBool>,
    pending: Arc<Pending>,
}

impl Input {
    fn stopped(&self) -> bool {
        self.stop.load(Relaxed)
    }

    fn scroll(&self, delta: f64) -> bool {
        !self.stopped() && self.pending.add(delta)
    }
}

struct Hook(Arc<AtomicBool>);

impl Drop for Hook {
    fn drop(&mut self) {
        self.0.store(true, Relaxed);
    }
}

#[derive(Default)]
pub(crate) struct VolumeGesture {
    hook: Mutex<Option<Hook>>,
    status: Mutex<Status>,
    pending: Arc<Pending>,
    #[cfg(not(target_os = "macos"))]
    overlay_ready: AtomicBool,
    #[cfg(not(target_os = "macos"))]
    latest: Mutex<Option<(u8, Instant)>>,
}

fn configure(
    state: &VolumeGesture,
    binding: Option<Binding>,
    request: bool,
) -> Result<Status, String> {
    let mut running = state
        .hook
        .lock()
        .map_err(|_| "Глобальный жест недоступен.")?;
    running.take();
    *state
        .status
        .lock()
        .map_err(|_| "Глобальный жест недоступен.")? = Status::Off;
    state.pending.delta.store(0, Relaxed);
    let Some(binding) = binding else {
        return Ok(Status::Off);
    };
    let stop = Arc::new(AtomicBool::new(false));
    let input = Input {
        stop: Arc::clone(&stop),
        pending: Arc::clone(&state.pending),
    };
    let (sender, ready) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("yap-volume-input".into())
        .spawn(move || platform::run(binding, input, request, sender))
        .map_err(|error| {
            eprintln!("Volume gesture: {error}");
            "Не удалось запустить обработчик прокрутки.".to_owned()
        })?;
    let hook = Hook(stop);
    let status = ready
        .recv_timeout(Duration::from_secs(2))
        .unwrap_or_else(|_| {
            #[cfg(target_os = "macos")]
            if request {
                return Status::PermissionRequired;
            }
            Status::Local
        });
    if status.global() {
        *running = Some(hook);
    }
    *state
        .status
        .lock()
        .map_err(|_| "Глобальный жест недоступен.")? = status;
    Ok(status)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod platform {
    use super::*;

    pub(super) fn run(_: Binding, _: Input, _: bool, ready: mpsc::SyncSender<Status>) {
        let _ = ready.send(Status::Local);
    }
}

#[tauri::command]
pub(crate) async fn configure_volume_gesture(
    window: WebviewWindow,
    binding: Option<Selection>,
    request_permission: Option<bool>,
) -> Result<Status, String> {
    if window.label() != "main" {
        return Err("Настройка доступна из главного окна.".into());
    }
    let binding = binding
        .map(Binding::try_from)
        .transpose()
        .map_err(str::to_owned)?;
    let request_permission = request_permission.unwrap_or(false);
    let app = window.app_handle().clone();
    let handle = app.clone();
    let status = tauri::async_runtime::spawn_blocking(move || {
        configure(
            &handle.state::<VolumeGesture>(),
            binding,
            request_permission,
        )
    })
    .await
    .map_err(|_| "Не удалось настроить жест громкости.")??;
    let handle = app.clone();
    app.run_on_main_thread(move || {
        if gesture_status(&handle.state::<VolumeGesture>()).shows_overlay() {
            #[cfg(not(target_os = "macos"))]
            ensure_overlay(&handle);
        } else {
            hide_overlay(&handle);
        }
    })
    .map_err(|_| "Не удалось обновить индикатор громкости.")?;
    #[cfg(target_os = "macos")]
    if request_permission
        && matches!(
            status,
            Status::PermissionRequired | Status::MacInterceptionUnavailable
        )
    {
        // Quartz can return without showing a prompt, including after a prior request.
        crate::external_tools::open_macos_privacy_settings().await?;
    }
    Ok(status)
}

#[tauri::command]
pub(crate) fn volume_gesture_scroll(
    window: WebviewWindow,
    state: State<'_, VolumeGesture>,
    delta: f64,
) {
    if window.label() == "main" && gesture_status(&state).local() {
        state.pending.add(delta);
    }
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub(crate) fn volume_overlay_ready(window: WebviewWindow, state: State<'_, VolumeGesture>) {
    if window.label() != OVERLAY {
        return;
    }
    state.overlay_ready.store(true, Relaxed);
    let latest = state.latest.lock().ok().and_then(|data| *data);
    if let Some((level, _)) = latest.filter(|(_, until)| *until > Instant::now()) {
        present(window.app_handle(), level);
    }
}

fn gesture_status(state: &VolumeGesture) -> Status {
    state
        .status
        .lock()
        .map(|status| *status)
        .unwrap_or_default()
}

#[cfg(not(target_os = "macos"))]
fn ensure_overlay(app: &AppHandle) {
    if app.get_webview_window(OVERLAY).is_some() {
        return;
    }
    match WebviewWindowBuilder::new(app, OVERLAY, WebviewUrl::App("volume-overlay.html".into()))
        .title("Громкость YAP")
        .inner_size(OVERLAY_WIDTH, OVERLAY_HEIGHT)
        .decorations(false)
        .resizable(false)
        .focused(false)
        .focusable(false)
        .skip_taskbar(true)
        .visible_on_all_workspaces(true)
        .always_on_top(true)
        .transparent(true)
        .visible(false)
        .build()
    {
        Ok(window) => {
            if let Err(error) = window.set_ignore_cursor_events(true) {
                eprintln!("Volume overlay click-through: {error}");
            }
        }
        Err(error) => eprintln!("Volume overlay: {error}"),
    }
}

fn present(app: &AppHandle, level: u8) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let state = handle.state::<VolumeGesture>();
        if !gesture_status(&state).shows_overlay() {
            return;
        }
        #[cfg(target_os = "macos")]
        macos_overlay::show(level);
        #[cfg(not(target_os = "macos"))]
        if state.overlay_ready.load(Relaxed) {
            present_web_overlay(&handle, level);
        }
    });
}

#[cfg(not(target_os = "macos"))]
fn present_web_overlay(app: &AppHandle, level: u8) {
    let Some(window) = app.get_webview_window(OVERLAY) else {
        return;
    };
    let visible = window.is_visible().unwrap_or(false);
    if !visible {
        let monitor = window
            .cursor_position()
            .ok()
            .and_then(|cursor| window.monitor_from_point(cursor.x, cursor.y).ok().flatten())
            .or_else(|| window.primary_monitor().ok().flatten());
        if let Some(monitor) = monitor {
            let bounds = monitor.work_area();
            let scale = monitor.scale_factor();
            let x = f64::from(bounds.position.x)
                + (f64::from(bounds.size.width) - OVERLAY_WIDTH * scale) / 2.0;
            let y = f64::from(bounds.position.y) + f64::from(bounds.size.height)
                - (OVERLAY_HEIGHT + 48.0) * scale;
            let _ = window.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
        }
    }
    let _ = app.emit_to(EventTarget::webview_window(OVERLAY), OVERLAY_EVENT, level);
    if !visible {
        let _ = window.show();
    }
}

fn hide_overlay(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.run_on_main_thread(macos_overlay::hide);
    #[cfg(not(target_os = "macos"))]
    if let Some(window) = app.get_webview_window(OVERLAY) {
        let _ = window.hide();
    }
}

pub(crate) fn start(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let app = app.clone();
        window.on_window_event(move |event| {
            if matches!(event, WindowEvent::Destroyed) {
                let _ = configure(&app.state::<VolumeGesture>(), None, false);
                #[cfg(target_os = "macos")]
                macos_overlay::close();
                #[cfg(not(target_os = "macos"))]
                if let Some(overlay) = app.get_webview_window(OVERLAY) {
                    let _ = overlay.close();
                }
            }
        });
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<VolumeGesture>();
        let mut deadline = Instant::now();
        let mut showing = false;
        loop {
            tokio::select! {
                _ = state.pending.input.notified() => {
                    tokio::time::sleep(Duration::from_millis(12)).await;
                    let delta = state.pending.delta.swap(0, Relaxed).clamp(-1_000_000, 1_000_000);
                    if delta == 0 || gesture_status(&state) == Status::Off { continue; }
                    if let Ok(volume) = app.state::<AudioEngine>().adjust_volume(delta as f32 / 1_000_000.0).await {
                        deadline = Instant::now() + SHOW_FOR;
                        showing = true;
                        let level = (volume * 100.0).round() as u8;
                        #[cfg(not(target_os = "macos"))]
                        if let Ok(mut latest) = state.latest.lock() {
                            *latest = Some((level, deadline));
                        }
                        present(&app, level);
                    }
                }
                _ = tokio::time::sleep_until(deadline), if showing => {
                    showing = false;
                    #[cfg(not(target_os = "macos"))]
                    if let Ok(mut latest) = state.latest.lock() {
                        *latest = None;
                    }
                    hide_overlay(&app);
                }
            }
        }
    });
}
