//! Code table and symbol lists, loaded from the harvest TSV format:
//! `tag \t name \t page \t key \t text`, one line per entry, where the entry
//! sits on 1-based `page` under numpad `key`. Names with no entries have a
//! single line with `-` for page and key.
//!
//! 字形 codes are named by their three digits ("892", "270"); symbol lists by
//! how they are opened ("." for punctuation, ".94" for 其他 → ㄅ).
//!
//! A list is stored as slots: slot `i` is page `i / 9`, key `i % 9 + 1`.

use std::collections::HashMap;
use std::fmt;

/// Keys per page.
pub const PAGE: usize = 9;

/// A 字形 code: left feature, component, then a right feature (分體) or 0 (獨體).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Code(pub [u8; 3]);

impl Code {
    pub fn parse(s: &str) -> Option<Code> {
        let &[f, c, r] = s.as_bytes() else { return None };
        let digit = |b: u8| b.is_ascii_digit().then(|| b - b'0');
        let (f, c, r) = (digit(f)?, digit(c)?, digit(r)?);
        (f >= 1 && c >= 1).then_some(Code([f, c, r]))
    }

    pub fn feature(self) -> u8 {
        self.0[0]
    }

    pub fn component(self) -> u8 {
        self.0[1]
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c] = self.0;
        write!(f, "{a}{b}{c}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for TableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for TableError {}

type Slots = Vec<Option<String>>;

/// Rows of one tag, by name. Every name in the file appears, even without entries.
fn parse_rows(tsv: &str, tag: &str) -> Result<HashMap<String, Slots>, TableError> {
    let mut lists: HashMap<String, Slots> = HashMap::new();
    for (i, line) in tsv.lines().enumerate() {
        let err = |message: String| TableError { line: i + 1, message };
        if line.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        let &[row_tag, name, page, key, text] = cols.as_slice() else {
            return Err(err(format!("expected 5 tab-separated columns, got {}", cols.len())));
        };
        if row_tag != tag {
            continue;
        }
        let slots = lists.entry(name.to_owned()).or_default();
        if page == "-" {
            continue;
        }
        let page: usize = page.parse().ok().filter(|&p| p >= 1).ok_or_else(|| err(format!("bad page {page:?}")))?;
        let key: usize = key.parse().ok().filter(|k| (1..=9).contains(k)).ok_or_else(|| err(format!("bad key {key:?}")))?;
        if text.is_empty() {
            return Err(err("empty text".into()));
        }
        let slot = (page - 1) * PAGE + key - 1;
        if slots.len() <= slot {
            slots.resize(slot + 1, None);
        }
        if slots[slot].replace(text.to_owned()).is_some() {
            return Err(err(format!("{name} page {page} key {key} appears twice")));
        }
    }
    Ok(lists)
}

/// 字形 candidates per code, in display order.
#[derive(Debug, Clone, Default)]
pub struct CodeTable {
    lists: HashMap<Code, Vec<String>>,
}

impl CodeTable {
    /// Load the `tag` rows of a harvest TSV. Rows whose name is not a 字形 code
    /// are ignored. Candidate lists must have no gaps.
    pub fn from_tsv(tsv: &str, tag: &str) -> Result<CodeTable, TableError> {
        let mut lists = HashMap::new();
        for (name, slots) in parse_rows(tsv, tag)? {
            let Some(code) = Code::parse(&name) else { continue };
            let list: Option<Vec<String>> = slots.into_iter().collect();
            let list = list.ok_or_else(|| TableError { line: 0, message: format!("code {code} has a gap") })?;
            lists.insert(code, list);
        }
        Ok(CodeTable { lists })
    }

    pub fn candidates(&self, code: Code) -> &[String] {
        self.lists.get(&code).map_or(&[], Vec::as_slice)
    }

    pub(crate) fn candidates_mut(&mut self, code: Code) -> Option<&mut Vec<String>> {
        self.lists.get_mut(&code)
    }

    pub fn len(&self) -> usize {
        self.lists.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lists.is_empty()
    }

    /// Each candidate's earliest slot over all codes. The table lists common
    /// characters early, so this doubles as a frequency rank.
    pub fn best_slots(&self) -> HashMap<&str, usize> {
        let mut best: HashMap<&str, usize> = HashMap::new();
        for list in self.lists.values() {
            for (i, text) in list.iter().enumerate() {
                best.entry(text.as_str()).and_modify(|b| *b = (*b).min(i)).or_insert(i);
            }
        }
        best
    }
}

/// Punctuation and symbol lists. Slots may be empty (punctuation page 1, key 9
/// is the 其他 menu rather than a symbol).
#[derive(Debug, Clone, Default)]
pub struct SymbolTable {
    lists: HashMap<String, Slots>,
}

impl SymbolTable {
    /// Load the `tag` rows of a harvest TSV whose names start with '.'.
    pub fn from_tsv(tsv: &str, tag: &str) -> Result<SymbolTable, TableError> {
        let mut lists = parse_rows(tsv, tag)?;
        lists.retain(|name, _| name.starts_with('.'));
        Ok(SymbolTable { lists })
    }

    pub fn slots(&self, name: &str) -> &[Option<String>] {
        self.lists.get(name).map_or(&[], Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_parse() {
        assert_eq!(Code::parse("892"), Some(Code([8, 9, 2])));
        assert_eq!(Code::parse("270"), Some(Code([2, 7, 0])));
        assert_eq!(Code::parse("089"), None);
        assert_eq!(Code::parse("80"), None);
        assert_eq!(Code::parse(".94"), None);
        assert_eq!(Code([2, 7, 0]).to_string(), "270");
    }

    #[test]
    fn loads_candidates_in_slot_order() {
        let gappy = "t\t270\t1\t2\t九\nt\t270\t1\t1\t右\nt\t270\t2\t1\t丸\n";
        let err = CodeTable::from_tsv(gappy, "t").unwrap_err().to_string();
        assert!(err.contains("gap"), "{err}");

        let full: String = (1..=9).map(|k| format!("t\t270\t1\t{k}\t字{k}\n")).collect::<String>() + "t\t270\t2\t1\t丸\nt\t161\t-\t-\t\n";
        let table = CodeTable::from_tsv(&full, "t").unwrap();
        let c = table.candidates(Code([2, 7, 0]));
        assert_eq!(c.len(), 10);
        assert_eq!(c[1], "字2");
        assert_eq!(c[9], "丸");
        assert!(table.candidates(Code([1, 6, 1])).is_empty());
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn rejects_duplicates_and_bad_rows() {
        assert!(CodeTable::from_tsv("t\t270\t1\t1\t右\nt\t270\t1\t1\t九\n", "t").unwrap_err().message.contains("twice"));
        assert_eq!(CodeTable::from_tsv("t\t270\t1\t0\t右\n", "t").unwrap_err().line, 1);
        assert!(CodeTable::from_tsv("t\t270\t1\n", "t").is_err());
    }

    #[test]
    fn symbols_keep_gaps() {
        let tsv = "t\t.\t1\t1\t！\nt\t.\t1\t8\t？\nt\t.\t2\t1\t‘\nt\t.94\t1\t1\tㄅ\nt\t892\t1\t1\t惰\n";
        let s = SymbolTable::from_tsv(tsv, "t").unwrap();
        let p = s.slots(".");
        assert_eq!(p.len(), 10);
        assert_eq!(p[0].as_deref(), Some("！"));
        assert_eq!(p[8], None);
        assert_eq!(p[9].as_deref(), Some("‘"));
        assert_eq!(s.slots(".94")[0].as_deref(), Some("ㄅ"));
        assert!(s.slots("892").is_empty());
    }

    #[test]
    fn best_slots_takes_the_earliest_position() {
        let tsv = "t\t110\t1\t1\t華\nt\t110\t1\t2\t萬\nt\t120\t1\t1\t萬\n";
        let table = CodeTable::from_tsv(tsv, "t").unwrap();
        let best = table.best_slots();
        assert_eq!(best["華"], 0);
        assert_eq!(best["萬"], 0);
    }
}
