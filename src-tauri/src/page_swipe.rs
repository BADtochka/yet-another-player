use std::{cell::Cell, ptr::NonNull};

use block2::RcBlock;
use objc2::{rc::Retained, runtime::AnyObject, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSEvent, NSEventMask, NSEventModifierFlags, NSEventPhase};
use serde::Serialize;
use tauri::{Emitter, WebviewWindow, WindowEvent};

thread_local! {
    static MONITOR: Cell<Option<Retained<AnyObject>>> = const { Cell::new(None) };
}

const PREVIEW_DISTANCE: f64 = 3.0;
const COMMIT_DISTANCE: f64 = 76.0;
const HORIZONTAL_RATIO: f64 = 1.6;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "phase", rename_all = "camelCase")]
enum Swipe {
    Preview { x: f64, y: f64, back: bool },
    Commit { x: f64, y: f64, back: bool },
    Cancel,
}

#[derive(Clone, Copy, Default)]
struct Gesture {
    start: Option<(f64, f64)>,
    horizontal: f64,
    vertical: f64,
    preview_back: Option<bool>,
}

impl Gesture {
    fn cancel(&mut self) -> Option<Swipe> {
        std::mem::take(self).preview_back.map(|_| Swipe::Cancel)
    }

    fn update(
        &mut self,
        phase: NSEventPhase,
        start: Option<(f64, f64)>,
        dx: f64,
        dy: f64,
    ) -> Option<Swipe> {
        if phase.contains(NSEventPhase::Cancelled) {
            return self.cancel();
        }
        let began = phase.contains(NSEventPhase::Began);
        if began {
            *self = Self {
                start,
                preview_back: self.preview_back,
                ..Self::default()
            };
        }
        let Some((x, y)) = self.start else {
            return self.cancel();
        };
        self.horizontal += dx;
        self.vertical += dy.abs();
        let distance = self.horizontal.abs();
        let horizontal_threshold = self.vertical * HORIZONTAL_RATIO;
        let back = self.horizontal > 0.0;
        if phase.contains(NSEventPhase::Ended) {
            let cancel = self.cancel();
            return if distance >= COMMIT_DISTANCE.max(horizontal_threshold) {
                Some(Swipe::Commit { x, y, back })
            } else {
                cancel
            };
        }

        let preview_back = (distance >= PREVIEW_DISTANCE.max(horizontal_threshold)).then_some(back);
        let previous = std::mem::replace(&mut self.preview_back, preview_back);
        match preview_back {
            Some(back) if began || preview_back != previous => Some(Swipe::Preview { x, y, back }),
            None if previous.is_some() => Some(Swipe::Cancel),
            _ => None,
        }
    }
}

pub(crate) fn install(window: &WebviewWindow) -> tauri::Result<()> {
    let target = window.clone();
    window.with_webview(move |webview| {
        let Some(main_thread) = MainThreadMarker::new() else {
            return;
        };
        // Tauri supplies the live NSWindow on the main thread for this callback.
        let native_window = unsafe { &*webview.ns_window().cast::<objc2_app_kit::NSWindow>() };
        MONITOR.set(monitor_window(
            native_window.windowNumber(),
            main_thread,
            move |swipe| {
                let _ = target.emit("page-swipe", swipe);
            },
        ));
    })?;
    window.on_window_event(|event| {
        if matches!(event, WindowEvent::Destroyed) {
            if let Some(monitor) = MONITOR.take() {
                // This token came from AppKit's local event monitor registration.
                unsafe { NSEvent::removeMonitor(&monitor) };
            }
        }
    });
    Ok(())
}

fn monitor_window(
    window_number: isize,
    main_thread: MainThreadMarker,
    on_swipe: impl Fn(Swipe) + 'static,
) -> Option<Retained<AnyObject>> {
    let app = NSApplication::sharedApplication(main_thread);
    let gesture = Cell::new(Gesture::default());
    let handler = RcBlock::new(move |pointer: NonNull<NSEvent>| {
        // AppKit keeps this event alive for the duration of the callback.
        let event = unsafe { pointer.as_ref() };
        let phase = event.phase();
        if !event.hasPreciseScrollingDeltas()
            || !event.momentumPhase().is_empty()
            || !phase.intersects(
                NSEventPhase::Began
                    | NSEventPhase::Changed
                    | NSEventPhase::Ended
                    | NSEventPhase::Cancelled,
            )
        {
            return pointer.as_ptr();
        }
        let event_window = event.window(main_thread);
        let screen_coordinates = event_window.is_none();
        let modified = event.modifierFlags().intersects(
            NSEventModifierFlags::Control
                | NSEventModifierFlags::Shift
                | NSEventModifierFlags::Option
                | NSEventModifierFlags::Command,
        );
        let Some(window) = event_window
            .or_else(|| app.keyWindow())
            .filter(|window| !modified && window.windowNumber() == window_number)
        else {
            if let Some(cancel) = gesture.take().cancel() {
                on_swipe(cancel);
            }
            return pointer.as_ptr();
        };
        let start = if phase.contains(NSEventPhase::Began) {
            window.contentView().map(|view| {
                let mut point = event.locationInWindow();
                if screen_coordinates {
                    point = window.convertPointFromScreen(point);
                }
                let point = view.convertPoint_fromView(point, None);
                let y = if view.isFlipped() {
                    point.y
                } else {
                    view.bounds().size.height - point.y
                };
                (point.x, y)
            })
        } else {
            None
        };
        let mut active = gesture.get();
        let swipe = active.update(
            phase,
            start,
            event.scrollingDeltaX(),
            event.scrollingDeltaY(),
        );
        gesture.set(active);
        if let Some(swipe) = swipe {
            on_swipe(swipe);
        }
        pointer.as_ptr()
    });
    // The callback returns AppKit's original event pointer without retaining or replacing it.
    unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::ScrollWheel, &handler)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_swipes_show_a_preview_before_committing() {
        let mut gesture = Gesture::default();
        for (delta, back) in [(48.0, true), (-48.0, false), (48.0, true)] {
            assert!(gesture
                .update(NSEventPhase::Began, Some((24.0, 400.0)), 0.0, 2.0)
                .is_none());
            assert_eq!(
                gesture.update(NSEventPhase::Changed, None, delta / 8.0, 1.0),
                Some(Swipe::Preview {
                    x: 24.0,
                    y: 400.0,
                    back
                })
            );
            assert!(gesture
                .update(NSEventPhase::Changed, None, delta, 1.0)
                .is_none());
            assert!(gesture
                .update(NSEventPhase::Changed, None, delta, 1.0)
                .is_none());
            assert_eq!(
                gesture.update(NSEventPhase::Ended, None, 0.0, 0.0),
                Some(Swipe::Commit {
                    x: 24.0,
                    y: 400.0,
                    back
                })
            );
            assert!(gesture
                .update(NSEventPhase::Changed, None, delta, 0.0)
                .is_none());
        }
    }

    #[test]
    fn vertical_and_incomplete_gestures_do_not_navigate() {
        let mut gesture = Gesture::default();
        gesture.update(NSEventPhase::Began, Some((110.0, 80.0)), 0.0, 12.0);
        assert!(gesture
            .update(NSEventPhase::Changed, None, 90.0, 100.0)
            .is_none());
        assert!(gesture
            .update(NSEventPhase::Ended, None, 0.0, 0.0)
            .is_none());
        for (delta, end) in [(90.0, NSEventPhase::Cancelled), (12.0, NSEventPhase::Ended)] {
            gesture.update(NSEventPhase::Began, Some((110.0, 80.0)), 0.0, 0.0);
            assert!(matches!(
                gesture.update(NSEventPhase::Changed, None, delta, 0.0),
                Some(Swipe::Preview { .. })
            ));
            assert_eq!(gesture.update(end, None, 0.0, 0.0), Some(Swipe::Cancel));
        }
        assert!(gesture
            .update(NSEventPhase::Ended, None, 0.0, 0.0)
            .is_none());
    }
}
