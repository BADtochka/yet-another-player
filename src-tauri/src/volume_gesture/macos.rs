use std::{
    ffi::{c_void, CStr},
    sync::mpsc::SyncSender,
    time::Duration,
};

use core_foundation::{
    base::TCFType,
    boolean::CFBoolean,
    dictionary::{CFDictionary, CFDictionaryRef},
    mach_port::{CFMachPort, CFMachPortRef},
    runloop::{kCFRunLoopDefaultMode, CFRunLoop},
    string::{CFString, CFStringRef},
};
use core_graphics::{
    event::{
        CGEventFlags, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
        EventField,
    },
    event_source::CGEventSourceStateID,
};

use super::{binding::*, Input, Status};

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> u8;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceKeyState(state: CGEventSourceStateID, key: u16) -> bool;
    fn CGEventSourceButtonState(state: CGEventSourceStateID, button: u32) -> bool;
    fn CGEventTapIsEnabled(tap: CFMachPortRef) -> bool;
    fn CGEventTapEnable(tap: CFMachPortRef, enabled: bool);
    fn CGEventTapCreate(
        location: CGEventTapLocation,
        placement: CGEventTapPlacement,
        options: CGEventTapOptions,
        mask: u64,
        callback: unsafe extern "C" fn(*mut c_void, u32, Event, *mut c_void) -> Event,
        context: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventGetFlags(event: Event) -> CGEventFlags;
    fn CGEventGetIntegerValueField(event: Event, field: u32) -> i64;
    fn CGEventGetDoubleValueField(event: Event, field: u32) -> f64;
}

type Event = *mut c_void;
type AccessFn = unsafe extern "C" fn() -> bool;
const SCROLL: u32 = CGEventType::ScrollWheel as u32;
const MOMENTUM_PHASE: u32 = 123;

struct Context {
    binding: Binding,
    input: Input,
}

struct Access {
    post: Option<AccessFn>,
    request_post: Option<AccessFn>,
}

impl Access {
    fn system() -> Self {
        fn resolve(name: &CStr) -> Option<AccessFn> {
            // These Quartz APIs are available from 10.15. Resolve them at runtime
            // so older installations can still use the legacy accessibility grant.
            let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
            (!symbol.is_null())
                .then(|| unsafe { std::mem::transmute::<*mut c_void, AccessFn>(symbol) })
        }
        Self {
            post: resolve(c"CGPreflightPostEventAccess"),
            request_post: resolve(c"CGRequestPostEventAccess"),
        }
    }

    fn granted(&self) -> bool {
        match self.post {
            Some(preflight) => unsafe { preflight() },
            None => unsafe { AXIsProcessTrusted() != 0 },
        }
    }

    fn request(&self) {
        if self.granted() {
            return;
        }
        if let Some(request) = self.request_post {
            unsafe { request() };
        } else {
            unsafe {
                let prompt = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
                let options = CFDictionary::from_CFType_pairs(&[(prompt, CFBoolean::true_value())]);
                AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef());
            }
        }
    }

    fn open(&self, context: &mut Context) -> Option<CFMachPort> {
        // A legacy Accessibility grant can also permit a filtering tap.
        if !self.granted() && unsafe { AXIsProcessTrusted() == 0 } {
            return None;
        }
        let port = unsafe {
            CGEventTapCreate(
                CGEventTapLocation::Session,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::Default,
                1 << SCROLL,
                scroll,
                std::ptr::from_mut(context).cast(),
            )
        };
        (!port.is_null()).then(|| unsafe { CFMachPort::wrap_under_create_rule(port) })
    }
}

pub(super) fn run(binding: Binding, input: Input, request: bool, ready: SyncSender<Status>) {
    let access = Access::system();
    let mut context = Context { binding, input };
    // The source is installed on this thread and removed before its context goes out of scope.
    let tap = access.open(&mut context).or_else(|| {
        if request {
            access.request();
            access.open(&mut context)
        } else {
            None
        }
    });
    let Some(tap) = tap else {
        let result = if access.granted() {
            Status::MacInterceptionUnavailable
        } else {
            Status::PermissionRequired
        };
        let _ = ready.send(result);
        return;
    };
    let port = tap.as_concrete_TypeRef();
    let Ok(source) = tap.create_runloop_source(0) else {
        let _ = ready.send(Status::MacInterceptionUnavailable);
        return;
    };
    let mode = unsafe { kCFRunLoopDefaultMode };
    let run_loop = CFRunLoop::get_current();
    run_loop.add_source(&source, mode);
    unsafe { CGEventTapEnable(port, true) };
    let _ = ready.send(Status::Global);
    while !context.input.stopped() {
        CFRunLoop::run_in_mode(mode, Duration::from_millis(100), false);
        if !unsafe { CGEventTapIsEnabled(port) } {
            unsafe { CGEventTapEnable(port, true) };
        }
    }
    run_loop.remove_source(&source, mode);
}

unsafe extern "C" fn scroll(
    _: *mut c_void,
    kind: u32,
    event: Event,
    context: *mut c_void,
) -> Event {
    if kind != SCROLL || event.is_null() {
        return event;
    }
    let context = unsafe { &*context.cast::<Context>() };
    if context.input.stopped() || !is_held(context.binding, unsafe { CGEventGetFlags(event) }) {
        return event;
    }
    let continuous =
        unsafe { CGEventGetIntegerValueField(event, EventField::SCROLL_WHEEL_EVENT_IS_CONTINUOUS) }
            != 0;
    let (vertical, horizontal, scale) = if continuous {
        (
            EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_1,
            EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_2,
            0.001,
        )
    } else {
        (
            EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_1,
            EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_2,
            0.025,
        )
    };
    let y = unsafe { CGEventGetDoubleValueField(event, vertical) };
    let x = unsafe { CGEventGetDoubleValueField(event, horizontal) };
    if y == 0.0 || x.abs() > y.abs() {
        return event;
    }
    // A matched inertial tail should not keep adjusting volume.
    let inertia = unsafe { CGEventGetIntegerValueField(event, MOMENTUM_PHASE) } != 0;
    if inertia || context.input.scroll(wheel_change(y, scale)) {
        std::ptr::null_mut()
    } else {
        event
    }
}

fn is_held(binding: Binding, flags: CGEventFlags) -> bool {
    let modifiers = [
        (CGEventFlags::CGEventFlagControl, CTRL),
        (CGEventFlags::CGEventFlagShift, SHIFT),
        (CGEventFlags::CGEventFlagAlternate, ALT),
        (CGEventFlags::CGEventFlagCommand, META),
    ]
    .into_iter()
    .fold(0, |mask, (flag, bit)| {
        mask | if flags.contains(flag) { bit } else { 0 }
    });
    if binding.modifiers != modifiers {
        return false;
    }
    let state = CGEventSourceStateID::CombinedSessionState;
    match binding.trigger {
        None => true,
        Some(Trigger::Keyboard(key)) => unsafe { CGEventSourceKeyState(state, key.mac) },
        Some(Trigger::Mouse(button)) => unsafe {
            CGEventSourceButtonState(state, u32::from(button))
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering::Relaxed;

    use core_foundation::base::CFType;
    use core_graphics::event::{CGScrollEventUnit, ScrollEventUnit};

    use super::*;

    extern "C" {
        fn CGEventCreateScrollWheelEvent2(
            source: *const c_void,
            units: CGScrollEventUnit,
            wheel_count: u32,
            wheel1: i32,
            wheel2: i32,
            wheel3: i32,
        ) -> Event;
        fn CGEventSetFlags(event: Event, flags: CGEventFlags);
    }

    #[test]
    fn only_bound_scroll_is_consumed_and_changes_volume() {
        let mut context = Context {
            binding: Binding {
                modifiers: ALT,
                trigger: None,
            },
            input: Input {
                stop: Default::default(),
                pending: Default::default(),
            },
        };
        let event = unsafe {
            CGEventCreateScrollWheelEvent2(std::ptr::null(), ScrollEventUnit::LINE, 1, 1, 0, 0)
        };
        assert!(!event.is_null());
        let _event = unsafe { CFType::wrap_under_create_rule(event.cast_const()) };
        let context_ptr = std::ptr::from_mut(&mut context).cast();

        unsafe { CGEventSetFlags(event, CGEventFlags::empty()) };
        assert_eq!(
            unsafe { scroll(std::ptr::null_mut(), SCROLL, event, context_ptr) },
            event
        );
        unsafe { CGEventSetFlags(event, CGEventFlags::CGEventFlagAlternate) };
        assert!(unsafe { scroll(std::ptr::null_mut(), SCROLL, event, context_ptr) }.is_null());
        assert_eq!(context.input.pending.delta.load(Relaxed), 25_000);
    }
}
