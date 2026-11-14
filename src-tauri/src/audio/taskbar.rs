use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{mpsc, Weak},
};

use tauri::{AppHandle, Manager};
use windows::{
    core::{w, Error, Result},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER},
        UI::{
            Shell::{
                DefSubclassProc, ITaskbarList3, RemoveWindowSubclass, SetWindowSubclass,
                TaskbarList, THBF_DISABLED, THBF_ENABLED, THBN_CLICKED, THB_FLAGS, THB_ICON,
                THB_TOOLTIP, THUMBBUTTON,
            },
            WindowsAndMessaging::{
                ChangeWindowMessageFilterEx, CreateIconFromResourceEx, DestroyIcon,
                RegisterWindowMessageW, HICON, IMAGE_FLAGS, MSGFLT_ALLOW, WM_COMMAND, WM_NCDESTROY,
            },
        },
    },
};

use super::{Action, Engine, RepeatMode, Request};

const SUBCLASS: usize = 1;
const FIRST_BUTTON: usize = 200;

#[derive(Clone, Copy)]
pub(super) enum Command {
    Shuffle,
    Previous,
    PlayPause,
    Next,
    Repeat,
}

const COMMANDS: [Command; 5] = [
    Command::Shuffle,
    Command::Previous,
    Command::PlayPause,
    Command::Next,
    Command::Repeat,
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct State {
    loaded: bool,
    preview: bool,
    playing: bool,
    shuffle: bool,
    repeat: RepeatMode,
}

impl Default for State {
    fn default() -> Self {
        Self {
            loaded: false,
            preview: false,
            playing: false,
            shuffle: false,
            repeat: RepeatMode::Off,
        }
    }
}

impl State {
    pub(super) fn from_engine(engine: &Engine) -> Self {
        let preview = engine.active_preview();
        Self {
            loaded: engine.snapshot.path.is_some() || preview.is_some(),
            preview: preview.is_some(),
            playing: preview.map_or(engine.snapshot.playing, |preview| preview.playing),
            shuffle: engine.snapshot.shuffle,
            repeat: engine.snapshot.repeat,
        }
    }
}

impl Engine {
    pub(super) fn apply_taskbar(&mut self, command: Command) {
        if let Some(preview) = self.active_preview() {
            if matches!(command, Command::PlayPause) {
                self.set_preview_playing(!preview.playing);
            }
            return;
        }
        let action = match command {
            Command::Shuffle => Action::ToggleShuffle,
            Command::Previous => Action::Previous,
            Command::PlayPause => Action::Playing {
                playing: !self.snapshot.playing,
            },
            Command::Next => Action::Next,
            Command::Repeat => Action::CycleRepeat,
        };
        self.apply(action);
    }
}

thread_local! {
    static TOOLBAR: RefCell<Option<Rc<Toolbar>>> = const { RefCell::new(None) };
}

struct Toolbar {
    hwnd: HWND,
    ready_message: u32,
    commands: Weak<mpsc::Sender<Request>>,
    state: Cell<State>,
    installed: Cell<bool>,
}

pub(super) fn install(app: &AppHandle, commands: Weak<mpsc::Sender<Request>>) {
    let Some(hwnd) = app
        .get_webview_window("main")
        .and_then(|window| window.hwnd().ok())
    else {
        return;
    };
    let hwnd = HWND(hwnd.0);
    // Tauri setup and the subclass callback both run on the HWND owner thread.
    let ready_message = unsafe { RegisterWindowMessageW(w!("TaskbarButtonCreated")) };
    if ready_message == 0
        || !unsafe { SetWindowSubclass(hwnd, Some(window_message), SUBCLASS, 0) }.as_bool()
    {
        eprintln!("Taskbar toolbar: {}", Error::from_thread());
        return;
    }
    let _ = unsafe { ChangeWindowMessageFilterEx(hwnd, ready_message, MSGFLT_ALLOW, None) };
    let toolbar = Rc::new(Toolbar {
        hwnd,
        ready_message,
        commands,
        state: Cell::default(),
        installed: Cell::new(false),
    });
    TOOLBAR.with(|slot| slot.replace(Some(Rc::clone(&toolbar))));
    // The window may already have received TaskbarButtonCreated during creation.
    let _ = toolbar.refresh();
}

pub(super) fn publish(app: &AppHandle, state: State) {
    let _ = app.run_on_main_thread(move || {
        let Some(toolbar) = TOOLBAR.with(|slot| slot.borrow().clone()) else {
            return;
        };
        toolbar.state.set(state);
        if toolbar.installed.get() {
            if let Err(error) = toolbar.refresh() {
                eprintln!("Taskbar toolbar: {error}");
            }
        }
    });
}

unsafe extern "system" fn window_message(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if message == WM_NCDESTROY {
        let _ = unsafe { RemoveWindowSubclass(hwnd, Some(window_message), SUBCLASS) };
        TOOLBAR.with(|slot| slot.replace(None));
    } else if let Some(toolbar) = TOOLBAR.with(|slot| slot.borrow().clone()) {
        if message == toolbar.ready_message {
            // Explorer sends this again after rebuilding the taskbar.
            if let Err(error) = toolbar.refresh() {
                eprintln!("Taskbar toolbar: {error}");
            }
        } else if message == WM_COMMAND
            && lparam.0 == 0
            && ((wparam.0 >> 16) as u16) == THBN_CLICKED as u16
        {
            if let Some(command) = (wparam.0 & 0xffff)
                .checked_sub(FIRST_BUTTON)
                .and_then(|index| COMMANDS.get(index))
            {
                if let Some(sender) = toolbar.commands.upgrade() {
                    let _ = sender.send(Request::Taskbar(*command));
                }
                return LRESULT(0);
            }
        }
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

impl Toolbar {
    fn refresh(&self) -> Result<()> {
        let taskbar: ITaskbarList3 =
            unsafe { CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER)? };
        unsafe { taskbar.HrInit()? };
        let state = self.state.get();
        let repeat = match state.repeat {
            RepeatMode::Off => "Повтор выключен",
            RepeatMode::All => "Повтор очереди",
            RepeatMode::One => "Повтор трека",
        };
        let play = if state.playing {
            (PAUSE, "Пауза")
        } else {
            (PLAY, "Воспроизвести")
        };
        let controls = [
            (
                SHUFFLE,
                if state.shuffle {
                    "Перемешивание включено"
                } else {
                    "Перемешивание выключено"
                },
                state.shuffle,
                !state.preview,
            ),
            (
                PREVIOUS,
                "Предыдущий трек",
                false,
                state.loaded && !state.preview,
            ),
            (play.0, play.1, false, state.loaded),
            (
                NEXT,
                "Следующий трек",
                false,
                state.loaded && !state.preview,
            ),
            (
                if state.repeat == RepeatMode::One {
                    REPEAT_ONE
                } else {
                    REPEAT
                },
                repeat,
                state.repeat != RepeatMode::Off,
                !state.preview,
            ),
        ];
        let icons = controls
            .iter()
            .map(|(shape, _, active, _)| Icon::new(*shape, *active))
            .collect::<Result<Vec<_>>>()?;
        let buttons = controls
            .iter()
            .zip(&icons)
            .enumerate()
            .map(|(index, ((_, label, _, enabled), icon))| {
                let mut button = THUMBBUTTON {
                    dwMask: THB_ICON | THB_TOOLTIP | THB_FLAGS,
                    iId: (FIRST_BUTTON + index) as u32,
                    hIcon: icon.0,
                    dwFlags: if *enabled {
                        THBF_ENABLED
                    } else {
                        THBF_DISABLED
                    },
                    ..THUMBBUTTON::default()
                };
                for (slot, unit) in button.szTip.iter_mut().zip(label.encode_utf16()) {
                    *slot = unit;
                }
                button
            })
            .collect::<Vec<_>>();
        // Add may fail when the toolbar already exists, including duplicate readiness notifications.
        if !self.installed.get()
            || unsafe { taskbar.ThumbBarUpdateButtons(self.hwnd, &buttons) }.is_err()
        {
            unsafe {
                taskbar
                    .ThumbBarAddButtons(self.hwnd, &buttons)
                    .or_else(|_| taskbar.ThumbBarUpdateButtons(self.hwnd, &buttons))?
            };
        }
        self.installed.set(true);
        Ok(())
    }
}

struct Icon(HICON);

impl Icon {
    fn new(shape: &[u16; 16], active: bool) -> Result<Self> {
        const SIDE: usize = 16;
        let mut resource = Vec::with_capacity(40 + SIDE * SIDE * 4 + SIDE * 4);
        // A 32-bit icon resource stores a bottom-up BGRA bitmap and a 1-bit transparency mask.
        for value in [40_u32, SIDE as u32, 2 * SIDE as u32] {
            resource.extend(value.to_le_bytes());
        }
        resource.extend(1_u16.to_le_bytes());
        resource.extend(32_u16.to_le_bytes());
        resource.resize(40, 0);
        let color = if active {
            [244, 153, 48, 255]
        } else {
            [140, 140, 140, 255]
        };
        for row in shape.iter().rev() {
            for x in 0..SIDE {
                resource.extend(if row & (0x8000 >> x) != 0 {
                    color
                } else {
                    [0; 4]
                });
            }
        }
        for row in shape.iter().rev() {
            resource.extend((!row).to_be_bytes());
            resource.extend([0; 2]);
        }
        unsafe {
            CreateIconFromResourceEx(&resource, true, 0x0003_0000, 0, 0, IMAGE_FLAGS(0)).map(Self)
        }
    }
}

impl Drop for Icon {
    fn drop(&mut self) {
        let _ = unsafe { DestroyIcon(self.0) };
    }
}

const PLAY: &[u16; 16] = &[
    0, 0, 0x1000, 0x1800, 0x1c00, 0x1e00, 0x1f00, 0x1f80, 0x1f80, 0x1f00, 0x1e00, 0x1c00, 0x1800,
    0x1000, 0, 0,
];
const PAUSE: &[u16; 16] = &[
    0, 0, 0, 0x18c0, 0x18c0, 0x18c0, 0x18c0, 0x18c0, 0x18c0, 0x18c0, 0x18c0, 0x18c0, 0x18c0, 0, 0,
    0,
];
const PREVIOUS: &[u16; 16] = &[
    0, 0, 0, 0x3030, 0x3070, 0x30f0, 0x31f0, 0x33f0, 0x33f0, 0x31f0, 0x30f0, 0x3070, 0x3030, 0, 0,
    0,
];
const NEXT: &[u16; 16] = &[
    0, 0, 0, 0x0c0c, 0x0e0c, 0x0f0c, 0x0f8c, 0x0fcc, 0x0fcc, 0x0f8c, 0x0f0c, 0x0e0c, 0x0c0c, 0, 0,
    0,
];
const SHUFFLE: &[u16; 16] = &[
    0, 0, 0x0020, 0x002c, 0x701e, 0x381f, 0x1c3e, 0x0e2c, 0x0720, 0x1c80, 0x3820, 0x703e, 0x003f,
    0x002e, 0x0020, 0,
];
const REPEAT: &[u16; 16] = &[
    0, 0, 0x0010, 0x0018, 0x3ffc, 0x3ffe, 0x301c, 0x3018, 0x000c, 0x180c, 0x380c, 0x7ffc, 0x3ffc,
    0x1800, 0x0800, 0,
];
const REPEAT_ONE: &[u16; 16] = &[
    0, 0, 0x0010, 0x0018, 0x3ffc, 0x3ffe, 0x319c, 0x3098, 0x008c, 0x188c, 0x39cc, 0x7ffc, 0x3ffc,
    0x1800, 0x0800, 0,
];
