//! The candidate window: a borderless panel that never takes focus, parked
//! in a right-hand corner of the visible frame of the screen the user is
//! typing on, at a fixed spot instead of following the caret. Its contents
//! and layout come from [`squareinput::ui`]; layout units are points.
//! Numpad `*` moves it between the bottom-right and top-right corners; the
//! choice is saved in the data directory.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSBackingStoreType, NSBezierPath, NSColor, NSEvent, NSFont, NSFontAttributeName, NSForegroundColorAttributeName,
    NSPanel, NSScreen, NSStringDrawing, NSView, NSWindowCollectionBehavior, NSWindowLevel, NSWindowStyleMask,
};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};
use squareinput::ui::{self, Align, Font, HEIGHT, Item, MARGIN, Rgb, WIDTH};
use squareinput::{View, store};

/// kCGPopUpMenuWindowLevel: above ordinary and floating windows, like the
/// system's own candidate windows.
const POPUP_MENU_LEVEL: NSWindowLevel = 101;

/// Fonts by preference; the first one installed is used. PingFang HK covers
/// HKSCS; the system falls back per character for anything rarer.
const HAN_FONTS: [&str; 2] = ["PingFang HK", "PingFang TC"];
const UI_FONTS: [&str; 2] = ["PingFang HK", "PingFang TC"];

/// Setting file holding the corner ("top" or "bottom").
const CORNER_SETTING: &str = "corner";

define_class!(
    // SAFETY: NSView is meant to be subclassed; there is no Drop.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "SquareInputGridView"]
    #[ivars = RefCell<Vec<Item>>]
    struct GridView;

    impl GridView {
        /// Flipped, so the origin is the top-left corner as in the layout.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let drawn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if let Ok(items) = self.ivars().try_borrow() {
                    draw(&items);
                }
            }));
            if drawn.is_err() {
                store::log("caught a panic while drawing");
            }
        }
    }
);

impl GridView {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RefCell::new(Vec::new()));
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(f64::from(WIDTH), f64::from(HEIGHT)));
        // SAFETY: NSView's designated initializer.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

fn color(Rgb(r, g, b): Rgb) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(f64::from(r) / 255.0, f64::from(g) / 255.0, f64::from(b) / 255.0, 1.0)
}

fn rect(r: ui::Rect) -> NSRect {
    NSRect::new(NSPoint::new(f64::from(r.x), f64::from(r.y)), NSSize::new(f64::from(r.w), f64::from(r.h)))
}

fn font(font: Font, size: i32) -> Retained<NSFont> {
    let names = match font {
        Font::Han => HAN_FONTS,
        Font::Ui => UI_FONTS,
    };
    let size = f64::from(size);
    names
        .iter()
        .find_map(|name| NSFont::fontWithName_size(&NSString::from_str(name), size))
        .unwrap_or_else(|| NSFont::systemFontOfSize(size))
}

/// Draw [`ui::paint`]'s items into the current graphics context.
fn draw(items: &[Item]) {
    for item in items {
        match item {
            Item::Fill(r, c) => {
                color(*c).setFill();
                NSBezierPath::fillRect(rect(*r));
            }
            Item::Frame(r, c) => {
                color(*c).setStroke();
                // A one-point line just inside the rectangle.
                let r = rect(*r);
                let inner = NSRect::new(
                    NSPoint::new(r.origin.x + 0.5, r.origin.y + 0.5),
                    NSSize::new(r.size.width - 1.0, r.size.height - 1.0),
                );
                NSBezierPath::strokeRect(inner);
            }
            Item::Text { text, rect: r, size, font: f, color: c, align } => {
                let font = font(*f, *size);
                let ink = color(*c);
                // SAFETY: reading AppKit's attribute name constants.
                let keys = unsafe { [NSFontAttributeName, NSForegroundColorAttributeName] };
                let values: [&AnyObject; 2] = [font.as_ref(), ink.as_ref()];
                let attributes = NSDictionary::from_slices(&keys, &values);
                let string = NSString::from_str(text);
                // SAFETY: the attributes map AppKit's keys to a font and a colour.
                let size = unsafe { string.sizeWithAttributes(Some(&attributes)) };
                let r = rect(*r);
                let x = match align {
                    Align::Center => r.origin.x + (r.size.width - size.width) / 2.0,
                    Align::Left => r.origin.x,
                };
                let y = r.origin.y + (r.size.height - size.height) / 2.0;
                // SAFETY: as above, and the view's graphics context is current in drawRect:.
                unsafe { string.drawAtPoint_withAttributes(NSPoint::new(x, y), Some(&attributes)) };
            }
        }
    }
}

/// Build a panel around `view`: borderless, never key, on every Space.
fn build_panel(mtm: MainThreadMarker, view: &GridView) -> Retained<NSPanel> {
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(f64::from(WIDTH), f64::from(HEIGHT)));
    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        frame,
        NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
        NSBackingStoreType::Buffered,
        false,
    );
    panel.setHasShadow(true);
    panel.setBecomesKeyOnlyIfNeeded(true);
    panel.setIgnoresMouseEvents(true);
    // An input method is never the active application, so never hide on deactivation.
    panel.setHidesOnDeactivate(false);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary,
    );
    // Last, so nothing above resets it.
    panel.setLevel(POPUP_MENU_LEVEL);
    panel.setContentView(Some(view));
    panel
}

pub struct CandidateWindow {
    panel: RefCell<Retained<NSPanel>>,
    view: Retained<GridView>,
    at_top: Cell<bool>,
    mtm: MainThreadMarker,
}

impl CandidateWindow {
    pub fn new(mtm: MainThreadMarker) -> CandidateWindow {
        let view = GridView::new(mtm);
        let panel = build_panel(mtm, &view);
        CandidateWindow { panel: RefCell::new(panel), view, at_top: Cell::new(false), mtm }
    }

    /// Show `view` in the chosen corner of the screen holding `caret` (or the
    /// mouse, if the application gives no caret).
    pub fn show(&self, view: &View, digits: bool, caret: Option<NSRect>) {
        *self.view.ivars().borrow_mut() = ui::paint(view, digits);
        self.view.setNeedsDisplay(true);
        let panel = self.panel.borrow().clone();
        if !panel.isVisible() {
            // Re-read the saved corner, which another session may have changed.
            self.at_top.set(store::read_setting(CORNER_SETTING).as_deref() == Some("top"));
        }
        let visible = self.screen(caret);
        let (w, h, margin) = (f64::from(WIDTH), f64::from(HEIGHT), f64::from(MARGIN));
        let x = visible.origin.x + visible.size.width - w - margin;
        let y = if self.at_top.get() { visible.origin.y + visible.size.height - h - margin } else { visible.origin.y + margin };
        panel.setFrame_display(NSRect::new(NSPoint::new(x, y), NSSize::new(w, h)), true);
        panel.orderFrontRegardless();
        // A panel can stay bound to the Spaces that existed when it was made
        // and miss a full-screen Space opened later; a new panel joins it.
        if !panel.isOnActiveSpace() {
            panel.orderOut(None);
            let fresh = build_panel(self.mtm, &self.view);
            fresh.setFrame_display(panel.frame(), true);
            fresh.orderFrontRegardless();
            *self.panel.borrow_mut() = fresh;
        }
    }

    pub fn hide(&self) {
        self.panel.borrow().orderOut(None);
    }

    /// Move the window to the other right-hand corner and remember it.
    pub fn toggle_corner(&self) {
        let top = !self.at_top.get();
        self.at_top.set(top);
        store::write_setting(CORNER_SETTING, if top { "top" } else { "bottom" });
    }

    /// The visible frame of the screen holding `caret`, else the mouse, else the main screen.
    fn screen(&self, caret: Option<NSRect>) -> NSRect {
        let screens = NSScreen::screens(self.mtm);
        let holding = |p: NSPoint| {
            screens.iter().find(|s| {
                let f = s.frame();
                p.x >= f.origin.x && p.x < f.origin.x + f.size.width && p.y >= f.origin.y && p.y < f.origin.y + f.size.height
            })
        };
        let caret = caret.filter(|c| c.size.height > 0.0 || c.origin.x != 0.0 || c.origin.y != 0.0).map(|c| c.origin);
        caret
            .and_then(holding)
            .or_else(|| holding(NSEvent::mouseLocation()))
            .or_else(|| NSScreen::mainScreen(self.mtm))
            .map(|s| s.visibleFrame())
            .unwrap_or_else(|| NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1280.0, 800.0)))
    }
}
