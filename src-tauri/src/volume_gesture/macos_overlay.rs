use std::cell::RefCell;

use objc2::{rc::Retained, MainThreadMarker};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSEvent, NSFont, NSImage, NSImageScaling, NSImageView, NSPanel,
    NSScreen, NSStatusWindowLevel, NSTextAlignment, NSTextField, NSView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{
    NSOperatingSystemVersion, NSPoint, NSProcessInfo, NSRect, NSSize, NSString,
};

use super::{OVERLAY_HEIGHT, OVERLAY_WIDTH};

thread_local! {
    static OVERLAY: RefCell<Option<Overlay>> = const { RefCell::new(None) };
}

const METER_WIDTH: f64 = 120.0;

struct Overlay {
    panel: Retained<NSPanel>,
    percent: Retained<NSTextField>,
    fill: Retained<NSView>,
    speaker: Retained<NSImageView>,
    symbols: [Option<Retained<NSImage>>; 3],
    level: Option<u8>,
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}

fn color(r: u8, g: u8, b: u8) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
        1.0,
    )
}

fn rounded(mtm: MainThreadMarker, frame: NSRect, color: &NSColor, radius: f64) -> Retained<NSView> {
    let view = NSView::initWithFrame(mtm.alloc(), frame);
    view.setWantsLayer(true);
    if let Some(layer) = view.layer() {
        layer.setBackgroundColor(Some(&color.CGColor()));
        layer.setCornerRadius(radius);
        layer.setMasksToBounds(true);
    }
    view
}

fn at_least(major: isize) -> bool {
    NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(NSOperatingSystemVersion {
        majorVersion: major,
        minorVersion: 0,
        patchVersion: 0,
    })
}

impl Overlay {
    fn new(mtm: MainThreadMarker) -> Self {
        let frame = rect(0.0, 0.0, OVERLAY_WIDTH, OVERLAY_HEIGHT);
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            mtm.alloc(),
            frame,
            NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            false,
        );
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(true);
        panel.setCanHide(false);
        panel.setHidesOnDeactivate(false);
        panel.setExcludedFromWindowsMenu(true);
        panel.setIgnoresMouseEvents(true);
        panel.setBecomesKeyOnlyIfNeeded(true);
        panel.setFloatingPanel(true);
        panel.setLevel(NSStatusWindowLevel);
        let fullscreen = if at_least(13) {
            NSWindowCollectionBehavior::CanJoinAllApplications
        } else {
            NSWindowCollectionBehavior::FullScreenAuxiliary
        };
        panel.setCollectionBehavior(
            fullscreen
                | NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::IgnoresCycle,
        );

        let text = color(250, 250, 250);
        let card = rounded(mtm, frame, &color(16, 16, 18), 12.0);
        if let Some(layer) = card.layer() {
            layer.setBorderColor(Some(&color(36, 36, 39).CGColor()));
            layer.setBorderWidth(1.0);
        }
        let badge = rounded(mtm, rect(12.0, 20.0, 34.0, 34.0), &color(47, 75, 190), 8.0);
        let speaker = NSImageView::initWithFrame(mtm.alloc(), rect(5.0, 5.0, 24.0, 24.0));
        speaker.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        speaker.setContentTintColor(Some(&text));
        badge.addSubview(&speaker);

        let title = NSTextField::labelWithString(&NSString::from_str("Громкость YAP"), mtm);
        title.setFrame(rect(56.0, 35.0, 122.0, 19.0));
        title.setFont(Some(&NSFont::systemFontOfSize_weight(13.0, 0.0)));
        title.setTextColor(Some(&text));
        let percent = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        percent.setFrame(rect(184.0, 27.0, 40.0, 20.0));
        percent.setFont(Some(&NSFont::monospacedDigitSystemFontOfSize_weight(
            13.0, 0.4,
        )));
        percent.setTextColor(Some(&text));
        percent.setAlignment(NSTextAlignment::Right);
        let track = rounded(
            mtm,
            rect(56.0, 20.0, METER_WIDTH, 5.0),
            &color(42, 42, 46),
            3.0,
        );
        let fill = rounded(mtm, rect(0.0, 0.0, 0.0, 5.0), &color(61, 90, 203), 0.0);
        track.addSubview(&fill);
        card.addSubview(&badge);
        card.addSubview(&title);
        card.addSubview(&track);
        card.addSubview(&percent);
        panel.setContentView(Some(&card));

        let symbols = [
            "speaker.slash.fill",
            "speaker.wave.1.fill",
            "speaker.wave.2.fill",
        ]
        .map(|name| {
            if at_least(11) {
                NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    &NSString::from_str(name),
                    None,
                )
            } else {
                NSImage::imageNamed(&NSString::from_str("NSActionTemplate"))
            }
        });
        Self {
            panel,
            percent,
            fill,
            speaker,
            symbols,
            level: None,
        }
    }

    fn present(&mut self, mtm: MainThreadMarker, level: u8) {
        if self.level != Some(level) {
            let icon = if level == 0 {
                0
            } else if level < 60 {
                1
            } else {
                2
            };
            self.speaker.setImage(self.symbols[icon].as_deref());
            self.percent
                .setStringValue(&NSString::from_str(&format!("{level}%")));
            self.fill
                .setFrameSize(NSSize::new(METER_WIDTH * f64::from(level) / 100.0, 5.0));
            self.level = Some(level);
        }
        let cursor = NSEvent::mouseLocation();
        let screens = NSScreen::screens(mtm);
        let screen = screens
            .iter()
            .find(|screen| {
                let frame = screen.frame();
                cursor.x >= frame.origin.x
                    && cursor.x < frame.origin.x + frame.size.width
                    && cursor.y >= frame.origin.y
                    && cursor.y < frame.origin.y + frame.size.height
            })
            .or_else(|| NSScreen::mainScreen(mtm));
        if let Some(screen) = screen {
            let bounds = screen.visibleFrame();
            self.panel.setFrameOrigin(NSPoint::new(
                (bounds.origin.x + (bounds.size.width - OVERLAY_WIDTH) / 2.0).round(),
                (bounds.origin.y + 48.0).round(),
            ));
        }
        self.panel.orderFrontRegardless();
    }
}

pub(super) fn show(level: u8) {
    if let Some(mtm) = MainThreadMarker::new() {
        OVERLAY.with_borrow_mut(|slot| {
            slot.get_or_insert_with(|| Overlay::new(mtm))
                .present(mtm, level)
        });
    }
}

pub(super) fn hide() {
    OVERLAY.with_borrow(|slot| {
        if let Some(overlay) = slot {
            overlay.panel.orderOut(None);
        }
    });
}

pub(super) fn close() {
    hide();
    OVERLAY.with_borrow_mut(|slot| {
        slot.take();
    });
}
