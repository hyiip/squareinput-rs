//! The 3×3 candidate window: a topmost popup that never takes focus, drawn
//! with GDI and parked in a right-hand corner of the work area of the monitor
//! the user is typing on. It sits at a fixed spot instead of
//! following the caret, and the grid mirrors the numpad: keys 7-9 on top, 1-3
//! at the bottom. Numpad `*` moves it between the bottom-right and top-right
//! corners; the choice is saved and shared by all applications.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use squareinput::{Cell, View};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::labels::{COMPONENTS, FEATURES};
use crate::sys::{self, Canvas};

// Layout in 96-dpi pixels.
const PAD: i32 = 6;
const CELL: i32 = 62;
const BOTTOM: i32 = 34;
const LINE: i32 = 19;
const WIDTH: i32 = PAD * 2 + CELL * 3;
const HEIGHT: i32 = PAD + CELL * 3 + BOTTOM + PAD + LINE * 3 + PAD;
const MARGIN: i32 = 12;

const BACKGROUND: COLORREF = COLORREF(0x00F3_EFEC);
const CELL_FILL: COLORREF = COLORREF(0x00FF_FFFF);
const BORDER: COLORREF = COLORREF(0x00C8_C0BA);
const INK: COLORREF = COLORREF(0x0020_2020);
const MUTED: COLORREF = COLORREF(0x0070_6A66);
const ACCENT: COLORREF = COLORREF(0x00B0_5A1E);

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
                        draw(&Painter { canvas }, content);
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

/// Draws in 96-dpi layout units on a canvas.
struct Painter<'a> {
    canvas: &'a Canvas,
}

impl Painter<'_> {
    fn px(&self, v: i32) -> i32 {
        (v as f32 * self.canvas.scale).round() as i32
    }

    fn rect(&self, x: i32, y: i32, w: i32, h: i32) -> RECT {
        RECT { left: self.px(x), top: self.px(y), right: self.px(x + w), bottom: self.px(y + h) }
    }

    fn fill(&self, r: &RECT, color: COLORREF) {
        self.canvas.fill(r, color);
    }

    fn frame(&self, r: &RECT, color: COLORREF) {
        self.canvas.frame(r, color);
    }

    fn text(&self, text: &str, r: &RECT, size: i32, face: &str, color: COLORREF, format: DRAW_TEXT_FORMAT) {
        self.canvas.text(text, r, self.px(size), face, color, format);
    }
}

fn draw(p: &Painter, content: &Content) {
    let v = &content.view;
    p.fill(&p.rect(0, 0, WIDTH, HEIGHT), BACKGROUND);

    for key in 1..=9u8 {
        let i = usize::from(key - 1);
        let (col, row) = (i as i32 % 3, 2 - i as i32 / 3);
        let (x, y) = (PAD + col * CELL, PAD + row * CELL);
        let r = p.rect(x, y, CELL, CELL);
        p.fill(&r, CELL_FILL);
        p.frame(&r, BORDER);
        if content.digits {
            continue;
        }
        match &v.cells[i] {
            Cell::Empty => {}
            Cell::Text(t) => p.text(t, &r, if t.chars().count() > 1 { 22 } else { 34 }, HAN_FONT, INK, CENTERED),
            Cell::Feature(f) => p.text(FEATURES[usize::from(*f - 1)], &r, 32, HAN_FONT, INK, CENTERED),
            Cell::Component { feature, key } => {
                let (glyph, examples) = COMPONENTS[usize::from(*feature - 1)][usize::from(*key - 1)];
                p.text(glyph, &p.rect(x, y + 2, CELL, CELL - 18), 28, HAN_FONT, INK, CENTERED);
                p.text(examples, &p.rect(x + 2, y + CELL - 18, CELL - 4, 16), 11, UI_FONT, MUTED, CENTERED);
            }
            Cell::More => p.text("其他", &r, 20, UI_FONT, ACCENT, CENTERED),
        }
    }

    let y = PAD + CELL * 3;
    let zero = p.rect(PAD, y, CELL * 2, BOTTOM);
    let dot = p.rect(PAD + CELL * 2, y, CELL, BOTTOM);
    for r in [&zero, &dot] {
        p.fill(r, CELL_FILL);
        p.frame(r, BORDER);
    }
    let (zero_label, dot_label) = if content.digits { ("", "") } else { (v.zero_label, v.dot_label) };
    p.text(zero_label, &zero, 17, UI_FONT, INK, CENTERED);
    p.text(dot_label, &dot, 15, UI_FONT, INK, CENTERED);

    let y = y + BOTTOM + PAD;
    let line = |n: i32| p.rect(PAD + 2, y + n * LINE, WIDTH - PAD * 2 - 4, LINE);
    let hints = if content.digits { ["數字模式", "按 / 返回中文"] } else { v.hints };
    p.text(hints[0], &line(0), 13, UI_FONT, INK, LEFT);
    p.text(hints[1], &line(1), 13, UI_FONT, MUTED, LEFT);

    let mut status = vec![if content.digits { "數字".to_owned() } else { "中".to_owned() }];
    if v.homophone_armed {
        status.push("同音".into());
    }
    if !v.typed.is_empty() {
        status.push(v.typed.clone());
    }
    if v.pages > 1 {
        status.push(format!("{}/{} 頁", v.page + 1, v.pages));
    }
    p.text(&status.join("　"), &line(2), 12, UI_FONT, ACCENT, LEFT);
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
