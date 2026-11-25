use std::{cell::RefCell, ptr, sync::mpsc::SyncSender};

use windows_sys::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, GetKeyboardLayout, MapVirtualKeyExW, MAPVK_VSC_TO_VK_EX, VK_CONTROL,
            VK_LWIN, VK_MBUTTON, VK_MENU, VK_RWIN, VK_SHIFT, VK_XBUTTON1, VK_XBUTTON2,
        },
        WindowsAndMessaging::{
            CallNextHookEx, DispatchMessageW, GetForegroundWindow, GetWindowThreadProcessId,
            MsgWaitForMultipleObjects, PeekMessageW, SetWindowsHookExW, TranslateMessage,
            UnhookWindowsHookEx, HC_ACTION, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, PM_REMOVE,
            QS_ALLINPUT, WH_MOUSE_LL, WM_MOUSEWHEEL, WM_QUIT,
        },
    },
};

use super::{binding::*, Input, Status};

thread_local! {
    static HOOK: RefCell<Option<(Binding, Input)>> = const { RefCell::new(None) };
}

pub(super) fn run(binding: Binding, input: Input, _: bool, ready: SyncSender<Status>) {
    HOOK.with_borrow_mut(|hook| *hook = Some((binding, input.clone())));
    unsafe {
        let module = GetModuleHandleW(ptr::null());
        let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(scroll), module, 0);
        if hook.is_null() {
            let _ = ready.send(Status::Local);
            return;
        }
        let _ = ready.send(Status::Global);
        let mut message = MSG::default();
        'running: while !input.stopped() {
            MsgWaitForMultipleObjects(0, ptr::null(), 0, 100, QS_ALLINPUT);
            while PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                if message.message == WM_QUIT {
                    break 'running;
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        UnhookWindowsHookEx(hook);
    }
}

unsafe extern "system" fn scroll(code: i32, message: WPARAM, pointer: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && message == WM_MOUSEWHEEL as WPARAM && pointer != 0 {
        let event = unsafe { &*(pointer as *const MSLLHOOKSTRUCT) };
        if event.flags & LLMHF_INJECTED == 0 {
            let used = HOOK.with_borrow(|hook| {
                hook.as_ref().is_some_and(|(binding, input)| {
                    let ticks = (event.mouseData >> 16) as i16;
                    !input.stopped()
                        && is_held(*binding)
                        && input.scroll(wheel_change(f64::from(ticks), 0.025 / 120.0))
                })
            });
            if used {
                return 1;
            }
        }
    }
    unsafe { CallNextHookEx(ptr::null_mut(), code, message, pointer) }
}

fn down(key: u16) -> bool {
    (unsafe { GetAsyncKeyState(i32::from(key)) }) < 0
}

fn is_held(binding: Binding) -> bool {
    let modifiers = [
        (down(VK_CONTROL), CTRL),
        (down(VK_SHIFT), SHIFT),
        (down(VK_MENU), ALT),
        (down(VK_LWIN) || down(VK_RWIN), META),
    ]
    .into_iter()
    .fold(0, |mask, (held, bit)| mask | if held { bit } else { 0 });
    if binding.modifiers != modifiers {
        return false;
    }
    match binding.trigger {
        None => true,
        Some(Trigger::Mouse(button)) => down(match button {
            2 => VK_MBUTTON,
            3 => VK_XBUTTON1,
            _ => VK_XBUTTON2,
        }),
        Some(Trigger::Keyboard(key)) => {
            let vk = unsafe {
                let thread = GetWindowThreadProcessId(GetForegroundWindow(), ptr::null_mut());
                MapVirtualKeyExW(
                    u32::from(key.scan),
                    MAPVK_VSC_TO_VK_EX,
                    GetKeyboardLayout(thread),
                )
            };
            vk != 0 && down(vk as u16)
        }
    }
}
