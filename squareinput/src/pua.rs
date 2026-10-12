//! Private-use characters in the harvested tables, which were kept in Big5
//! and converted to Unicode the way Microsoft's MS950_HKSCS does, so Big5
//! cells that HKSCS-2008 leaves undefined come out as private-use code points
//! that no font draws. [`clean`] replaces those that have a standard character
//! and takes out the rest. The cells are:
//!
//! - HKSCS compatibility points: GCCS characters that HKSCS unified with
//!   other characters (`big5cmp.txt`, published with HKSCS by the Hong Kong
//!   government). Most are duplicates of a character already under the code.
//! - GCCS characters HKSCS retired as unverifiable. Only A062 has been encoded
//!   since (Unicode U-source UTC-00892).
//! - Big5 81xx, which no standard defines.
//!
//! On page 1 a character taken out leaves a blank cell, so the others keep
//! their keys. From page 2 on, which auto-promotion reorders anyway, later
//! characters move up.

use std::collections::{HashMap, HashSet};

use crate::table::{BLANK, Code, PAGE};

/// HKSCS compatibility points and the character each was unified with.
const COMPATIBILITY: [(u16, char); 84] = [
    (0x8E69, '箸'), (0x8E6F, '簆'), (0x8E7E, '糎'), (0x8EAB, '緒'), (0x8EB4, '縝'), (0x8ECD, '者'),
    (0x8ED0, '耨'), (0x8F57, '菁'), (0x8F69, '蒨'), (0x8F6E, '萏'), (0x8FCB, '覦'), (0x8FCC, '覩'),
    (0x8FFE, '起'), (0x906D, '都'), (0x907A, '銹'), (0x90DC, '靜'), (0x90F1, '響'), (0x91BF, '鼖'),
    (0x9244, '蔃'), (0x92AF, '兙'), (0x92B0, '兛'), (0x92B1, '兝'), (0x92B2, '兞'), (0x92C8, '鍮'),
    (0x92D1, '瑹'), (0x9447, '浧'), (0x94CA, '禛'), (0x95D9, '邗'), (0x9644, '靝'), (0x96ED, '瀞'),
    (0x96FC, '嬨'), (0x9B76, '爁'), (0x9B78, '矗'), (0x9B7B, '纇'), (0x9BC6, '駖'), (0x9BDE, '釔'),
    (0x9BEC, '惞'), (0x9BF6, '澶'), (0x9C42, '輶'), (0x9C53, '侻'), (0x9C62, '營'), (0x9C68, '鄄'),
    (0x9C6B, '鷰'), (0x9C77, '菏'), (0x9CBC, '尐'), (0x9CBD, '秣'), (0x9CD0, '婧'), (0x9D57, '輋'),
    (0x9D5A, '筑'), (0x9DC4, '拐'), (0x9EA9, '恢'), (0x9EEF, '痹'), (0x9EFD, '汊'), (0x9F60, '鬮'),
    (0x9F66, '鼗'), (0x9FCB, '僭'), (0x9FD8, '弌'), (0xA063, '蠏'), (0xA077, '拎'), (0xA0D5, '瑨'),
    (0xA0DF, '煢'), (0xA0E4, '牐'), (0xFA5F, '倩'), (0xFA66, '偽'), (0xFABD, '包'), (0xFAC5, '卄'),
    (0xFAD5, '卿'), (0xFB48, '嘅'), (0xFBB8, '婷'), (0xFBF3, '幵'), (0xFBF9, '廐'), (0xFC4F, '彘'),
    (0xFC6C, '悤'), (0xFCB9, '撐'), (0xFCE2, '晴'), (0xFCF1, '杞'), (0xFDB7, '沜'), (0xFDB8, '渝'),
    (0xFDBB, '港'), (0xFDF1, '煮'), (0xFE52, '猪'), (0xFE6F, '瑜'), (0xFEAA, '瓩'), (0xFEDD, '砉'),
];

/// Retired GCCS characters that Unicode has encoded since.
const RETIRED: [(u16, char); 1] = [(0xA062, '\u{2DF3C}')];

/// Where code page 950 puts the Big5 user-defined ranges: first and last
/// private-use code point, and the Big5 code of the first.
const RANGES: [(u32, u32, u16); 4] =
    [(0xE000, 0xE310, 0xFA40), (0xE311, 0xEEB7, 0x8E40), (0xEEB8, 0xF6B0, 0x8140), (0xF6B1, 0xF848, 0xC6A1)];

/// Cells per Big5 lead byte: trail bytes 40-7E, then A1-FE.
const ROW: u32 = 157;

fn trail_index(trail: u32) -> u32 {
    if trail < 0x7F { trail - 0x40 } else { trail - 0xA1 + 0x3F }
}

fn trail_byte(index: u32) -> u32 {
    if index < 0x3F { 0x40 + index } else { 0xA1 + index - 0x3F }
}

fn is_private_use(c: char) -> bool {
    ('\u{E000}'..='\u{F8FF}').contains(&c)
}

/// The Big5 code a private-use character was converted from.
fn big5(c: char) -> Option<u16> {
    let c = u32::from(c);
    let &(first, _, start) = RANGES.iter().find(|&&(first, last, _)| (first..=last).contains(&c))?;
    let start = u32::from(start);
    let i = c - first + trail_index(start & 0xFF);
    u16::try_from(((start >> 8) + i / ROW) << 8 | trail_byte(i % ROW)).ok()
}

/// `c`, or for a private-use character the standard one, if any.
fn standard(c: char) -> Option<char> {
    if !is_private_use(c) {
        return Some(c);
    }
    let code = big5(c)?;
    COMPATIBILITY.iter().chain(&RETIRED).find(|&&(b, _)| b == code).map(|&(_, u)| u)
}

/// A harvest TSV without private-use characters, from [`clean`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cleaned {
    pub tsv: String,
    /// Entries replaced by their standard character.
    pub mapped: usize,
    /// Entries taken out on page 1, now blank cells.
    pub blanked: usize,
    /// Entries taken out from page 2 on.
    pub removed: usize,
}

/// One row of a shape code: (slot, text).
type Row<'a> = (usize, &'a str);

/// Rewrite the shape-code rows of a harvest TSV without private-use characters.
/// A private-use entry becomes its standard character, unless there is none or
/// the code already lists it. Every other line is copied unchanged.
pub fn clean(tsv: &str) -> Cleaned {
    enum Line<'a> {
        Copy(&'a str),
        List(&'a str, &'a str),
    }
    let mut lines = Vec::new();
    let mut lists: HashMap<(&str, &str), Vec<Row<'_>>> = HashMap::new();
    for line in tsv.lines() {
        let cols: Vec<&str> = line.split('\t').collect();
        let &[tag, name, page, key, text] = cols.as_slice() else {
            lines.push(Line::Copy(line));
            continue;
        };
        let (Some(_), Ok(page @ 1..), Ok(key @ 1..=9)) = (Code::parse(name), page.parse::<usize>(), key.parse::<usize>()) else {
            lines.push(Line::Copy(line));
            continue;
        };
        let rows = lists.entry((tag, name)).or_default();
        if rows.is_empty() {
            lines.push(Line::List(tag, name));
        }
        rows.push(((page - 1) * PAGE + key - 1, text));
    }

    let mut out = Cleaned::default();
    for line in lines {
        match line {
            Line::Copy(line) => {
                out.tsv += line;
                out.tsv.push('\n');
            }
            Line::List(tag, name) => {
                let mut rows = lists.remove(&(tag, name)).unwrap_or_default();
                rows.sort_unstable();
                let rows = clean_list(&rows, &mut out);
                for (slot, text) in rows {
                    out.tsv += &format!("{tag}\t{name}\t{}\t{}\t{text}\n", slot / PAGE + 1, slot % PAGE + 1);
                }
            }
        }
    }
    out
}

/// Clean one code's rows, sorted by slot, counting into `out`.
fn clean_list(rows: &[Row<'_>], out: &mut Cleaned) -> Vec<(usize, String)> {
    let plain: HashSet<&str> = rows.iter().map(|&(_, text)| text).filter(|t| !t.chars().any(is_private_use)).collect();
    let mut added: HashSet<String> = HashSet::new();
    let mut cleaned = Vec::new();
    let mut next = PAGE;
    for &(slot, text) in rows {
        let text = if text.chars().any(is_private_use) {
            let standard: Option<String> = text.chars().map(standard).collect();
            let kept = standard.filter(|s| !plain.contains(s.as_str()) && added.insert(s.clone()));
            match (&kept, slot < PAGE) {
                (Some(_), _) => out.mapped += 1,
                (None, true) => out.blanked += 1,
                (None, false) => out.removed += 1,
            }
            kept
        } else {
            Some(text.to_owned())
        };
        if slot < PAGE {
            cleaned.push((slot, text.unwrap_or_else(|| BLANK.to_owned())));
        } else if let Some(text) = text {
            cleaned.push((next, text));
            next += 1;
        }
    }
    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn big5_follows_code_page_950() {
        assert_eq!(big5('\u{E000}'), Some(0xFA40));
        assert_eq!(big5('\u{E311}'), Some(0x8E40));
        assert_eq!(big5('\u{EEB7}'), Some(0xA0FE));
        assert_eq!(big5('\u{EEB8}'), Some(0x8140));
        assert_eq!(big5('\u{F6B1}'), Some(0xC6A1));
        assert_eq!(big5('\u{F848}'), Some(0xC8FE));
        assert_eq!(big5('\u{F849}'), None);
        // The example in OpenJDK bug 7006360: MS950_HKSCS decodes 8F57 to U+E3C5.
        assert_eq!(big5('\u{E3C5}'), Some(0x8F57));
    }

    #[test]
    fn standard_characters() {
        assert_eq!(standard('\u{E3C5}'), Some('菁'));
        assert_eq!(standard('\u{EE3D}'), Some('\u{2DF3C}'));
        // Big5 81A3: no standard character.
        assert_eq!(standard('\u{EEF9}'), None);
        assert_eq!(standard('快'), Some('快'));
    }

    #[test]
    fn page_one_keeps_its_keys_and_later_pages_close_up() {
        // 165, page 1: blank, 都 as a compatibility point, 都, Big5 81A3, 甲.
        // Page 2: Big5 81A3, 乙, 菁 as a compatibility point, 丙.
        let mut tsv = String::from("t\t.\t1\t1\t！\nt\t161\t-\t-\t\n");
        for (i, text) in [BLANK, "\u{E478}", "都", "\u{EEF9}", "甲", "\u{EEF9}", "乙", "\u{E3C5}", "丙"].iter().enumerate() {
            let slot = if i < 5 { i } else { PAGE + i - 5 };
            tsv += &format!("t\t165\t{}\t{}\t{text}\n", slot / PAGE + 1, slot % PAGE + 1);
        }
        let out = clean(&tsv);
        let want = format!(
            "t\t.\t1\t1\t！\nt\t161\t-\t-\t\n\
             t\t165\t1\t1\t{BLANK}\nt\t165\t1\t2\t{BLANK}\nt\t165\t1\t3\t都\nt\t165\t1\t4\t{BLANK}\nt\t165\t1\t5\t甲\n\
             t\t165\t2\t1\t乙\nt\t165\t2\t2\t菁\nt\t165\t2\t3\t丙\n"
        );
        assert_eq!(out.tsv, want);
        assert_eq!((out.mapped, out.blanked, out.removed), (1, 2, 1));
    }

    #[test]
    fn tables_without_private_use_are_unchanged() {
        let tsv = "t\t892\t1\t1\t惰\nt\t892\t1\t2\t快\nt\t.94\t1\t1\tㄅ\n";
        assert_eq!(clean(tsv), Cleaned { tsv: tsv.to_owned(), ..Cleaned::default() });
    }
}
