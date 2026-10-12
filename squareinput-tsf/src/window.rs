//! The 3×3 candidate window: a topmost popup that never takes focus, drawn
//! with GDI and parked in a right-hand corner of the work area of the monitor
//! the user is typing on. It sits at a fixed spot instead of following the
//! caret; its contents and layout come from [`squareinput::ui`]. Numpad `*`
//! moves it between the bottom-right and top-right corners; the choice is
//! saved and shared by all applications.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use squareinput::View;
use squareinput::ui::{self, Align, Font, HEIGHT, Item, MARGIN, Rgb, WIDTH};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::sys::{self, Canvas};

const HAN_FONT: &str = "MingLiU_HKSCS";
const UI_FONT: &str = "Microsoft JhengHei UI";
const CENTERED: DRAW_TEXT_FORMAT = DRAW_TEXT_FORMAT(DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0);
const LEFT: DRAW_TEXT_FORMAT = DRAW_TEXT_FORMAT(DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_END_ELLIPSIS.0);

/// What to draw.
pub struct Content {
    pub view: View,
    /// Numpad digits pass straight through (toggled with numpad `/`).
    pub digits: bool,
}

thread_local! {
    /// What each candidate window of this thread shows, by window handle. A
    /// window and its procedure always run on the thread that created it, so
    /// a thread-local table needs no pointers stashed in the window itself.
    static CONTENT: RefCell<HashMap<isize, Content>> = RefCell::default();
}

fn key(hwnd: HWND) -> isize {
    hwnd.0 as isize
}

/// Setting file holding the corner ("top" or "bottom").
const CORNER_SETTING: &str = "corner";
static AT_TOP: AtomicBool = AtomicBool::new(false);

/// Re-read the saved corner, which another application may have changed.
fn load_corner() {
    AT_TOP.store(crate::data::read_setting(CORNER_SETTING).as_deref() == Some("top"), Ordering::Relaxed);
}

/// Move the window to the other right-hand corner and remember it.
pub fn toggle_corner() {
    let top = !AT_TOP.load(Ordering::Relaxed);
    AT_TOP.store(top, Ordering::Relaxed);
    crate::data::write_setting(CORNER_SETTING, if top { "top" } else { "bottom" });
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let painted = crate::guard(false, || {
                sys::paint_buffered(hwnd, |canvas| {
                    CONTENT.with(|c| {
                        let Ok(map) = c.try_borrow() else { return false };
                        let Some(content) = map.get(&key(hwnd)) else { return false };
                        draw(canvas, content);
                        true
                    })
                });
                true
            });
            if !painted {
                sys::validate(hwnd);
            }
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_NCDESTROY => {
            CONTENT.with(|c| {
                if let Ok(mut map) = c.try_borrow_mut() {
                    map.remove(&key(hwnd));
                }
            });
            sys::default_window_proc(hwnd, msg, wp, lp)
        }
        _ => sys::default_window_proc(hwnd, msg, wp, lp),
    }
}

fn colorref(Rgb(r, g, b): Rgb) -> COLORREF {
    COLORREF(u32::from(r) | u32::from(g) << 8 | u32::from(b) << 16)
}

/// Draw [`ui::paint`]'s items, scaling its 96-dpi layout units to the monitor.
fn draw(canvas: &Canvas, content: &Content) {
    let px = |v: i32| (v as f32 * canvas.scale).round() as i32;
    let rect = |r: ui::Rect| RECT { left: px(r.x), top: px(r.y), right: px(r.x + r.w), bottom: px(r.y + r.h) };
    for item in ui::paint(&content.view, content.digits) {
        match item {
            Item::Fill(r, color) => canvas.fill(&rect(r), colorref(color)),
            Item::Frame(r, color) => canvas.frame(&rect(r), colorref(color)),
            Item::Text { text, rect: r, size, font, color, align } => {
                let face = match font {
                    Font::Han => HAN_FONT,
                    Font::Ui => UI_FONT,
                };
                let format = match align {
                    Align::Center => CENTERED,
                    Align::Left => LEFT,
                };
                canvas.text(&text, &rect(r), px(size), face, colorref(color), format);
            }
        }
    }
}

pub struct CandidateWindow {
    hwnd: HWND,
}

impl CandidateWindow {
    pub fn new() -> Option<CandidateWindow> {
        sys::register_window_class(wndproc);
        let hwnd = sys::create_popup("squareinput", WIDTH, HEIGHT)
            .inspect_err(|e| crate::data::log(&format!("cannot create the candidate window: {e}")))
            .ok()?;
        Some(CandidateWindow { hwnd })
    }

    /// Show `content` in the chosen corner of the monitor the user is typing on.
    pub fn show(&self, content: Content) {
        CONTENT.with(|c| {
            if let Ok(mut map) = c.try_borrow_mut() {
                map.insert(key(self.hwnd), content);
            }
        });
        if !sys::is_visible(self.hwnd) {
            load_corner();
        }
        if let Some(work) = sys::foreground_work_area() {
            let scale = sys::dpi_scale(self.hwnd);
            let px = |v: i32| (v as f32 * scale).round() as i32;
            let (w, h) = (px(WIDTH), px(HEIGHT));
            let y = if AT_TOP.load(Ordering::Relaxed) { work.top + px(MARGIN) } else { work.bottom - h - px(MARGIN) };
            sys::place_topmost(self.hwnd, work.right - w - px(MARGIN), y, w, h);
        }
        sys::invalidate(self.hwnd);
    }

    pub fn hide(&self) {
        sys::hide(self.hwnd);
    }
}

impl Drop for CandidateWindow {
    fn drop(&mut self) {
        sys::destroy(self.hwnd);
    }
}
