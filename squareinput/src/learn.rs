//! Auto-promotion: a character committed from page 3 or later moves to page 2 key 1,
//! pushing the characters between down one slot. Characters already on the
//! first two pages never move, so page 1, where muscle memory matters most,
//! is stable and page 2 changes only by insertion at its front.
//!
//! The same goes for homophones, per syllable: a homophone picked from page 3 or
//! later of its reading moves to page 2 key 1 of that reading, for every
//! character with that reading.
//!
//! Moves are kept as a log of (code or syllable, text) and replayed onto the
//! base order, so the base table can be re-harvested without losing them. The
//! log is append-only, so several processes can add to the same file.

use std::fmt;

use crate::table::{Code, CodeTable, PAGE};

/// Candidates from this slot on (page 3) move when committed.
pub const PROMOTE_FROM: usize = 2 * PAGE;
/// Slot that promoted characters move to: page 2, key 1.
pub const PROMOTE_TO: usize = PAGE;

/// The list a move happened in.
#[derive(Debug, Clone, PartialEq, Eq)]
enum List {
    Code(Code),
    /// The homophones of a toneless syllable, e.g. "si".
    Syllable(String),
}

impl List {
    fn parse(s: &str) -> Option<List> {
        match Code::parse(s) {
            Some(code) => Some(List::Code(code)),
            None => (!s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase())).then(|| List::Syllable(s.to_owned())),
        }
    }
}

impl fmt::Display for List {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            List::Code(code) => code.fmt(f),
            List::Syllable(s) => f.write_str(s),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Promotions {
    log: Vec<(List, String)>,
}

/// Move `list[index]` to [`PROMOTE_TO`] if it is on page 3 or later. Returns whether it moved.
fn promote<T>(list: &mut Vec<T>, index: usize) -> bool {
    if index < PROMOTE_FROM || index >= list.len() {
        return false;
    }
    let item = list.remove(index);
    list.insert(PROMOTE_TO, item);
    true
}

fn line(list: &List, text: &str) -> String {
    format!("{list}\t{text}\n")
}

impl Promotions {
    /// Parse a saved log (`code or syllable \t text` per line); malformed lines are skipped.
    pub fn parse(saved: &str) -> Promotions {
        let log = saved
            .lines()
            .filter_map(|line| {
                let (list, text) = line.split_once('\t')?;
                Some((List::parse(list)?, text.to_owned())).filter(|(_, t)| !t.is_empty())
            })
            .collect();
        Promotions { log }
    }

    pub fn serialize(&self) -> String {
        self.log.iter().map(|(list, text)| line(list, text)).collect()
    }

    /// The most recent move as a log line, to append to the saved log.
    pub fn last_line(&self) -> Option<String> {
        self.log.last().map(|(list, text)| line(list, text))
    }

    pub fn len(&self) -> usize {
        self.log.len()
    }

    pub fn is_empty(&self) -> bool {
        self.log.is_empty()
    }

    /// Replay the code moves onto a freshly loaded table.
    pub fn apply(&self, table: &mut CodeTable) {
        for (list, text) in &self.log {
            if let List::Code(code) = list
                && let Some(list) = table.candidates_mut(*code)
                && let Some(i) = list.iter().position(|t| t.as_ref() == Some(text))
            {
                promote(list, i);
            }
        }
    }

    /// The candidate at `index` of `code` was committed. Returns whether the
    /// table (and so the log) changed.
    pub fn record(&mut self, table: &mut CodeTable, code: Code, index: usize) -> bool {
        let Some(list) = table.candidates_mut(code) else { return false };
        let Some(text) = list.get(index).cloned().flatten() else { return false };
        if !promote(list, index) {
            return false;
        }
        self.log.push((List::Code(code), text));
        true
    }

    /// The homophones of `syllable` in learned order: `group` (the base order) with
    /// the moves replayed.
    pub fn order(&self, syllable: &str, group: &[String]) -> Vec<String> {
        let mut order = group.to_vec();
        for (list, text) in &self.log {
            if matches!(list, List::Syllable(s) if s == syllable)
                && let Some(i) = order.iter().position(|t| t == text)
            {
                promote(&mut order, i);
            }
        }
        order
    }

    /// `text`, at `index` of the learned order of `syllable`, was picked as a
    /// homophone. Returns whether the order (and so the log) changed.
    pub fn record_homophone(&mut self, syllable: &str, index: usize, text: &str) -> bool {
        if index < PROMOTE_FROM {
            return false;
        }
        self.log.push((List::Syllable(syllable.to_owned()), text.to_owned()));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODE: Code = Code([1, 1, 0]);

    fn table(n: usize) -> CodeTable {
        let tsv: String = (0..n).map(|i| format!("t\t110\t{}\t{}\tc{i}\n", i / PAGE + 1, i % PAGE + 1)).collect();
        CodeTable::from_tsv(&tsv, "t").unwrap()
    }

    #[test]
    fn pages_one_and_two_never_move() {
        let mut t = table(30);
        let mut p = Promotions::default();
        for i in [0, 4, 8, 9, 13, 17] {
            assert!(!p.record(&mut t, CODE, i), "{i}");
        }
        assert_eq!(t.candidates(CODE), table(30).candidates(CODE));
        assert!(p.is_empty());
    }

    #[test]
    fn page_three_on_moves_to_page_two_key_one() {
        let mut t = table(30);
        let mut p = Promotions::default();
        assert!(p.record(&mut t, CODE, 18));
        assert_eq!(t.candidates(CODE)[9].as_deref(), Some("c18"));
        assert!(p.record(&mut t, CODE, 25));
        let c = t.candidates(CODE);
        assert_eq!(c[9].as_deref(), Some("c25"));
        assert_eq!(c[10].as_deref(), Some("c18"));
        assert_eq!(c[11].as_deref(), Some("c9"));
        assert_eq!(c[26].as_deref(), Some("c26"));
        assert_eq!(&c[..9], &table(30).candidates(CODE)[..9]);
        assert_eq!(p.last_line().as_deref(), Some("110\tc25\n"));
    }

    #[test]
    fn saved_log_replays_to_the_same_order() {
        let mut t = table(40);
        let mut p = Promotions::default();
        p.record(&mut t, CODE, 30);
        p.record(&mut t, CODE, 35);
        p.record(&mut t, CODE, 20);

        let saved = p.serialize();
        let reloaded = Promotions::parse(&saved);
        assert_eq!(reloaded, p);
        let mut fresh = table(40);
        reloaded.apply(&mut fresh);
        assert_eq!(fresh.candidates(CODE), t.candidates(CODE));
    }

    #[test]
    fn parse_skips_junk() {
        let p = Promotions::parse("110\tc20\nnot a line\n999\t\n0x1\tx\nsi\t時\nSi\tx\n\tx\n");
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn unknown_code_or_index_is_ignored() {
        let mut t = table(20);
        let mut p = Promotions::default();
        assert!(!p.record(&mut t, Code([9, 9, 9]), 19));
        assert!(!p.record(&mut t, CODE, 99));
    }

    #[test]
    fn homophones_move_per_syllable() {
        let group: Vec<String> = (0..30).map(|i| format!("h{i}")).collect();
        let mut p = Promotions::default();
        assert!(!p.record_homophone("si", 17, "h17"));
        assert!(p.record_homophone("si", 20, "h20"));
        assert_eq!(p.last_line().as_deref(), Some("si\th20\n"));
        let order = p.order("si", &group);
        assert_eq!((order[9].as_str(), order[10].as_str(), order[21].as_str()), ("h20", "h9", "h21"));
        assert_eq!(&order[..9], &group[..9]);
        // Other syllables and the code table are untouched.
        assert_eq!(p.order("ngo", &group), group);
        let mut t = table(30);
        p.apply(&mut t);
        assert_eq!(t.candidates(CODE), table(30).candidates(CODE));

        // Code and syllable moves share one log.
        let mut t = table(30);
        p.record(&mut t, CODE, 25);
        let reloaded = Promotions::parse(&p.serialize());
        assert_eq!(reloaded, p);
        assert_eq!(reloaded.order("si", &group), order);
    }

    #[test]
    fn blank_cells_never_move() {
        let tsv: String = (0..20).map(|i| format!("t\t110\t{}\t{}\t{}\n", i / PAGE + 1, i % PAGE + 1, if i == 19 { crate::table::BLANK } else { "c" })).collect();
        let mut t = CodeTable::from_tsv(&tsv, "t").unwrap();
        let mut p = Promotions::default();
        assert!(!p.record(&mut t, CODE, 19));
        assert_eq!(t.candidates(CODE)[19], None);
        assert!(p.is_empty());
    }
}
