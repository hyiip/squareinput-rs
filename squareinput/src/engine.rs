//! The key state machine of the shape mode.
//! Only numpad keys exist here: the input method is meant to be used with the
//! right hand alone.
//!
//! - Idle: 1-9 picks a left feature. `.` opens punctuation. `0` also opens
//!   punctuation while nothing has been typed yet ("fresh"); afterwards it
//!   shows the characters associated with the last one committed, or the nine
//!   common characters if there are none.
//! - Associated characters: 1-9 commits, `0` is next page, `-` previous page.
//!   A character picked here has associated characters of its own, so `0`
//!   continues the word.
//! - Feature: 1-9 picks a component.
//! - Component: `0` completes a single-part code; 1-9 picks the right feature
//!   of a split code.
//! - Candidates: 1-9 commits, `0` is next page, `-` previous page.
//! - `.` cancels to idle from any other screen, and disarms homophones.
//! - Punctuation page 1, key 9 (More) opens a menu: 2 (sound) arms homophones
//!   for the next character, 4-9 open symbol lists. 1 and 3 (stroke and
//!   radical lookups) are not supported.
//! - With homophones armed, picking a character shows its homophones instead
//!   of committing it; picking one of those commits it.
//!
//! Surname lookup (`0` after a feature), shape lookup (`-`) and word input
//! (`+`) are not supported.
//! While composing, any key without a meaning is swallowed so it cannot leak
//! into the application mid-code; while idle, such keys pass through.

use std::sync::Arc;

use crate::association::Associations;
use crate::homophone::{Homophones, Layout};
use crate::learn::Promotions;
use crate::table::{Code, CodeTable, PAGE, SymbolTable};

/// The common characters `0` offers once something has been typed, by key
/// 1-9, when the last commit has no associated characters.
pub const COMMON_CHARS: [&str; 9] = ["個", "能", "的", "到", "資", "就", "你", "這", "好"];

const PUNCTUATION: &str = ".";
/// Punctuation page 1, key 9 opens the More menu.
const MORE_SLOT: usize = 8;
/// More menu by key 1-9; the stroke (1) and radical (3) lookups are not supported.
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
    /// More: opens the symbol menu.
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
    Associations,
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
    /// Auto-promotion moved a character; append [`Promotions::last_line`] to the saved log.
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
    Homophones { layout: Layout, page: usize },
    /// The characters associated with `of`.
    Associations { of: String, page: usize },
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
    associations: Arc<Associations>,
    promotions: Promotions,
    state: State,
    fresh: bool,
    armed: bool,
    /// The text committed last, whose associated characters `0` shows.
    last: Option<String>,
}

impl Engine {
    pub fn new(table: CodeTable, symbols: impl Into<Arc<SymbolTable>>) -> Engine {
        Engine {
            table,
            symbols: symbols.into(),
            homophones: Arc::default(),
            associations: Arc::default(),
            promotions: Promotions::default(),
            state: State::Idle,
            fresh: true,
            armed: false,
            last: None,
        }
    }

    pub fn with_homophones(mut self, homophones: impl Into<Arc<Homophones>>) -> Engine {
        self.homophones = homophones.into();
        self
    }

    pub fn with_associations(mut self, associations: impl Into<Arc<Associations>>) -> Engine {
        self.associations = associations.into();
        self
    }

    /// Restore a saved auto-promotion log, replaying it onto the table.
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

    /// Abandon any composition, e.g. when focus moves to another field. The
    /// last commit is forgotten too: it may not precede the caret any more.
    pub fn reset(&mut self) {
        self.state = State::Idle;
        self.armed = false;
        self.last = None;
    }

    pub fn key(&mut self, key: Key) -> Outcome {
        let state = std::mem::replace(&mut self.state, State::Idle);
        let (next, outcome) = self.step(state, key);
        if outcome.eaten {
            self.fresh = false;
        }
        if outcome.commit.is_some() {
            self.last.clone_from(&outcome.commit);
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
            (S::Idle, Digit(0)) => match &self.last {
                Some(of) if !self.associations.followers(of).is_empty() => stay(S::Associations { of: of.clone(), page: 0 }),
                _ => stay(S::Common),
            },
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

            (S::Homophones { layout, page }, Digit(k @ 1..=9)) => {
                let slot = page * PAGE + k as usize - 1;
                match layout.slots.get(slot).cloned().flatten() {
                    Some(text) => {
                        let learned =
                            layout.locate(slot).is_some_and(|(syllable, i)| self.promotions.record_homophone(syllable, i, &text));
                        (S::Idle, Outcome::commit(text, learned))
                    }
                    None => stay(S::Homophones { layout, page }),
                }
            }
            (S::Homophones { layout, page }, Digit(0) | Minus) => {
                let page = turn(page, layout.slots.len(), key == Digit(0));
                stay(S::Homophones { layout, page })
            }

            (S::Associations { of, page }, Digit(k @ 1..=9)) => {
                match self.associations.followers(&of).get(page * PAGE + k as usize - 1).cloned() {
                    Some(text) => self.pick_char(text),
                    None => stay(S::Associations { of, page }),
                }
            }
            (S::Associations { of, page }, Digit(0) | Minus) => {
                let page = turn(page, self.associations.followers(&of).len(), key == Digit(0));
                stay(S::Associations { of, page })
            }

            (S::Common, Digit(k @ 1..=9)) => self.pick_char(COMMON_CHARS[k as usize - 1].to_owned()),

            (state, _) => stay(state),
        }
    }

    fn open_code(&self, code: Code) -> State {
        if self.table.candidates(code).iter().all(Option::is_none) {
            State::NoCandidates(code)
        } else {
            State::Candidates { code, page: 0 }
        }
    }

    fn pick_candidate(&mut self, code: Code, page: usize, k: u8) -> (State, Outcome) {
        let index = page * PAGE + k as usize - 1;
        let Some(text) = self.table.candidates(code).get(index).cloned().flatten() else {
            return (State::Candidates { code, page }, Outcome::eaten());
        };
        if self.armed {
            return self.pick_char(text);
        }
        let learned = self.promotions.record(&mut self.table, code, index);
        (State::Idle, Outcome::commit(text, learned))
    }

    /// Commit `text`, or show its homophones if homophones are armed.
    fn pick_char(&mut self, text: String) -> (State, Outcome) {
        if std::mem::take(&mut self.armed) {
            let layout = self.homophones.layout(&text, |syllable, group| self.promotions.order(syllable, group));
            if !layout.slots.is_empty() {
                return (State::Homophones { layout, page: 0 }, Outcome::eaten());
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
                v.cells = text_cells(list, *page);
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
            S::Homophones { layout, page } => {
                v.screen = Screen::Homophones;
                v.cells = text_cells(&layout.slots, *page);
                v.zero_label = "下　頁";
                v.hints = ["選同音字", "或按下頁繼續找尋"];
                (v.page, v.pages) = (*page, page_count(layout.slots.len()));
            }
            S::Associations { of, page } => {
                let followers = self.associations.followers(of);
                v.screen = Screen::Associations;
                v.cells = std::array::from_fn(|i| followers.get(page * PAGE + i).map_or(Cell::Empty, |t| Cell::Text(t.clone())));
                v.zero_label = "下　頁";
                v.hints = ["選關聯字", "或按下頁繼續找尋"];
                (v.page, v.pages) = (*page, page_count(followers.len()));
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
        // Punctuation page 1 has no symbol on key 9 (More).
        tsv += &rows(".", &["！", "：", "；", "。", "，", "、", "—", "？"]);
        tsv += "t\t.\t2\t1\t‘\nt\t.\t2\t2\t“\n";
        let table = CodeTable::from_tsv(&tsv, "t").unwrap();
        let symbols = SymbolTable::from_tsv(&tsv, "t").unwrap();
        // Without tiers, 餓 (slot 0) and 鵝 (slot 1) rank before 我 (slot 6).
        let unihan = "U+6211\tkCantonese\tngo5\nU+9913\tkCantonese\tngo6\nU+9D5D\tkCantonese\tngo4\n";
        let homophones = Homophones::from_compact(&crate::homophone::compile(&table, unihan, "", "", &[]));
        let associations = Associations::from_compact("快\t樂 速 捷 餐 遞 車 感 意 活 手\n我\t哋 們\n反\t餓\n");
        Engine::new(table, symbols).with_homophones(homophones).with_associations(associations)
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
    fn split_code_commits_from_the_grid() {
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
    fn single_part_code_is_completed_with_zero() {
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
    fn blank_cells_pick_nothing() {
        use crate::table::BLANK;
        let tsv = rows("165", &[BLANK, BLANK, BLANK, BLANK, "都"]) + &rows("166", &[BLANK]);
        let mut e = Engine::new(CodeTable::from_tsv(&tsv, "t").unwrap(), SymbolTable::default());
        type_keys(&mut e, "165");
        assert_eq!(e.view().cells[0], Cell::Empty);
        assert_eq!(type_keys(&mut e, "1"), "");
        assert!(e.is_composing());
        assert_eq!(type_keys(&mut e, "5"), "都");
        // A code with nothing but blanks has no candidates.
        type_keys(&mut e, "166");
        assert_eq!(e.view().screen, Screen::NoCandidates);
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
    fn zero_after_a_commit_shows_its_associated_characters() {
        let mut e = engine();
        assert_eq!(type_keys(&mut e, "8922"), "快");
        type_keys(&mut e, "0");
        let v = e.view();
        assert_eq!((v.screen, v.zero_label), (Screen::Associations, "下　頁"));
        assert_eq!(v.cells[..3], [Cell::Text("樂".into()), Cell::Text("速".into()), Cell::Text("捷".into())]);
        assert_eq!((v.page, v.pages), (0, 2));
        // Page 2 holds 手 alone; an empty cell picks nothing.
        type_keys(&mut e, "0");
        assert_eq!(e.view().cells[0], Cell::Text("手".into()));
        assert_eq!(type_keys(&mut e, "2"), "");
        assert_eq!(type_keys(&mut e, "-2"), "速");
        assert!(!e.is_composing());
        // 速 has no associated characters, so `0` falls back to the common ones.
        type_keys(&mut e, "0");
        assert_eq!(e.view().screen, Screen::Common);
    }

    #[test]
    fn associated_characters_chain_and_cancel() {
        let mut e = engine();
        assert_eq!(type_keys(&mut e, "9707"), "我");
        assert_eq!(type_keys(&mut e, "01"), "哋");
        type_keys(&mut e, "0");
        assert_eq!(e.view().screen, Screen::Common);
        type_keys(&mut e, ".");
        assert_eq!(type_keys(&mut e, "9707"), "我");
        // `.` cancels; 我 is still the last commit.
        type_keys(&mut e, "0.0");
        assert_eq!(e.view().screen, Screen::Associations);
        // Punctuation is a commit too, with nothing associated.
        assert_eq!(type_keys(&mut e, "..5"), "，");
        type_keys(&mut e, "0");
        assert_eq!(e.view().screen, Screen::Common);
    }

    #[test]
    fn reset_forgets_the_last_commit() {
        let mut e = engine();
        assert_eq!(type_keys(&mut e, "8922"), "快");
        e.reset();
        type_keys(&mut e, "0");
        assert_eq!(e.view().screen, Screen::Common);
    }

    #[test]
    fn armed_homophones_apply_to_an_associated_character() {
        let mut e = engine();
        assert_eq!(type_keys(&mut e, "9701"), "反");
        type_keys(&mut e, ".92");
        assert_eq!(type_keys(&mut e, "01"), "");
        assert_eq!(e.view().screen, Screen::Homophones);
        assert_eq!(type_keys(&mut e, "2"), "鵝");
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
        // The unsupported stroke lookup does nothing.
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
    fn homophones_from_page_three_move_to_page_two_key_one() {
        let chars: Vec<String> = (0..25).map(|i| char::from_u32(0x4E00 + i).unwrap().to_string()).collect();
        let items: Vec<&str> = chars.iter().map(String::as_str).collect();
        let table = CodeTable::from_tsv(&rows("110", &items), "t").unwrap();
        let readings: String = (0..25).map(|i| format!("U+{:X}\tkCantonese\tsi1\n", 0x4E00 + i)).collect();
        let homophones = Homophones::from_compact(&crate::homophone::compile(&table, &readings, "", "", &[]));
        let mut e = Engine::new(table, SymbolTable::default()).with_homophones(homophones);

        // Arm homophones, type the first character, then page 3 key 2.
        let out = ".921101002".chars().map(|c| e.key(key_of(c))).last().unwrap();
        assert_eq!(out.commit.as_deref(), Some(chars[19].as_str()));
        assert!(out.learned);
        assert_eq!(e.promotions().last_line(), Some(format!("si\t{}\n", chars[19])));

        // It is now page 2 key 1 of every character read si; page 1 is unchanged.
        type_keys(&mut e, ".921105");
        assert_eq!(e.view().cells[0], Cell::Text(chars[0].clone()));
        type_keys(&mut e, "0");
        assert_eq!(e.view().cells[0], Cell::Text(chars[19].clone()));
        // Picking from page 2 learns nothing.
        let out = e.key(Key::Digit(2));
        assert_eq!(out.commit.as_deref(), Some(chars[9].as_str()));
        assert!(!out.learned);
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
