//! 自動調前: a character committed from page 3 or later moves to page 2 key 1,
//! pushing the characters between down one slot. Characters already on the
//! first two pages never move, so page 1, where muscle memory matters most,
//! is stable and page 2 changes only by insertion at its front.
//!
//! Moves are kept as a log of (code, text) and replayed onto the base table at
//! start-up, so the base table can be re-harvested without losing them. The
//! log is append-only, so several processes can add to the same file.

use crate::table::{Code, CodeTable, PAGE};

/// Candidates from this slot on (page 3) move when committed.
pub const PROMOTE_FROM: usize = 2 * PAGE;
/// Slot that promoted characters move to: page 2, key 1.
pub const PROMOTE_TO: usize = PAGE;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Promotions {
    log: Vec<(Code, String)>,
}

/// Move `list[index]` to [`PROMOTE_TO`] if it is on page 3 or later. Returns whether it moved.
fn promote(list: &mut Vec<Option<String>>, index: usize) -> bool {
    if index < PROMOTE_FROM || index >= list.len() {
        return false;
    }
    let item = list.remove(index);
    list.insert(PROMOTE_TO, item);
    true
}

fn line(code: &Code, text: &str) -> String {
    format!("{code}\t{text}\n")
}

impl Promotions {
    /// Parse a saved log (`code \t text` per line); malformed lines are skipped.
    pub fn parse(saved: &str) -> Promotions {
        let log = saved
            .lines()
            .filter_map(|line| {
                let (code, text) = line.split_once('\t')?;
                Some((Code::parse(code)?, text.to_owned())).filter(|(_, t)| !t.is_empty())
            })
            .collect();
        Promotions { log }
    }

    pub fn serialize(&self) -> String {
        self.log.iter().map(|(code, text)| line(code, text)).collect()
    }

    /// The most recent move as a log line, to append to the saved log.
    pub fn last_line(&self) -> Option<String> {
        self.log.last().map(|(code, text)| line(code, text))
    }

    pub fn len(&self) -> usize {
        self.log.len()
    }

    pub fn is_empty(&self) -> bool {
        self.log.is_empty()
    }

    /// Replay the log onto a freshly loaded table.
    pub fn apply(&self, table: &mut CodeTable) {
        for (code, text) in &self.log {
            if let Some(list) = table.candidates_mut(*code)
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
        self.log.push((code, text));
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
        let p = Promotions::parse("110\tc20\nnot a line\n999\t\n0x1\tx\n");
        assert_eq!(p.len(), 1);
    }

    #[test]
    fn unknown_code_or_index_is_ignored() {
        let mut t = table(20);
        let mut p = Promotions::default();
        assert!(!p.record(&mut t, Code([9, 9, 9]), 19));
        assert!(!p.record(&mut t, CODE, 99));
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
