use std::{collections::HashMap, mem::MaybeUninit, ptr, slice, sync::mpsc::SyncSender};

use x11_dl::{xinput2::*, xlib::*};

use super::{binding::*, Input, Status};

struct DisplayConnection<'a> {
    x: &'a Xlib,
    display: *mut Display,
}

impl Drop for DisplayConnection<'_> {
    fn drop(&mut self) {
        unsafe { (self.x.XCloseDisplay)(self.display) };
    }
}

pub(super) fn run(binding: Binding, input: Input, _: bool, ready: SyncSender<Status>) {
    let wayland = std::env::var("XDG_SESSION_TYPE")
        .is_ok_and(|kind| kind.eq_ignore_ascii_case("wayland"))
        || std::env::var_os("WAYLAND_DISPLAY").is_some();
    if wayland {
        let _ = ready.send(Status::Wayland);
        return;
    }
    let (Ok(x), Ok(xi)) = (Xlib::open(), XInput2::open()) else {
        let _ = ready.send(Status::Local);
        return;
    };
    let display = unsafe { (x.XOpenDisplay)(ptr::null()) };
    if display.is_null() {
        let _ = ready.send(Status::Local);
        return;
    }
    let _connection = DisplayConnection { x: &x, display };
    // The private X11 connection and all XInput event storage stay on this thread.
    unsafe { listen(&x, &xi, display, binding, input, ready) };
}

unsafe fn listen(
    x: &Xlib,
    xi: &XInput2,
    display: *mut Display,
    binding: Binding,
    input: Input,
    ready: SyncSender<Status>,
) {
    let (mut opcode, mut event, mut error) = (0, 0, 0);
    let (mut major, mut minor) = (2, 1);
    if (x.XQueryExtension)(
        display,
        c"XInputExtension".as_ptr(),
        &mut opcode,
        &mut event,
        &mut error,
    ) == 0
        || (xi.XIQueryVersion)(display, &mut major, &mut minor) != 0
        || (major, minor) < (2, 1)
    {
        let _ = ready.send(Status::Local);
        return;
    }
    let root = (x.XDefaultRootWindow)(display);
    let mut raw = (XI_RawButtonPressMask | XI_RawMotionMask).to_le_bytes();
    let mut changes = (XI_HierarchyChangedMask | XI_DeviceChangedMask).to_le_bytes();
    let mut masks = [
        XIEventMask {
            deviceid: XIAllMasterDevices,
            mask_len: raw.len() as i32,
            mask: raw.as_mut_ptr(),
        },
        XIEventMask {
            deviceid: XIAllDevices,
            mask_len: changes.len() as i32,
            mask: changes.as_mut_ptr(),
        },
    ];
    if (xi.XISelectEvents)(display, root, masks.as_mut_ptr(), masks.len() as i32) != 0 {
        let _ = ready.send(Status::Local);
        return;
    }
    (x.XSync)(display, 0);
    let mut axes = scroll_axes(xi, display);
    let mut fd = libc::pollfd {
        fd: (x.XConnectionNumber)(display),
        events: libc::POLLIN,
        revents: 0,
    };
    let _ = ready.send(Status::X11);
    while !input.stopped() {
        if (x.XPending)(display) == 0 {
            if libc::poll(&mut fd, 1, 100) <= 0 {
                continue;
            }
            if fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                break;
            }
        }
        while !input.stopped() && (x.XPending)(display) > 0 {
            let mut event = MaybeUninit::<XEvent>::uninit();
            (x.XNextEvent)(display, event.as_mut_ptr());
            let mut event = event.assume_init();
            if event.type_ != GenericEvent || event.generic_event_cookie.extension != opcode {
                continue;
            }
            let cookie = &mut event.generic_event_cookie;
            if (x.XGetEventData)(display, cookie) == 0 {
                continue;
            }
            let kind = cookie.evtype;
            if kind == XI_HierarchyChanged || kind == XI_DeviceChanged {
                axes = scroll_axes(xi, display);
            } else if !cookie.data.is_null() && (kind == XI_RawMotion || kind == XI_RawButtonPress)
            {
                let event = &*cookie.data.cast::<XIRawEvent>();
                let delta = scroll_delta(&axes, event);
                if delta != 0.0 && is_held(x, xi, display, root, event.deviceid, binding) {
                    input.scroll(wheel_change(delta, 0.025));
                }
            }
            (x.XFreeEventData)(display, cookie);
        }
    }
}

type ScrollAxes = HashMap<(i32, usize), (i32, f64)>;

unsafe fn scroll_axes(xi: &XInput2, display: *mut Display) -> ScrollAxes {
    let mut axes = HashMap::new();
    let mut count = 0;
    let devices = (xi.XIQueryDevice)(display, XIAllDevices, &mut count);
    if devices.is_null() {
        return axes;
    }
    for device in slice::from_raw_parts(devices, count.max(0) as usize) {
        if device.classes.is_null() {
            continue;
        }
        for &class in slice::from_raw_parts(device.classes, device.num_classes.max(0) as usize) {
            if !class.is_null() && (*class)._type == XIScrollClass {
                let scroll = &*class.cast::<XIScrollClassInfo>();
                if scroll.number >= 0 && scroll.increment.is_finite() && scroll.increment != 0.0 {
                    axes.insert(
                        (scroll.sourceid, scroll.number as usize),
                        (scroll.scroll_type, scroll.increment),
                    );
                }
            }
        }
    }
    (xi.XIFreeDeviceInfo)(devices);
    axes
}

unsafe fn scroll_delta(axes: &ScrollAxes, event: &XIRawEvent) -> f64 {
    if event.evtype == XI_RawButtonPress {
        if event.flags & XIPointerEmulated != 0 {
            return 0.0;
        }
        return match event.detail {
            4 => 1.0,
            5 => -1.0,
            _ => 0.0,
        };
    }
    let state = event.valuators;
    if state.mask.is_null() || state.values.is_null() || state.mask_len <= 0 {
        return 0.0;
    }
    let mut value = state.values;
    let (mut vertical, mut horizontal) = (0.0_f64, 0.0_f64);
    for (byte, mask) in slice::from_raw_parts(state.mask, state.mask_len as usize)
        .iter()
        .enumerate()
    {
        for bit in 0..8 {
            if mask & (1 << bit) == 0 {
                continue;
            }
            if let Some(&(axis, increment)) = axes.get(&(event.sourceid, byte * 8 + bit)) {
                let delta = *value / increment;
                if axis == XIScrollTypeVertical {
                    vertical -= delta;
                } else {
                    horizontal += delta;
                }
            }
            value = value.add(1);
        }
    }
    if horizontal.abs() > vertical.abs() {
        0.0
    } else {
        vertical
    }
}

unsafe fn is_held(
    x: &Xlib,
    xi: &XInput2,
    display: *mut Display,
    root: Window,
    device: i32,
    binding: Binding,
) -> bool {
    let mut keys = [0_i8; 32];
    (x.XQueryKeymap)(display, keys.as_mut_ptr().cast());
    let down = |code: u8| keys[usize::from(code / 8)] as u8 & (1 << (code % 8)) != 0;
    let modifiers = [
        (37, 105, CTRL),
        (50, 62, SHIFT),
        (64, 108, ALT),
        (133, 134, META),
    ]
    .into_iter()
    .fold(0, |mask, (left, right, bit)| {
        mask | if down(left) || down(right) { bit } else { 0 }
    });
    if modifiers != binding.modifiers {
        return false;
    }
    match binding.trigger {
        None => true,
        Some(Trigger::Keyboard(key)) => key.x11 != 0 && down(key.x11),
        Some(Trigger::Mouse(button)) => {
            let (mut root_return, mut child) = (0, 0);
            let (mut rx, mut ry, mut wx, mut wy) = (0.0, 0.0, 0.0, 0.0);
            let (mut buttons, mut mods, mut group) = (
                XIButtonState::default(),
                XIModifierState::default(),
                XIGroupState::default(),
            );
            let ok = (xi.XIQueryPointer)(
                display,
                device,
                root,
                &mut root_return,
                &mut child,
                &mut rx,
                &mut ry,
                &mut wx,
                &mut wy,
                &mut buttons,
                &mut mods,
                &mut group,
            );
            if buttons.mask.is_null() {
                return false;
            }
            let code = match button {
                2 => 3,
                3 => 8,
                _ => 9,
            };
            let pressed = ok != 0
                && buttons.mask_len > code / 8
                && *buttons.mask.add((code / 8) as usize) & (1 << (code % 8)) != 0;
            (x.XFree)(buttons.mask.cast());
            pressed
        }
    }
}
