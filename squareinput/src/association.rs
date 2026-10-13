//! Associated characters: after a character is committed, `0` offers the
//! characters that most often follow it inside a word.
//!
//! Two stages, as for homophones: [`compile`] turns word lists, character
//! pair counts and the code table into a compact list (`char \t follower
//! follower…`, most common follower first), which the input method loads with
//! [`Associations::from_compact`].

use std::collections::{HashMap, HashSet};

use crate::homophone::frequencies;
use crate::table::{CodeTable, PAGE};

/// Followers kept per character: three pages.
pub const MAX_FOLLOWERS: usize = 3 * PAGE;

/// Only words up to this many characters lend their pairs. Longer entries are
/// mostly phrases and sample sentences, whose pairs often cross a word
/// boundary.
pub const MAX_WORD_LEN: usize = 4;

#[derive(Debug, Clone, Default)]
pub struct Associations {
    followers: HashMap<String, Vec<String>>,
}

/// Pair scores from `first \t second \t count` lists, each scaled to counts
/// per million pairs and multiplied by its weight, summed.
fn pair_scores(lists: &[(&str, f64)]) -> HashMap<(char, char), f64> {
    let mut scores: HashMap<(char, char), f64> = HashMap::new();
    for &(list, weight) in lists {
        let counts: Vec<((char, char), f64)> = list
            .lines()
            .filter_map(|line| {
                let mut cols = line.split('\t');
                let (first, second, count) = (single(cols.next()?)?, single(cols.next()?)?, cols.next()?);
                Some(((first, second), count.trim().parse::<f64>().ok()?))
            })
            .collect();
        let total: f64 = counts.iter().map(|&(_, n)| n).sum();
        if total > 0.0 {
            for (pair, n) in counts {
                *scores.entry(pair).or_default() += weight * n * 1e6 / total;
            }
        }
    }
    scores
}

fn single(s: &str) -> Option<char> {
    let mut chars = s.chars();
    chars.next().filter(|_| chars.next().is_none())
}

/// Compile the compact association list. `word_lists` are word lists in the
/// CC-CEDICT format (one entry per line, traditional form first, `#` starts a
/// comment); every pair of neighbouring characters inside a word of at most
/// [`MAX_WORD_LEN`] characters is a candidate. Candidates are ranked by
/// `pair_lists` (`first \t second \t count`, weight), then by the follower's
/// [`frequencies`] in `frequency_lists`, then by its position in the table.
/// Both characters must be in `table`, so followers are always typeable (no
/// simplified forms, which the word lists keep in their second column).
pub fn compile(table: &CodeTable, word_lists: &[&str], pair_lists: &[(&str, f64)], frequency_lists: &[(&str, f64)]) -> String {
    let best: HashMap<char, usize> = table.best_slots().into_iter().filter_map(|(text, slot)| Some((single(text)?, slot))).collect();
    let mut pairs: HashSet<(char, char)> = HashSet::new();
    for list in word_lists {
        for line in list.lines().filter(|line| !line.starts_with('#')) {
            let Some(word) = line.split_whitespace().next() else { continue };
            let chars: Vec<char> = word.chars().collect();
            if chars.len() > MAX_WORD_LEN {
                continue;
            }
            pairs.extend(chars.windows(2).map(|w| (w[0], w[1])).filter(|(a, b)| best.contains_key(a) && best.contains_key(b)));
        }
    }

    let scores = pair_scores(pair_lists);
    let freq = frequencies(frequency_lists);
    // Pair score, follower frequency, follower's best table slot, follower.
    type Ranked = (f64, f64, usize, char);
    let mut followers: HashMap<char, Vec<Ranked>> = HashMap::new();
    for (first, second) in pairs {
        let score = scores.get(&(first, second)).copied().unwrap_or(0.0);
        let f = freq.get(second.to_string().as_str()).copied().unwrap_or(0.0);
        followers.entry(first).or_default().push((score, f, best[&second], second));
    }
    let mut rows: Vec<(char, Vec<Ranked>)> = followers.into_iter().collect();
    rows.sort_unstable_by_key(|&(first, _)| first);
    rows.into_iter()
        .map(|(first, mut list)| {
            list.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.total_cmp(&a.1)).then((a.2, a.3).cmp(&(b.2, b.3))));
            let shown: Vec<String> = list.iter().take(MAX_FOLLOWERS).map(|&(_, _, _, c)| c.to_string()).collect();
            format!("{first}\t{}\n", shown.join(" "))
        })
        .collect()
}

impl Associations {
    /// Load a list made by [`compile`].
    pub fn from_compact(compact: &str) -> Associations {
        let mut a = Associations::default();
        for line in compact.lines() {
            let Some((ch, followers)) = line.split_once('\t') else { continue };
            let followers: Vec<String> = followers.split_whitespace().map(str::to_owned).collect();
            if !ch.is_empty() && !followers.is_empty() {
                a.followers.entry(ch.to_owned()).or_insert(followers);
            }
        }
        a
    }

    /// The characters to offer after `ch`, most common first; empty if none.
    pub fn followers(&self, ch: &str) -> &[String] {
        self.followers.get(ch).map_or(&[], Vec::as_slice)
    }

    pub fn len(&self) -> usize {
        self.followers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.followers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> CodeTable {
        // 国 is typeable too, but only as the simplified column of the word list.
        let tsv: String = "中國国心華文一的人民共和士"
            .chars()
            .enumerate()
            .map(|(i, c)| format!("t\t110\t{}\t{}\t{c}\n", i / PAGE + 1, i % PAGE + 1))
            .collect();
        CodeTable::from_tsv(&tsv, "t").unwrap()
    }

    const WORDS: &str = "\
# CC-CEDICT
中國 中国 [Zhong1 guo2] /China/
中心 中心 [zhong1 xin1] /center/
中文 中文 [Zhong1 wen2] /Chinese language/
中華 中华 [Zhong1 hua2] /China/
的士 的士 [di1 shi4] /taxi/
人民 人民 [ren2 min2] /the people/
中華人民共和國 中华人民共和国 [Zhong1 hua2 Ren2 min2 Gong4 he2 guo2] /People's Republic of China/
";

    #[test]
    fn followers_come_from_words_ranked_by_pair_counts() {
        // 中國 is the commonest pair; 中的 is common too but not inside any word.
        let pairs = [("中\t國\t50\n中\t的\t40\n中\t文\t10\n中\t心\t5\n", 1.0), ("中\t心\t100\n", 0.5)];
        let a = Associations::from_compact(&compile(&table(), &[WORDS], &pairs, &[]));
        assert_eq!(a.followers("中"), ["心", "國", "文", "華"]);
        assert_eq!(a.followers("的"), ["士"]);
        // 民共 and 和國 lie only inside a word longer than four characters.
        assert_eq!(a.followers("人"), ["民"]);
        assert!(a.followers("民").is_empty() && a.followers("和").is_empty());
    }

    #[test]
    fn unseen_pairs_follow_by_character_frequency() {
        let freq = [("文\t9\n華\t5\n", 1.0)];
        let a = Associations::from_compact(&compile(&table(), &[WORDS], &[("中\t心\t1\n", 1.0)], &freq));
        assert_eq!(a.followers("中"), ["心", "文", "華", "國"]);
    }

    #[test]
    fn at_most_three_pages_of_followers() {
        let chars: Vec<char> = (0..40).map(|i| char::from_u32(0x4E00 + i).unwrap()).collect();
        let tsv: String = chars.iter().enumerate().map(|(i, c)| format!("t\t110\t{}\t{}\t{c}\n", i / PAGE + 1, i % PAGE + 1)).collect();
        let table = CodeTable::from_tsv(&tsv, "t").unwrap();
        let words: String = chars[1..].iter().map(|c| format!("{}{c} x\n", chars[0])).collect();
        let a = Associations::from_compact(&compile(&table, &[words.as_str()], &[], &[]));
        // Without counts, table order decides.
        assert_eq!(a.followers(&chars[0].to_string()).len(), MAX_FOLLOWERS);
        assert_eq!(a.followers(&chars[0].to_string())[0], chars[1].to_string());
    }

    #[test]
    fn from_compact_skips_junk() {
        let a = Associations::from_compact("中\t國 心\nbad line\n\t國\n人\t\n中\t文\n");
        assert_eq!(a.followers("中"), ["國", "心"]);
        assert!(a.followers("人").is_empty());
        assert_eq!(a.len(), 1);
    }
}
