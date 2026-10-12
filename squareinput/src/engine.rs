//! The key state machine of the 字形 mode.
//! Only numpad keys exist here: the input method is meant to be used with the
//! right hand alone.
//!
//! - Idle: 1-9 picks a left feature. `.` opens punctuation. `0` also opens
//!   punctuation while nothing has been typed yet ("fresh"); afterwards it
//!   picks from the nine common characters on the idle grid.
//! - Feature: 1-9 picks a component.
//! - Component: `0` completes a 獨體 code; 1-9 picks the right feature of a 分體 code.
//! - Candidates: 1-9 commits, `0` is next page, `-` previous page.
//! - `.` cancels to idle from any other screen, and disarms 同音字.
//! - Punctuation page 1, key 9 (其他) opens a menu: 2 (音) arms 同音字 for the
//!   next character, 4-9 open symbol lists. 1 and 3 (劃, 部) are not supported.
//! - With 同音字 armed, picking a character shows its homophones instead of
//!   committing it; picking one of those commits it.
//!
//! 姓氏 (`0` after a feature), 字形檢字 (`-`) and 詞輸入 (`+`) are not supported.
//! While composing, any key without a meaning is swallowed so it cannot leak
//! into the application mid-code; while idle, such keys pass through.

use std::sync::Arc;

use crate::homophone::Homophones;
use crate::learn::Promotions;
use crate::table::{Code, CodeTable, PAGE, SymbolTable};

/// The common characters `0` offers once something has been typed, by key 1-9.
pub const COMMON_CHARS: [&str; 9] = ["個", "能", "的", "到", "資", "就", "你", "這", "好"];

const PUNCTUATION: &str = ".";
/// Punctuation page 1, key 9 opens the 其他 menu.
const MORE_SLOT: usize = 8;
/// 其他 menu by key 1-9; 劃 (1) and 部 (3) lookups are not supported.
const MENU: [&str; 9] = ["", "音", "", "ㄅ", "Ａ", "﹡", "├", "１", "()"];
const MENU_HOMOPHONES: u8 = 2;

/// A numpad key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// 0-9.
    Digit(u8),
    Dot,
    Minus,
    Plus,
}

/// What a grid cell shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cell {
    Empty,
    Text(String),
    /// One of the nine features (一丨丿 / 丶乙口 / 十乂冂 on keys 7-9 / 4-6 / 1-3),
    /// drawn by the UI.
    Feature(u8),
    /// Component `key` of `feature`, drawn by the UI.
    Component { feature: u8, key: u8 },
    /// 其他: opens the symbol menu.
    More,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Idle,
    Feature,
    Component,
    Candidates,
    NoCandidates,
    Punctuation,
    Menu,
    Symbols,
    Homophones,
    Common,
}

/// Everything the candidate window needs to draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub screen: Screen,
    /// `cells[k - 1]` is shown on numpad key k.
    pub cells: [Cell; 9],
    pub zero_label: &'static str,
    pub dot_label: &'static str,
    pub hints: [&'static str; 2],
    /// 0-based current page and page count.
    pub page: usize,
    pub pages: usize,
    /// Code keys typed so far, e.g. "89".
    pub typed: String,
    pub homophone_armed: bool,
}

/// The result of one key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The engine used the key; it must not reach the application.
    pub eaten: bool,
    /// Text to insert into the document.
    pub commit: Option<String>,
    /// 自動調前 moved a character; append [`Promotions::last_line`] to the saved log.
    pub learned: bool,
}

impl Outcome {
    fn eaten() -> Outcome {
        Outcome { eaten: true, ..Outcome::default() }
    }

    fn passed() -> Outcome {
        Outcome::default()
    }

    fn commit(text: String, learned: bool) -> Outcome {
        Outcome { eaten: true, commit: Some(text), learned }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum State {
    Idle,
    Feature(u8),
    Component(u8, u8),
    Candidates { code: Code, page: usize },
    NoCandidates(Code),
    Punctuation { page: usize },
    Menu,
    Symbols { list: u8, page: usize },
    Homophones { slots: Vec<Option<String>>, page: usize },
    Common,
}

fn page_count(len: usize) -> usize {
    len.div_ceil(PAGE).max(1)
}

fn turn(page: usize, len: usize, forward: bool) -> usize {
    let n = page_count(len);
    if forward { (page + 1) % n } else { (page + n - 1) % n }
}

fn symbol_list(list: u8) -> String {
    format!(".9{list}")
}

pub struct Engine {
    table: CodeTable,
    symbols: Arc<SymbolTable>,
    homophones: Arc<Homophones>,
    promotions: Promotions,
    state: State,
    fresh: bool,
    armed: bool,
}

impl Engine {
    pub fn new(table: CodeTable, symbols: impl Into<Arc<SymbolTable>>) -> Engine {
        Engine {
            table,
            symbols: symbols.into(),
            homophones: Arc::default(),
            promotions: Promotions::default(),
            state: State::Idle,
            fresh: true,
            armed: false,
        }
    }

    pub fn with_homophones(mut self, homophones: impl Into<Arc<Homophones>>) -> Engine {
        self.homophones = homophones.into();
        self
    }

    /// Restore a saved 自動調前 log, replaying it onto the table.
    pub fn with_promotions(mut self, promotions: Promotions) -> Engine {
        promotions.apply(&mut self.table);
        self.promotions = promotions;
        self
    }

    pub fn promotions(&self) -> &Promotions {
        &self.promotions
    }

    pub fn is_composing(&self) -> bool {
        self.state != State::Idle
    }

    /// Whether [`Engine::key`] would eat `key`, without changing anything.
    pub fn wants(&self, key: Key) -> bool {
        self.is_composing() || matches!(key, Key::Digit(_) | Key::Dot)
    }

    /// Abandon any composition, e.g. when focus moves to another field.
    pub fn reset(&mut self) {
        self.state = State::Idle;
        self.armed = false;
    }

    pub fn key(&mut self, key: Key) -> Outcome {
        let state = std::mem::replace(&mut self.state, State::Idle);
        let (next, outcome) = self.step(state, key);
        if outcome.eaten {
            self.fresh = false;
        }
        self.state = next;
        outcome
    }

    fn step(&mut self, state: State, key: Key) -> (State, Outcome) {
        use Key::*;
        use State as S;
        let stay = |s: State| (s, Outcome::eaten());
        match (state, key) {
            (S::Idle, Digit(f @ 1..=9)) => stay(S::Feature(f)),
            (S::Idle, Digit(0)) if self.fresh => stay(S::Punctuation { page: 0 }),
            (S::Idle, Digit(0)) => stay(S::Common),
            (S::Idle, Dot) => stay(S::Punctuation { page: 0 }),
            (S::Idle, _) => (S::Idle, Outcome::passed()),

            (_, Dot) => {
                self.armed = false;
                stay(S::Idle)
            }

            (S::Feature(f), Digit(c @ 1..=9)) => stay(S::Component(f, c)),

            (S::Component(f, c), Digit(r)) => stay(self.open_code(Code([f, c, r]))),

            (S::Candidates { code, page }, Digit(k @ 1..=9)) => self.pick_candidate(code, page, k),
            (S::Candidates { code, page }, Digit(0) | Minus) => {
                let len = self.table.candidates(code).len();
                stay(S::Candidates { code, page: turn(page, len, key == Digit(0)) })
            }

            (S::NoCandidates(_), Digit(0)) => stay(S::Punctuation { page: 0 }),

            (S::Punctuation { page: 0 }, Digit(9)) => stay(S::Menu),
            (S::Punctuation { page }, Digit(k @ 1..=9)) => {
                self.pick_symbol(PUNCTUATION, page, k).unwrap_or_else(|| stay(S::Punctuation { page }))
            }
            (S::Punctuation { page }, Digit(0) | Minus) => {
                let len = self.symbols.slots(PUNCTUATION).len();
                stay(S::Punctuation { page: turn(page, len, key == Digit(0)) })
            }

            (S::Menu, Digit(MENU_HOMOPHONES)) => {
                self.armed = true;
                stay(S::Idle)
            }
            (S::Menu, Digit(list @ 4..=9)) => stay(S::Symbols { list, page: 0 }),

            (S::Symbols { list, page }, Digit(k @ 1..=9)) => {
                self.pick_symbol(&symbol_list(list), page, k).unwrap_or_else(|| stay(S::Symbols { list, page }))
            }
            (S::Symbols { list, page }, Digit(0) | Minus) => {
                let len = self.symbols.slots(&symbol_list(list)).len();
                stay(S::Symbols { list, page: turn(page, len, key == Digit(0)) })
            }

            (S::Homophones { slots, page }, Digit(k @ 1..=9)) => {
                match slots.get(page * PAGE + k as usize - 1).cloned().flatten() {
                    Some(text) => (S::Idle, Outcome::commit(text, false)),
                    None => stay(S::Homophones { slots, page }),
                }
            }
            (S::Homophones { slots, page }, Digit(0) | Minus) => {
                let page = turn(page, slots.len(), key == Digit(0));
                stay(S::Homophones { slots, page })
            }

            (S::Common, Digit(k @ 1..=9)) => self.pick_char(COMMON_CHARS[k as usize - 1].to_owned()),

            (state, _) => stay(state),
        }
    }

    fn open_code(&self, code: Code) -> State {
        if self.table.candidates(code).is_empty() {
            State::NoCandidates(code)
        } else {
            State::Candidates { code, page: 0 }
        }
    }

    fn pick_candidate(&mut self, code: Code, page: usize, k: u8) -> (State, Outcome) {
        let index = page * PAGE + k as usize - 1;
        let Some(text) = self.table.candidates(code).get(index).cloned() else {
            return (State::Candidates { code, page }, Outcome::eaten());
        };
        if self.armed {
            return self.pick_char(text);
        }
        let learned = self.promotions.record(&mut self.table, code, index);
        (State::Idle, Outcome::commit(text, learned))
    }

    /// Commit `text`, or show its homophones if 同音字 is armed.
    fn pick_char(&mut self, text: String) -> (State, Outcome) {
        if std::mem::take(&mut self.armed) {
            let slots = self.homophones.slots(&text);
            if !slots.is_empty() {
                return (State::Homophones { slots, page: 0 }, Outcome::eaten());
            }
        }
        (State::Idle, Outcome::commit(text, false))
    }

    fn pick_symbol(&self, list: &str, page: usize, k: u8) -> Option<(State, Outcome)> {
        let text = self.symbols.slots(list).get(page * PAGE + k as usize - 1).cloned().flatten()?;
        Some((State::Idle, Outcome::commit(text, false)))
    }

    pub fn view(&self) -> View {
        use State as S;
        let text_cells = |slots: &[Option<String>], page: usize| -> [Cell; 9] {
            std::array::from_fn(|i| slots.get(page * PAGE + i).cloned().flatten().map_or(Cell::Empty, Cell::Text))
        };
        let fixed = |texts: [&str; 9]| -> [Cell; 9] {
            std::array::from_fn(|i| if texts[i].is_empty() { Cell::Empty } else { Cell::Text(texts[i].to_owned()) })
        };
        let features: [Cell; 9] = std::array::from_fn(|i| Cell::Feature(i as u8 + 1));
        const PICK: [&str; 2] = ["直接選字", "或按下頁繼續找尋"];

        let mut v = View {
            screen: Screen::Idle,
            cells: std::array::from_fn(|_| Cell::Empty),
            zero_label: "",
            dot_label: "取消",
            hints: ["", ""],
            page: 0,
            pages: 1,
            typed: String::new(),
            homophone_armed: self.armed,
        };
        match &self.state {
            S::Idle => {
                // COMMON_CHARS could stand here once something has been typed; the
                // user prefers the features, which say what 1-9 will do.
                v.cells = features;
                v.zero_label = if self.fresh { "標　點" } else { "選　字" };
                v.dot_label = "標點";
                v.hints = if self.armed {
                    ["同音字：打一個字", "顯示佢嘅同音字"]
                } else {
                    ["選字形的左上方之特徵", "按０輸入標點／檢字"]
                };
            }
            S::Feature(f) => {
                v.screen = Screen::Feature;
                v.cells = std::array::from_fn(|i| Cell::Component { feature: *f, key: i as u8 + 1 });
                v.hints = ["選部件", ""];
                v.typed = f.to_string();
            }
            S::Component(f, c) => {
                v.screen = Screen::Component;
                v.cells = features;
                v.zero_label = "確　定";
                v.hints = ["獨體字按０", "分體字選最右上之特徵"];
                v.typed = format!("{f}{c}");
            }
            S::Candidates { code, page } => {
                let list = self.table.candidates(*code);
                v.screen = Screen::Candidates;
                v.cells = std::array::from_fn(|i| list.get(page * PAGE + i).map_or(Cell::Empty, |t| Cell::Text(t.clone())));
                v.zero_label = "下　頁";
                v.hints = PICK;
                (v.page, v.pages) = (*page, page_count(list.len()));
                v.typed = code.to_string();
            }
            S::NoCandidates(code) => {
                v.screen = Screen::NoCandidates;
                v.zero_label = "標　點";
                v.hints = ["沒有中文字供選用", "請輸入其他字"];
                v.typed = code.to_string();
            }
            S::Punctuation { page } => {
                let slots = self.symbols.slots(PUNCTUATION);
                v.screen = Screen::Punctuation;
                v.cells = text_cells(slots, *page);
                if *page == 0 {
                    v.cells[MORE_SLOT] = Cell::More;
                }
                v.zero_label = "下　頁";
                v.hints = PICK;
                (v.page, v.pages) = (*page, page_count(slots.len()));
            }
            S::Menu => {
                v.screen = Screen::Menu;
                v.cells = fixed(MENU);
                v.hints = ["選檢字方法", ""];
            }
            S::Symbols { list, page } => {
                let slots = self.symbols.slots(&symbol_list(*list));
                v.screen = Screen::Symbols;
                v.cells = text_cells(slots, *page);
                v.zero_label = "下　頁";
                v.hints = PICK;
                (v.page, v.pages) = (*page, page_count(slots.len()));
            }
            S::Homophones { slots, page } => {
                v.screen = Screen::Homophones;
                v.cells = text_cells(slots, *page);
                v.zero_label = "下　頁";
                v.hints = ["選同音字", "或按下頁繼續找尋"];
                (v.page, v.pages) = (*page, page_count(slots.len()));
            }
            S::Common => {
                v.screen = Screen::Common;
                v.cells = fixed(COMMON_CHARS);
                v.hints = ["直接選字", ""];
            }
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(name: &str, items: &[&str]) -> String {
        items.iter().enumerate().map(|(i, t)| format!("t\t{name}\t{}\t{}\t{t}\n", i / PAGE + 1, i % PAGE + 1)).collect()
    }

    fn engine() -> Engine {
        let many: Vec<String> = (0..30).map(|i| format!("c{i}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        let mut tsv = rows("892", &["惰", "快", "懺", "憾", "怪", "恢", "忱", "怖", "慟", "憭", "惙"])
            + &rows("970", &["反", "身", "向", "愛", "看", "手", "我", "受", "乎"])
            + &rows("110", &many)
            + &rows("999", &["餓", "鵝"])
            + "t\t161\t-\t-\t\n"
            + &rows(".94", &["ㄅ", "ㄆ", "ㄇ"]);
        // Punctuation page 1 has no symbol on key 9 (其他).
        tsv += &rows(".", &["！", "：", "；", "。", "，", "、", "—", "？"]);
        tsv += "t\t.\t2\t1\t‘\nt\t.\t2\t2\t“\n";
        let table = CodeTable::from_tsv(&tsv, "t").unwrap();
        let symbols = SymbolTable::from_tsv(&tsv, "t").unwrap();
        // Without tiers, 餓 (slot 0) and 鵝 (slot 1) rank before 我 (slot 6).
        let unihan = "U+6211\tkCantonese\tngo5\nU+9913\tkCantonese\tngo6\nU+9D5D\tkCantonese\tngo4\n";
        let homophones = Homophones::from_compact(&crate::homophone::compile(&table, unihan, "", ""));
        Engine::new(table, symbols).with_homophones(homophones)
    }

    fn key_of(c: char) -> Key {
        match c {
            '0'..='9' => Key::Digit(c as u8 - b'0'),
            '.' => Key::Dot,
            '-' => Key::Minus,
            '+' => Key::Plus,
            _ => panic!("no key for {c:?}"),
        }
    }

    /// Press keys and return everything committed.
    fn type_keys(e: &mut Engine, keys: &str) -> String {
        keys.chars().filter_map(|c| e.key(key_of(c)).commit).collect()
    }

    #[test]
    fn fen_ti_code_commits_from_the_grid() {
        let mut e = engine();
        assert_eq!(type_keys(&mut e, "892"), "");
        let v = e.view();
        assert_eq!(v.screen, Screen::Candidates);
        assert_eq!(v.cells[1], Cell::Text("快".into()));
        assert_eq!((v.page, v.pages), (0, 2));
        assert_eq!(v.typed, "892");
        assert_eq!(type_keys(&mut e, "2"), "快");
        assert!(!e.is_composing());
    }

    #[test]
    fn du_ti_code_is_completed_with_zero() {
        let mut e = engine();
        type_keys(&mut e, "97");
        let v = e.view();
        assert_eq!((v.screen, v.zero_label), (Screen::Component, "確　定"));
        assert_eq!(v.cells[0], Cell::Feature(1));
        assert_eq!(type_keys(&mut e, "07"), "我");
    }

    #[test]
    fn feature_screen_shows_its_components() {
        let mut e = engine();
        type_keys(&mut e, "8");
        let v = e.view();
        assert_eq!(v.screen, Screen::Feature);
        assert_eq!(v.cells[6], Cell::Component { feature: 8, key: 7 });
    }

    #[test]
    fn paging_wraps_both_ways() {
        let mut e = engine();
        type_keys(&mut e, "8920");
        assert_eq!(e.view().cells[0], Cell::Text("憭".into()));
        assert_eq!(e.view().cells[2], Cell::Empty);
        type_keys(&mut e, "0");
        assert_eq!(e.view().page, 0);
        type_keys(&mut e, "-");
        assert_eq!(e.view().page, 1);
        // An empty slot on a short last page does nothing.
        assert_eq!(type_keys(&mut e, "9"), "");
        assert_eq!(type_keys(&mut e, "1"), "憭");
    }

    #[test]
    fn dot_cancels_from_any_stage() {
        let mut e = engine();
        for keys in ["8.", "89.", "892.", "8920."] {
            assert_eq!(type_keys(&mut e, keys), "", "{keys}");
            assert!(!e.is_composing(), "{keys}");
        }
    }

    #[test]
    fn code_without_candidates_offers_punctuation() {
        let mut e = engine();
        type_keys(&mut e, "161");
        let v = e.view();
        assert_eq!(v.screen, Screen::NoCandidates);
        assert_eq!(v.hints[0], "沒有中文字供選用");
        type_keys(&mut e, "0");
        assert_eq!(e.view().screen, Screen::Punctuation);
    }

    #[test]
    fn zero_opens_punctuation_only_while_fresh() {
        let mut e = engine();
        assert_eq!(e.view().cells[0], Cell::Feature(1));
        type_keys(&mut e, "0");
        assert_eq!(e.view().screen, Screen::Punctuation);
        type_keys(&mut e, ".");
        // The idle grid keeps showing the features after typing.
        assert_eq!(e.view().cells[6], Cell::Feature(7));
        assert_eq!(e.view().zero_label, "選　字");
        type_keys(&mut e, "0");
        assert_eq!(e.view().screen, Screen::Common);
        assert_eq!(e.view().cells[6], Cell::Text("你".into()));
        assert_eq!(type_keys(&mut e, "7"), "你");
    }

    #[test]
    fn punctuation_pages_and_symbol_menu() {
        let mut e = engine();
        assert_eq!(type_keys(&mut e, ".5"), "，");
        assert_eq!(type_keys(&mut e, ".01"), "‘");
        type_keys(&mut e, ".");
        assert_eq!(e.view().cells[8], Cell::More);
        type_keys(&mut e, "9");
        assert_eq!(e.view().screen, Screen::Menu);
        assert_eq!(e.view().cells[0], Cell::Empty);
        assert_eq!(type_keys(&mut e, "42"), "ㄆ");
        // Unsupported 劃 lookup does nothing.
        type_keys(&mut e, ".91");
        assert_eq!(e.view().screen, Screen::Menu);
    }

    #[test]
    fn homophones_replace_the_picked_character_once() {
        let mut e = engine();
        type_keys(&mut e, ".92");
        assert!(e.view().homophone_armed);
        assert_eq!(e.view().screen, Screen::Idle);
        assert_eq!(type_keys(&mut e, "9707"), "");
        let v = e.view();
        assert_eq!(v.screen, Screen::Homophones);
        assert_eq!(v.cells[..3], [Cell::Text("餓".into()), Cell::Text("鵝".into()), Cell::Text("我".into())]);
        assert!(!v.homophone_armed);
        assert_eq!(type_keys(&mut e, "1"), "餓");
        assert_eq!(type_keys(&mut e, "9707"), "我");
    }

    #[test]
    fn dot_disarms_homophones() {
        let mut e = engine();
        type_keys(&mut e, ".9289.");
        assert!(!e.view().homophone_armed);
        assert_eq!(type_keys(&mut e, "9707"), "我");
    }

    #[test]
    fn committing_from_page_three_promotes_to_page_two_key_one() {
        let mut e = engine();
        // Page 2 characters stay where they are.
        let out: Vec<Outcome> = "11002".chars().map(|c| e.key(key_of(c))).collect();
        assert_eq!(out.last().unwrap().commit.as_deref(), Some("c10"));
        assert!(!out.last().unwrap().learned);
        assert!(e.promotions().is_empty());

        let out: Vec<Outcome> = "110003".chars().map(|c| e.key(key_of(c))).collect();
        let last = out.last().unwrap();
        assert_eq!(last.commit.as_deref(), Some("c20"));
        assert!(last.learned);
        assert_eq!(e.promotions().last_line().as_deref(), Some("110\tc20\n"));
        type_keys(&mut e, "1100");
        assert_eq!(e.view().cells[0], Cell::Text("c20".into()));
        // Page 1 is untouched.
        assert_eq!(type_keys(&mut e, "-1"), "c0");
    }

    #[test]
    fn idle_passes_minus_and_plus_but_composing_swallows_them() {
        let mut e = engine();
        for k in [Key::Minus, Key::Plus] {
            assert!(!e.wants(k), "{k:?}");
            assert!(!e.key(k).eaten, "{k:?}");
        }
        assert!(e.wants(Key::Dot) && e.wants(Key::Digit(0)));
        type_keys(&mut e, "8");
        assert!(e.wants(Key::Plus));
        assert!(e.key(Key::Plus).eaten);
        assert_eq!(e.view().screen, Screen::Feature);
    }
}
