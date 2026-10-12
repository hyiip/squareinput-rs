//! The input controller. IMKServer makes one per input session (text field);
//! they all share one engine and one candidate window, on the main thread.
//!
//! Only numeric keypad keys are handled, as on Windows: digits, `.`, `-` and
//! `+` go to the engine, `/` switches between shape input and plain digits,
//! `*` moves the window between the bottom-right and top-right corners. Keys
//! with Command, Control or Option held always pass through.

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};

use objc2::rc::{Allocated, Retained};
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, define_class, msg_send};
use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
use objc2_foundation::{NSNotFound, NSObjectProtocol, NSPoint, NSRange, NSRect, NSSize, NSString};
use objc2_input_method_kit::{IMKInputController, IMKServer};
use squareinput::{Engine, Key, store};

use crate::window::CandidateWindow;

/// What a keypad key does.
#[derive(Clone, Copy)]
enum Pad {
    Engine(Key),
    ToggleDigits,
    MoveWindow,
}

/// Keypad keys by virtual key code (kVK_ANSI_Keypad*).
fn pad_key(code: u16) -> Option<Pad> {
    Some(match code {
        0x52 => Pad::Engine(Key::Digit(0)),
        0x53 => Pad::Engine(Key::Digit(1)),
        0x54 => Pad::Engine(Key::Digit(2)),
        0x55 => Pad::Engine(Key::Digit(3)),
        0x56 => Pad::Engine(Key::Digit(4)),
        0x57 => Pad::Engine(Key::Digit(5)),
        0x58 => Pad::Engine(Key::Digit(6)),
        0x59 => Pad::Engine(Key::Digit(7)),
        0x5B => Pad::Engine(Key::Digit(8)),
        0x5C => Pad::Engine(Key::Digit(9)),
        0x41 => Pad::Engine(Key::Dot),
        0x4E => Pad::Engine(Key::Minus),
        0x45 => Pad::Engine(Key::Plus),
        0x4B => Pad::ToggleDigits,
        0x43 => Pad::MoveWindow,
        _ => return None,
    })
}

#[derive(Default)]
struct State {
    /// Created on first use; stays None (inert) if the data cannot be loaded.
    engine: Option<Engine>,
    engine_failed: bool,
    window: Option<CandidateWindow>,
    digits: bool,
    /// A session is active: the user is typing with this input method.
    active: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
}

/// Run `f` on the shared state unless it is already borrowed further up the stack.
fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.try_borrow_mut().ok().map(|mut s| f(&mut s)))
}

/// Run `f`, turning a panic into `fallback`: nothing may unwind into AppKit.
fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        store::log("caught a panic");
        fallback
    })
}

impl State {
    fn engine(&mut self) -> Option<&mut Engine> {
        if self.engine.is_none() && !self.engine_failed {
            self.engine = store::new_engine();
            self.engine_failed = self.engine.is_none();
        }
        self.engine.as_mut()
    }

    /// The pad action if we would eat the key.
    fn decide(&mut self, pad: Pad) -> Option<Pad> {
        let digits = self.digits;
        let engine = self.engine()?;
        match pad {
            Pad::ToggleDigits => Some(pad),
            // In digit mode `*` is just `*`, e.g. for a spreadsheet formula.
            Pad::MoveWindow => (!digits).then_some(pad),
            Pad::Engine(k) => (!digits && engine.wants(k)).then_some(pad),
        }
    }

    fn reset(&mut self) {
        if let Some(e) = self.engine.as_mut() {
            e.reset();
        }
    }

    /// Show the window while a session is active, near the caret of `client`.
    fn refresh(&mut self, mtm: MainThreadMarker, client: Option<&AnyObject>) {
        if self.active && self.engine().is_some() {
            let view = self.engine.as_ref().map(Engine::view);
            let window = self.window.get_or_insert_with(|| CandidateWindow::new(mtm));
            if let Some(view) = view {
                window.show(&view, self.digits, client.map(caret));
            }
        } else if let Some(w) = &self.window {
            w.hide();
        }
    }
}

/// `{NSNotFound, 0}`: replace nothing, insert at the insertion point.
const NO_REPLACEMENT: NSRange = NSRange::new(NSNotFound as usize, 0);

fn insert_text(client: &AnyObject, text: &str) {
    let string = NSString::from_str(text);
    // SAFETY: IMK hands us a client conforming to IMKTextInput, which has
    // insertText:replacementRange:.
    unsafe {
        let _: () = msg_send![client, insertText: &*string, replacementRange: NO_REPLACEMENT];
    }
}

/// The caret line of `client` in screen coordinates, to pick the screen the
/// user is typing on; zero if the application does not say.
fn caret(client: &AnyObject) -> NSRect {
    let mut rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0));
    // SAFETY: attributesForCharacterIndex:lineHeightRectangle: is part of
    // IMKTextInput; it fills `rect` and returns an autoreleased dictionary.
    unsafe {
        let _: Option<Retained<AnyObject>> = msg_send![client, attributesForCharacterIndex: 0usize, lineHeightRectangle: &mut rect];
    }
    rect
}

fn handle(event: &NSEvent, client: &AnyObject) -> bool {
    if event.r#type() != NSEventType::KeyDown {
        return false;
    }
    let chord = NSEventModifierFlags::Command | NSEventModifierFlags::Control | NSEventModifierFlags::Option;
    if event.modifierFlags().intersects(chord) {
        return false;
    }
    let (Some(pad), Some(mtm)) = (pad_key(event.keyCode()), MainThreadMarker::new()) else { return false };
    let step = with(|s| {
        s.active = true;
        let pad = s.decide(pad)?;
        let commit = match pad {
            Pad::ToggleDigits => {
                s.digits = !s.digits;
                s.reset();
                None
            }
            Pad::MoveWindow => {
                if let Some(w) = &s.window {
                    w.toggle_corner();
                }
                None
            }
            Pad::Engine(k) => {
                let engine = s.engine()?;
                let outcome = engine.key(k);
                if outcome.learned
                    && let Some(line) = engine.promotions().last_line()
                {
                    store::save_promotion(&line);
                }
                outcome.commit
            }
        };
        s.refresh(mtm, Some(client));
        Some(commit)
    });
    let Some(Some(commit)) = step else { return false };
    if let Some(text) = commit {
        insert_text(client, &text);
    }
    true
}

/// A session starts or ends: show or hide the window, dropping any code being typed.
fn set_active(active: bool, client: Option<&AnyObject>) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    with(|s| {
        s.active = active;
        s.reset();
        s.refresh(mtm, client);
    });
}

define_class!(
    // SAFETY: IMKInputController is meant to be subclassed; there is no Drop.
    #[unsafe(super(IMKInputController))]
    // Must match InputMethodServerControllerClass in Info.plist.
    #[name = "SquareInputController"]
    #[ivars = ()]
    pub struct Controller;

    impl Controller {
        /// The designated initializer IMKServer calls for each new session.
        #[unsafe(method_id(initWithServer:delegate:client:))]
        fn init_with_server(
            this: Allocated<Self>,
            server: Option<&IMKServer>,
            delegate: Option<&AnyObject>,
            client: Option<&AnyObject>,
        ) -> Option<Retained<Self>> {
            let this = this.set_ivars(());
            // SAFETY: forwards IMK's own arguments to the superclass initializer.
            unsafe { msg_send![super(this), initWithServer: server, delegate: delegate, client: client] }
        }

        /// Every key event; returning true keeps it from the application.
        #[unsafe(method(handleEvent:client:))]
        fn handle_event(&self, event: Option<&NSEvent>, client: Option<&AnyObject>) -> bool {
            match (event, client) {
                (Some(event), Some(client)) => guard(false, || handle(event, client)),
                _ => false,
            }
        }

        #[unsafe(method(activateServer:))]
        fn activate_server(&self, sender: Option<&AnyObject>) {
            guard((), || set_active(true, sender));
        }

        #[unsafe(method(deactivateServer:))]
        fn deactivate_server(&self, sender: Option<&AnyObject>) {
            guard((), || set_active(false, sender));
        }

        /// The application wants any composition finished (focus moved, input
        /// source switched). Nothing is ever pending in the document, so just
        /// drop the code being typed.
        #[unsafe(method(commitComposition:))]
        fn commit_composition(&self, sender: Option<&AnyObject>) {
            guard((), || {
                let Some(mtm) = MainThreadMarker::new() else { return };
                with(|s| {
                    s.reset();
                    s.refresh(mtm, sender);
                });
            });
        }
    }

    unsafe impl NSObjectProtocol for Controller {}
);
