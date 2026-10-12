//! 同音字: characters sharing a Cantonese syllable, ignoring tone, from
//! Unihan's `kCantonese` readings (plus the 粵 readings in
//! `kSMSZD2003Readings`, which add second readings such as 車 geoi1).
//!
//! Two stages: [`compile`] turns Unihan, character frequency lists and the
//! code table into a compact list (`char \t syllable syllable…`, most common
//! character first), which the input method loads with
//! [`Homophones::from_compact`]. Parsing the 9 MB of Unihan text in every
//! process that types would be far too slow.

use std::collections::HashMap;

use crate::table::{CodeTable, PAGE};

/// Tier of characters [`unihan_tiers`] knows nothing about.
const UNTIERED: u8 = 8;

#[derive(Debug, Clone, Default)]
pub struct Homophones {
    /// Toneless syllables of each character, primary reading first.
    readings: HashMap<String, Vec<String>>,
    /// Characters of each syllable, most common first.
    groups: HashMap<String, Vec<String>>,
}

/// How common characters are, from Unihan (lower is more common): 0-5 for the
/// Hong Kong primary school grade a character is taught in (kGradeLevel),
/// 6 for Big5 常用字 (level 1), 7 for Big5 次常用字 (level 2). Characters with
/// none of these are absent.
pub fn unihan_tiers(dictionary_like: &str, other_mappings: &str) -> HashMap<String, u8> {
    let mut tiers: HashMap<String, u8> = HashMap::new();
    let fields = dictionary_like.lines().chain(other_mappings.lines()).filter_map(|line| {
        let mut cols = line.split('\t');
        Some((char_at(cols.next()?)?, cols.next()?, cols.next()?))
    });
    for (ch, field, value) in fields {
        let tier = match field {
            "kGradeLevel" => value.parse::<u8>().ok().filter(|g| (1..=6).contains(g)).map(|g| g - 1),
            "kBigFive" => match u16::from_str_radix(value, 16) {
                Ok(0xA440..=0xC67E) => Some(6),
                Ok(0xC940..=0xF9D5) => Some(7),
                _ => None,
            },
            _ => None,
        };
        if let Some(t) = tier {
            tiers.entry(ch).and_modify(|old| *old = (*old).min(t)).or_insert(t);
        }
    }
    tiers
}

fn toneless(jyutping: &str) -> Option<String> {
    let s = jyutping.trim_end_matches(|c: char| c.is_ascii_digit());
    (!s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase())).then(|| s.to_owned())
}

fn char_at(codepoint: &str) -> Option<String> {
    let hex = codepoint.strip_prefix("U+")?;
    char::from_u32(u32::from_str_radix(hex, 16).ok()?).map(String::from)
}

/// Toneless readings per character from Unihan_Readings.txt, primary first.
fn unihan_readings(unihan: &str) -> HashMap<String, Vec<String>> {
    let mut readings: HashMap<String, Vec<String>> = HashMap::new();
    for line in unihan.lines() {
        let mut cols = line.split('\t');
        let (Some(cp), Some(field), Some(value)) = (cols.next(), cols.next(), cols.next()) else { continue };
        let syllables: Vec<String> = match field {
            "kCantonese" => value.split_whitespace().filter_map(toneless).collect(),
            "kSMSZD2003Readings" => value
                .split_whitespace()
                .filter_map(|r| r.split_once('粵').map(|(_, yue)| yue))
                .flat_map(|yue| yue.split(','))
                .filter_map(toneless)
                .collect(),
            _ => continue,
        };
        // kCantonese precedes kSMSZD2003Readings for each character, so the
        // primary reading stays first.
        let Some(ch) = char_at(cp) else { continue };
        let list = readings.entry(ch).or_default();
        for s in syllables {
            if !list.contains(&s) {
                list.push(s);
            }
        }
    }
    readings
}

/// Character frequencies from `char \t count` lists, each scaled to counts per
/// million characters and multiplied by its weight, summed.
pub fn frequencies(lists: &[(&str, f64)]) -> HashMap<String, f64> {
    let mut freq: HashMap<String, f64> = HashMap::new();
    for &(list, weight) in lists {
        let counts: Vec<(&str, f64)> = list
            .lines()
            .filter_map(|line| {
                let (ch, count) = line.split_once('\t')?;
                Some((ch, count.trim().parse::<f64>().ok()?))
            })
            .collect();
        let total: f64 = counts.iter().map(|&(_, n)| n).sum();
        if total > 0.0 {
            for (ch, n) in counts {
                *freq.entry(ch.to_owned()).or_default() += weight * n * 1e6 / total;
            }
        }
    }
    freq
}

/// Compile the compact homophone list for the characters in `table`, most
/// common first: by [`frequencies`] of `frequency_lists` (list text, weight),
/// then, for characters the lists rank the same (mostly those they lack), by
/// [`unihan_tiers`] and position in the table. Characters not in the table
/// are left out, so groups hold only characters the user can type (no
/// simplified forms). The other arguments are the texts of
/// Unihan_Readings.txt, Unihan_DictionaryLikeData.txt and
/// Unihan_OtherMappings.txt.
pub fn compile(
    table: &CodeTable,
    readings: &str,
    dictionary_like: &str,
    other_mappings: &str,
    frequency_lists: &[(&str, f64)],
) -> String {
    let tiers = unihan_tiers(dictionary_like, other_mappings);
    let freq = frequencies(frequency_lists);
    let best = table.best_slots();
    let mut rows: Vec<(f64, usize, String, Vec<String>)> = unihan_readings(readings)
        .into_iter()
        .filter(|(_, syllables)| !syllables.is_empty())
        .filter_map(|(ch, syllables)| {
            let slot = *best.get(ch.as_str())?;
            let tier = tiers.get(&ch).copied().unwrap_or(UNTIERED);
            let f = freq.get(&ch).copied().unwrap_or(0.0);
            Some((f, usize::from(tier) * 1_000_000 + slot, ch, syllables))
        })
        .collect();
    rows.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| (a.1, &a.2).cmp(&(b.1, &b.2))));
    rows.iter().map(|(_, _, ch, syllables)| format!("{ch}\t{}\n", syllables.join(" "))).collect()
}

impl Homophones {
    /// Load a list made by [`compile`]; earlier lines are more common.
    pub fn from_compact(compact: &str) -> Homophones {
        let mut h = Homophones::default();
        for line in compact.lines() {
            let Some((ch, syllables)) = line.split_once('\t') else { continue };
            let syllables: Vec<String> = syllables.split_whitespace().map(str::to_owned).collect();
            if ch.is_empty() || syllables.is_empty() || h.readings.contains_key(ch) {
                continue;
            }
            for s in &syllables {
                h.groups.entry(s.clone()).or_default().push(ch.to_owned());
            }
            h.readings.insert(ch.to_owned(), syllables);
        }
        h
    }

    pub fn readings(&self, ch: &str) -> &[String] {
        self.readings.get(ch).map_or(&[], Vec::as_slice)
    }

    pub fn group(&self, syllable: &str) -> &[String] {
        self.groups.get(syllable).map_or(&[], Vec::as_slice)
    }

    /// Slots to pick a homophone of `ch` from: the characters of each of its
    /// readings in turn (`ch` among them), each reading starting on a new page
    /// and listed in the order `order` gives for its syllable and base order.
    /// Empty if `ch` has no reading.
    pub fn layout(&self, ch: &str, order: impl Fn(&str, &[String]) -> Vec<String>) -> Layout {
        let mut layout = Layout::default();
        for syllable in self.readings(ch) {
            layout.starts.push((layout.slots.len(), syllable.clone()));
            layout.slots.extend(order(syllable, self.group(syllable)).into_iter().map(Some));
            layout.slots.resize(layout.slots.len().div_ceil(PAGE) * PAGE, None);
        }
        while layout.slots.last().is_some_and(Option::is_none) {
            layout.slots.pop();
        }
        layout
    }

    /// [`Homophones::layout`] slots in the base order.
    pub fn slots(&self, ch: &str) -> Vec<Option<String>> {
        self.layout(ch, |_, group| group.to_vec()).slots
    }
}

/// 同音字 laid out for picking, from [`Homophones::layout`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layout {
    pub slots: Vec<Option<String>>,
    /// Each reading and the slot its characters start at.
    pub starts: Vec<(usize, String)>,
}

impl Layout {
    /// The syllable `slot` belongs to and the slot's index within it.
    pub fn locate(&self, slot: usize) -> Option<(&str, usize)> {
        let (start, syllable) = self.starts.iter().rev().find(|(start, _)| *start <= slot)?;
        Some((syllable, slot - start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const READINGS: &str = "\
# comment
U+6211\tkCantonese\tngo5
U+6211\tkMandarin\twǒ
U+9913\tkCantonese\tngo6
U+5CE8\tkCantonese\tngo4
U+6D90\tkCantonese\tngo4
U+4FC4\tkCantonese\tngo4
U+8ECA\tkCantonese\tce1
U+8ECA\tkSMSZD2003Readings\tchē粵ce1 jū粵geoi1
U+5C45\tkCantonese\tgeoi1
";
    const DICT: &str = "U+6211\tkGradeLevel\t1\nU+9913\tkGradeLevel\t3\nU+9913\tkTotalStrokes\t15\n";
    // 峨 AE70 is Big5 level 1, 涐 E2FC level 2.
    const OTHER: &str = "U+5CE8\tkBigFive\tAE70\nU+6D90\tkBigFive\tE2FC\n";

    fn table() -> CodeTable {
        // 涐 sits earlier in the table than 峨, but its tier is lower. 俄 is
        // not in the table at all.
        let tsv = "t\t111\t1\t1\t涐\nt\t111\t1\t2\t峨\nt\t222\t1\t1\t餓\nt\t222\t1\t2\t我\nt\t333\t1\t1\t車\nt\t333\t1\t2\t居\n";
        CodeTable::from_tsv(tsv, "t").unwrap()
    }

    fn homophones() -> Homophones {
        Homophones::from_compact(&compile(&table(), READINGS, DICT, OTHER, &[]))
    }

    #[test]
    fn frequency_lists_rank_first_and_tiers_break_ties() {
        // 涐 is now the most frequent; 峨 and 餓 are absent, so tiers order them.
        let lists = [("涐\t30\n我\t10\n", 1.0), ("我\t1\n俄\t1\n", 0.5)];
        let f = frequencies(&lists);
        assert_eq!(f["涐"], 750_000.0);
        assert_eq!(f["我"], 250_000.0 + 250_000.0);
        let h = Homophones::from_compact(&compile(&table(), READINGS, DICT, OTHER, &lists));
        assert_eq!(h.group("ngo"), ["涐", "我", "餓", "峨"]);
    }

    #[test]
    fn tiers_prefer_school_grade_then_big5_level() {
        let t = unihan_tiers(DICT, OTHER);
        assert_eq!(t["我"], 0);
        assert_eq!(t["餓"], 2);
        assert_eq!(t["峨"], 6);
        assert_eq!(t["涐"], 7);
        assert!(!t.contains_key("俄"));
    }

    #[test]
    fn groups_ignore_tone_follow_tier_and_keep_only_typeable() {
        let h = homophones();
        assert_eq!(h.readings("我"), ["ngo"]);
        assert_eq!(h.group("ngo"), ["我", "餓", "峨", "涐"]);
        assert!(h.readings("俄").is_empty());
    }

    #[test]
    fn second_readings_come_from_smszd() {
        let h = homophones();
        assert_eq!(h.readings("車"), ["ce", "geoi"]);
        assert_eq!(h.group("geoi"), ["車", "居"]);
    }

    #[test]
    fn each_reading_starts_a_page() {
        let h = homophones();
        let s = h.slots("我");
        assert_eq!(s.iter().flatten().collect::<Vec<_>>(), ["我", "餓", "峨", "涐"]);

        let s = h.slots("車");
        assert_eq!(s[0].as_deref(), Some("車"));
        assert_eq!(s[1], None);
        assert_eq!(s[PAGE].as_deref(), Some("車"));
        assert_eq!(s[PAGE + 1].as_deref(), Some("居"));
        assert_eq!(s.len(), PAGE + 2);
        assert!(h.slots("x").is_empty());

        let layout = h.layout("車", |_, group| group.to_vec());
        assert_eq!(layout.locate(0), Some(("ce", 0)));
        assert_eq!(layout.locate(PAGE + 1), Some(("geoi", 1)));
    }

    #[test]
    fn long_groups_spill_onto_following_pages_without_gaps() {
        let compact: String = (0..20).map(|i| format!("{}\tsi\n", char::from_u32(0x4E00 + i).unwrap())).collect();
        let s = Homophones::from_compact(&compact).slots("一");
        assert_eq!(s.len(), 20);
        assert!(s.iter().all(Option::is_some));
    }

    #[test]
    fn from_compact_skips_junk() {
        let h = Homophones::from_compact("我\tngo\nbad line\n\tngo\n餓\t\n我\tsi\n");
        assert_eq!(h.readings("我"), ["ngo"]);
        assert_eq!(h.group("ngo"), ["我"]);
        assert!(h.group("si").is_empty());
    }
}
