//! The TSF text service: takes numpad keys, runs them through the engine,
//! inserts committed text and keeps the candidate window in step. The COM
//! objects are declared in `sys`; their interface methods are here.
//!
//! Every COM method body runs under `guard`, so a panic can never unwind into
//! the host application. No `RefCell` borrow is held while calling into TSF,
//! which may call back into our sinks synchronously; a sink that finds the
//! state busy does nothing rather than panic.

use std::cell::RefCell;
use std::collections::HashSet;

use squareinput::{Engine, Key};
use windows::Win32::Foundation::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::TextServices::*;
use windows::core::*;

use crate::sys::{self, CommitSession, CommitSession_Impl, TextService, TextService_Impl};
use crate::window::{CandidateWindow, Content};
use crate::{data, guard};

/// What a numpad key does.
#[derive(Clone, Copy)]
enum Pad {
    Engine(Key),
    /// Numpad `/`: switch between 字形 input and plain digits.
    ToggleDigits,
    /// Numpad `*`: move the window between the bottom-right and top-right corners.
    MoveWindow,
}

fn pad_key(vk: u16) -> Option<Pad> {
    Some(match VIRTUAL_KEY(vk) {
        k if (VK_NUMPAD0.0..=VK_NUMPAD9.0).contains(&k.0) => Pad::Engine(Key::Digit((k.0 - VK_NUMPAD0.0) as u8)),
        VK_DECIMAL => Pad::Engine(Key::Dot),
        VK_SUBTRACT => Pad::Engine(Key::Minus),
        VK_ADD => Pad::Engine(Key::Plus),
        VK_DIVIDE => Pad::ToggleDigits,
        VK_MULTIPLY => Pad::MoveWindow,
        _ => return None,
    })
}

fn chord_held() -> bool {
    [VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN].into_iter().any(sys::key_held)
}

#[derive(Default)]
pub(crate) struct State {
    thread_mgr: Option<ITfThreadMgr>,
    client_id: u32,
    sink_cookies: Vec<u32>,
    /// Created on first use; stays None (inert) if the data cannot be loaded.
    engine: Option<Engine>,
    engine_failed: bool,
    window: Option<CandidateWindow>,
    digits: bool,
    /// A document of this thread has TSF focus.
    doc_focus: bool,
    /// We are the active keyboard text service.
    foreground: bool,
    /// This thread has the input focus.
    thread_focus: bool,
    /// Keys whose key-down we ate, so we eat their key-up too.
    eaten_down: HashSet<u16>,
}

impl State {
    fn engine(&mut self) -> Option<&mut Engine> {
        if self.engine.is_none() && !self.engine_failed {
            self.engine = data::new_engine();
            self.engine_failed = self.engine.is_none();
        }
        self.engine.as_mut()
    }

    /// The pad action if we would eat `vk`.
    fn decide(&mut self, vk: u16) -> Option<Pad> {
        if chord_held() {
            return None;
        }
        let pad = pad_key(vk)?;
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

    /// Show the window while the user can type into one of our documents.
    fn refresh(&mut self) {
        let active = self.doc_focus && self.foreground && self.thread_focus;
        if active && self.engine().is_some() {
            if self.window.is_none() {
                self.window = CandidateWindow::new();
            }
            let view = self.engine.as_ref().map(Engine::view);
            if let (Some(w), Some(view)) = (&self.window, view) {
                w.show(Content { view, digits: self.digits });
            }
        } else if let Some(w) = &self.window {
            w.hide();
        }
    }
}

impl TextService {
    pub fn new() -> TextService {
        TextService { state: RefCell::default() }
    }

    /// Run `f` on the state unless it is already borrowed further up the stack.
    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> Option<R> {
        self.state.try_borrow_mut().ok().map(|mut s| f(&mut s))
    }
}

impl ITfTextInputProcessor_Impl for TextService_Impl {
    fn Activate(&self, ptim: Ref<ITfThreadMgr>, tid: u32) -> Result<()> {
        self.ActivateEx(ptim, tid, 0)
    }

    fn Deactivate(&self) -> Result<()> {
        guard(Ok(()), || {
            let taken = self.with(|s| (s.thread_mgr.take(), s.client_id, std::mem::take(&mut s.sink_cookies)));
            if let Some((Some(tm), tid, cookies)) = taken {
                if let Ok(keystrokes) = tm.cast::<ITfKeystrokeMgr>() {
                    sys::unadvise_key_sink(&keystrokes, tid);
                }
                if let Ok(source) = tm.cast::<ITfSource>() {
                    for cookie in cookies {
                        sys::unadvise_sink(&source, cookie);
                    }
                }
            }
            self.with(|s| {
                s.window = None;
                s.engine = None;
                s.doc_focus = false;
            });
            Ok(())
        })
    }
}

impl ITfTextInputProcessorEx_Impl for TextService_Impl {
    fn ActivateEx(&self, ptim: Ref<ITfThreadMgr>, tid: u32, flags: u32) -> Result<()> {
        let result = guard(Err(E_UNEXPECTED.into()), || self.activate(ptim, tid));
        match &result {
            Ok(()) => data::debug(|| format!("activated: tid={tid} flags={flags:#x}")),
            Err(e) => data::log(&format!("activation failed: {e}")),
        }
        result
    }
}

impl TextService_Impl {
    fn activate(&self, ptim: Ref<ITfThreadMgr>, tid: u32) -> Result<()> {
        let tm: ITfThreadMgr = ptim.ok()?.clone();
        self.with(|s| {
            s.thread_mgr = Some(tm.clone());
            s.client_id = tid;
            s.foreground = true;
            s.thread_focus = true;
        });
        let source: ITfSource = tm.cast()?;
        let cookies = vec![
            sys::advise_sink(&source, &self.to_interface::<ITfThreadMgrEventSink>())?,
            sys::advise_sink(&source, &self.to_interface::<ITfThreadFocusSink>())?,
        ];
        sys::advise_key_sink(&tm.cast()?, tid, &self.to_interface())?;
        let doc_focus = sys::has_focus_document(&tm);
        self.with(|s| {
            s.sink_cookies = cookies;
            s.doc_focus = doc_focus;
            s.refresh();
        });
        Ok(())
    }
}

impl ITfKeyEventSink_Impl for TextService_Impl {
    fn OnSetFocus(&self, foreground: BOOL) -> Result<()> {
        guard(Ok(()), || {
            self.with(|s| {
                s.foreground = foreground.as_bool();
                s.reset();
                s.refresh();
            });
            Ok(())
        })
    }

    fn OnTestKeyDown(&self, _pic: Ref<ITfContext>, wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        guard(Ok(FALSE), || {
            let eat = self.with(|s| s.decide(wparam.0 as u16).is_some());
            data::debug(|| format!("OnTestKeyDown vk={:#x} -> {eat:?}", wparam.0));
            Ok(eat.unwrap_or(false).into())
        })
    }

    fn OnKeyDown(&self, pic: Ref<ITfContext>, wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        guard(Ok(FALSE), || {
            let vk = wparam.0 as u16;
            let step = self.with(|s| {
                let pad = s.decide(vk)?;
                s.eaten_down.insert(vk);
                let commit = match pad {
                    Pad::ToggleDigits => {
                        s.digits = !s.digits;
                        s.reset();
                        None
                    }
                    Pad::MoveWindow => {
                        crate::window::toggle_corner();
                        None
                    }
                    Pad::Engine(k) => {
                        let engine = s.engine()?;
                        let outcome = engine.key(k);
                        if outcome.learned
                            && let Some(line) = engine.promotions().last_line()
                        {
                            data::save_promotion(&line);
                        }
                        outcome.commit
                    }
                };
                s.refresh();
                Some((commit, s.client_id))
            });
            let Some(Some((commit, tid))) = step else { return Ok(FALSE) };
            if let (Some(text), Ok(context)) = (commit, pic.ok()) {
                commit_text(context, tid, &text);
            }
            Ok(TRUE)
        })
    }

    fn OnTestKeyUp(&self, _pic: Ref<ITfContext>, wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        guard(Ok(FALSE), || Ok(self.with(|s| s.eaten_down.contains(&(wparam.0 as u16))).unwrap_or(false).into()))
    }

    fn OnKeyUp(&self, _pic: Ref<ITfContext>, wparam: WPARAM, _lparam: LPARAM) -> Result<BOOL> {
        guard(Ok(FALSE), || Ok(self.with(|s| s.eaten_down.remove(&(wparam.0 as u16))).unwrap_or(false).into()))
    }

    fn OnPreservedKey(&self, _pic: Ref<ITfContext>, _guid: *const GUID) -> Result<BOOL> {
        Ok(FALSE)
    }
}

impl ITfThreadMgrEventSink_Impl for TextService_Impl {
    fn OnInitDocumentMgr(&self, _dim: Ref<ITfDocumentMgr>) -> Result<()> {
        Ok(())
    }

    fn OnUninitDocumentMgr(&self, _dim: Ref<ITfDocumentMgr>) -> Result<()> {
        Ok(())
    }

    fn OnSetFocus(&self, focus: Ref<ITfDocumentMgr>, _previous: Ref<ITfDocumentMgr>) -> Result<()> {
        guard(Ok(()), || {
            let has_focus = focus.ok().is_ok();
            self.with(|s| {
                s.doc_focus = has_focus;
                s.reset();
                s.refresh();
            });
            Ok(())
        })
    }

    fn OnPushContext(&self, _context: Ref<ITfContext>) -> Result<()> {
        Ok(())
    }

    fn OnPopContext(&self, _context: Ref<ITfContext>) -> Result<()> {
        Ok(())
    }
}

impl ITfThreadFocusSink_Impl for TextService_Impl {
    fn OnSetThreadFocus(&self) -> Result<()> {
        guard(Ok(()), || {
            self.with(|s| {
                s.thread_focus = true;
                s.refresh();
            });
            Ok(())
        })
    }

    fn OnKillThreadFocus(&self) -> Result<()> {
        guard(Ok(()), || {
            self.with(|s| {
                s.thread_focus = false;
                s.reset();
                s.refresh();
            });
            Ok(())
        })
    }
}

fn commit_text(context: &ITfContext, tid: u32, text: &str) {
    let session: ITfEditSession = CommitSession { context: context.clone(), text: text.encode_utf16().collect() }.into();
    match sys::request_edit_session(context, tid, &session) {
        Ok(hr) if hr.is_ok() => {}
        Ok(hr) => data::log(&format!("edit session failed: {hr:?}")),
        Err(e) => data::log(&format!("cannot request an edit session: {e}")),
    }
}

impl ITfEditSession_Impl for CommitSession_Impl {
    fn DoEditSession(&self, ec: u32) -> Result<()> {
        guard(Err(E_UNEXPECTED.into()), || sys::insert_text(&self.context, ec, &self.text))
    }
}
