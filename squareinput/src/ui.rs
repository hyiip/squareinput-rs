//! The candidate window's look, shared by every platform: [`paint`] turns a
//! [`View`] into a list of fills, frames and texts in layout units (pixels at
//! 96 dpi on Windows, points on macOS), which each platform draws with its own
//! graphics API.
//!
//! The 3×3 grid mirrors the numpad: keys 7-9 on top, 1-3 at the bottom, then
//! the `0` and `.` keys, two hint lines and a status line.

use crate::engine::{Cell, View};
use crate::labels::{COMPONENTS, FEATURES};

const PAD: i32 = 6;
const CELL: i32 = 62;
const BOTTOM: i32 = 34;
const LINE: i32 = 19;
pub const WIDTH: i32 = PAD * 2 + CELL * 3;
pub const HEIGHT: i32 = PAD + CELL * 3 + BOTTOM + PAD + LINE * 3 + PAD;
/// Gap between the window and the screen edges.
pub const MARGIN: i32 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

const BACKGROUND: Rgb = Rgb(0xEC, 0xEF, 0xF3);
const CELL_FILL: Rgb = Rgb(0xFF, 0xFF, 0xFF);
const BORDER: Rgb = Rgb(0xBA, 0xC0, 0xC8);
const INK: Rgb = Rgb(0x20, 0x20, 0x20);
const MUTED: Rgb = Rgb(0x66, 0x6A, 0x70);
const ACCENT: Rgb = Rgb(0x1E, 0x5A, 0xB0);

/// A rectangle in layout units, from the top-left corner of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

const fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
    Rect { x, y, w, h }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Font {
    /// For characters: one with HKSCS coverage.
    Han,
    /// For labels and hints.
    Ui,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    /// Centred both ways.
    Center,
    /// Left, vertically centred, cut with an ellipsis if too long.
    Left,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Fill(Rect, Rgb),
    /// A one-unit outline just inside the rectangle.
    Frame(Rect, Rgb),
    Text { text: String, rect: Rect, size: i32, font: Font, color: Rgb, align: Align },
}

fn text(items: &mut Vec<Item>, text: &str, rect: Rect, size: i32, font: Font, color: Rgb, align: Align) {
    if !text.is_empty() {
        items.push(Item::Text { text: text.to_owned(), rect, size, font, color, align });
    }
}

/// Everything to draw for `view`, in order. With `digits` (numpad digits pass
/// straight through) the grid is shown empty.
pub fn paint(view: &View, digits: bool) -> Vec<Item> {
    let mut items = vec![Item::Fill(rect(0, 0, WIDTH, HEIGHT), BACKGROUND)];
    let it = &mut items;

    for key in 1..=9u8 {
        let i = usize::from(key - 1);
        let (col, row) = (i as i32 % 3, 2 - i as i32 / 3);
        let (x, y) = (PAD + col * CELL, PAD + row * CELL);
        let r = rect(x, y, CELL, CELL);
        it.push(Item::Fill(r, CELL_FILL));
        it.push(Item::Frame(r, BORDER));
        if digits {
            continue;
        }
        match &view.cells[i] {
            Cell::Empty => {}
            Cell::Text(t) => text(it, t, r, if t.chars().count() > 1 { 22 } else { 34 }, Font::Han, INK, Align::Center),
            Cell::Feature(f) => text(it, FEATURES[usize::from(*f - 1)], r, 32, Font::Han, INK, Align::Center),
            Cell::Component { feature, key } => {
                let (glyph, examples) = COMPONENTS[usize::from(*feature - 1)][usize::from(*key - 1)];
                text(it, glyph, rect(x, y + 2, CELL, CELL - 18), 28, Font::Han, INK, Align::Center);
                text(it, examples, rect(x + 2, y + CELL - 18, CELL - 4, 16), 11, Font::Ui, MUTED, Align::Center);
            }
            Cell::More => text(it, "其他", r, 20, Font::Ui, ACCENT, Align::Center),
        }
    }

    let y = PAD + CELL * 3;
    let zero = rect(PAD, y, CELL * 2, BOTTOM);
    let dot = rect(PAD + CELL * 2, y, CELL, BOTTOM);
    for r in [zero, dot] {
        it.push(Item::Fill(r, CELL_FILL));
        it.push(Item::Frame(r, BORDER));
    }
    let (zero_label, dot_label) = if digits { ("", "") } else { (view.zero_label, view.dot_label) };
    text(it, zero_label, zero, 17, Font::Ui, INK, Align::Center);
    text(it, dot_label, dot, 15, Font::Ui, INK, Align::Center);

    let y = y + BOTTOM + PAD;
    let line = |n: i32| rect(PAD + 2, y + n * LINE, WIDTH - PAD * 2 - 4, LINE);
    let hints = if digits { ["數字模式", "按 / 返回中文"] } else { view.hints };
    text(it, hints[0], line(0), 13, Font::Ui, INK, Align::Left);
    text(it, hints[1], line(1), 13, Font::Ui, MUTED, Align::Left);

    let mut status = vec![if digits { "數字".to_owned() } else { "中".to_owned() }];
    if view.homophone_armed {
        status.push("同音".into());
    }
    if !view.typed.is_empty() {
        status.push(view.typed.clone());
    }
    if view.pages > 1 {
        status.push(format!("{}/{} 頁", view.page + 1, view.pages));
    }
    text(it, &status.join("　"), line(2), 12, Font::Ui, ACCENT, Align::Left);
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Screen;

    fn view() -> View {
        let mut cells: [Cell; 9] = std::array::from_fn(|i| Cell::Feature(i as u8 + 1));
        cells[0] = Cell::Text("快".into());
        cells[1] = Cell::Empty;
        View {
            screen: Screen::Candidates,
            cells,
            zero_label: "下　頁",
            dot_label: "取消",
            hints: ["直接選字", ""],
            page: 0,
            pages: 2,
            typed: "892".into(),
            homophone_armed: false,
        }
    }

    fn texts(items: &[Item]) -> Vec<(&str, Rect)> {
        items.iter().filter_map(|i| if let Item::Text { text, rect, .. } = i { Some((text.as_str(), *rect)) } else { None }).collect()
    }

    #[test]
    fn grid_mirrors_the_numpad() {
        let items = paint(&view(), false);
        let t = texts(&items);
        // Key 1 is bottom left; key 7 (feature 7, 一) is top left.
        let (_, key1) = t.iter().find(|(s, _)| *s == "快").unwrap();
        let (_, key7) = t.iter().find(|(s, _)| *s == FEATURES[6]).unwrap();
        assert_eq!((key1.x, key1.y), (PAD, PAD + 2 * CELL));
        assert_eq!((key7.x, key7.y), (PAD, PAD));
        // Empty cells draw no text; empty hints are skipped.
        assert_eq!(t.iter().filter(|(s, _)| s.is_empty()).count(), 0);
        assert!(t.iter().any(|(s, _)| *s == "中　892　1/2 頁"));
    }

    #[test]
    fn digit_mode_shows_an_empty_grid() {
        let items = paint(&view(), true);
        let t: Vec<&str> = texts(&items).into_iter().map(|(s, _)| s).collect();
        assert_eq!(t, ["數字模式", "按 / 返回中文", "數字　892　1/2 頁"]);
    }
}
