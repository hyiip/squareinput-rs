//! Checks against the real harvested tables and Unihan, which live outside
//! the repository (the tables are not ours to publish). Each test is skipped
//! when its data is missing. Set SQUAREINPUT_LOCAL to the data directory; the
//! default is `local` beside the workspace.

use std::path::PathBuf;

use squareinput::{Code, CodeTable, Engine, Homophones, Key, PAGE, SymbolTable, compile_homophones};

fn local() -> PathBuf {
    std::env::var_os("SQUAREINPUT_LOCAL").map_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../local")), PathBuf::from)
}

fn read(name: &str) -> Option<String> {
    let path = local().join(name);
    match std::fs::read_to_string(&path) {
        Ok(s) => Some(s),
        Err(_) => {
            eprintln!("skipping: {} not found", path.display());
            None
        }
    }
}

/// 200,000 random numpad keys over the real tables, 同音字 and 自動調前
/// included: the engine must never panic and its view must stay consistent.
#[test]
fn random_keys_keep_the_engine_consistent() {
    let (Some(tsv), Some(punct), Some(readings), Some(dict), Some(other)) = (
        read("zixing.tsv"),
        read("punct.tsv"),
        read(r"unihan\x\Unihan_Readings.txt"),
        read(r"unihan\x\Unihan_DictionaryLikeData.txt"),
        read(r"unihan\x\Unihan_OtherMappings.txt"),
    ) else {
        return;
    };
    let table = CodeTable::from_tsv(&tsv, "current").unwrap();
    let homophones = Homophones::from_compact(&compile_homophones(&table, &readings, &dict, &other));
    let symbols = SymbolTable::from_tsv(&punct, "current").unwrap();
    let mut e = Engine::new(table, symbols).with_homophones(homophones);

    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let (mut commits, mut learned) = (0, 0);
    for _ in 0..200_000 {
        let key = match next() % 14 {
            n @ 0..=9 => Key::Digit(n as u8),
            10 => Key::Dot,
            11 => Key::Minus,
            12 => Key::Plus,
            // Extra weight on 0 (paging), the key with the most state.
            _ => Key::Digit(0),
        };
        let wanted = e.wants(key);
        let out = e.key(key);
        assert_eq!(out.eaten, wanted, "wants() disagreed with key() for {key:?}");
        if let Some(text) = &out.commit {
            assert!(!text.is_empty());
            commits += 1;
        }
        learned += usize::from(out.learned);
        let v = e.view();
        assert!(v.pages >= 1 && v.page < v.pages, "page {} of {}", v.page, v.pages);
    }
    eprintln!("200000 random keys: {commits} commits, {learned} 自動調前 moves, {} logged", e.promotions().len());
    assert!(commits > 1000);
}

fn press(e: &mut Engine, keys: &str) -> String {
    keys.chars()
        .filter_map(|c| {
            let key = match c {
                '0'..='9' => Key::Digit(c as u8 - b'0'),
                '.' => Key::Dot,
                _ => panic!("no key for {c:?}"),
            };
            e.key(key).commit
        })
        .collect()
}

/// Keys that reach `slot` of a code's list: the code, `0` per page turn, the cell key.
fn keys_for(code: Code, slot: usize) -> String {
    format!("{code}{}{}", "0".repeat(slot / PAGE), slot % PAGE + 1)
}

#[test]
fn every_harvested_candidate_is_typed_by_its_keys() {
    let Some(tsv) = read("zixing.tsv") else { return };
    let table = CodeTable::from_tsv(&tsv, "current").unwrap();
    assert_eq!(table.len(), 810);
    let mut e = Engine::new(table.clone(), SymbolTable::default());
    let mut checked = 0;
    for line in tsv.lines() {
        let cols: Vec<&str> = line.split('\t').collect();
        let (Some(code), Ok(page), Ok(key)) = (Code::parse(cols[1]), cols[2].parse::<usize>(), cols[3].parse::<usize>()) else {
            continue;
        };
        let slot = (page - 1) * PAGE + key - 1;
        // Committing from page 3 on would promote (自動調前) and shift later
        // slots, so check those through a fresh engine.
        if slot >= squareinput::PROMOTE_FROM {
            e = Engine::new(table.clone(), SymbolTable::default());
        }
        assert_eq!(press(&mut e, &keys_for(code, slot)), cols[4], "{code} page {page} key {key}");
        checked += 1;
    }
    assert!(checked > 21_000, "{checked}");
}

#[test]
fn manual_examples() {
    let Some(tsv) = read("zixing.tsv") else { return };
    let table = CodeTable::from_tsv(&tsv, "current").unwrap();
    let mut e = Engine::new(table, SymbolTable::default());
    // Known codes, plus the key to pick each.
    for (keys, want) in [("8101", "中"), ("2702", "九"), ("8922", "快"), ("4405", "方"), ("9875", "行"), ("7393", "髦"), ("3399", "峰")] {
        assert_eq!(press(&mut e, keys), want, "{keys}");
    }
}

#[test]
fn punctuation_and_symbols() {
    let Some(tsv) = read("punct.tsv") else { return };
    let symbols = SymbolTable::from_tsv(&tsv, "current").unwrap();
    let mut e = Engine::new(CodeTable::default(), symbols);
    assert_eq!(press(&mut e, "04"), "。");
    assert_eq!(press(&mut e, ".5"), "，");
    assert_eq!(press(&mut e, ".7"), "—");
    assert_eq!(press(&mut e, ".03"), "（");
    assert_eq!(press(&mut e, ".941"), "ㄅ");
    assert_eq!(press(&mut e, ".982"), "１");
}

#[test]
fn homophones_of_wo_cover_the_common_ones() {
    let (Some(tsv), Some(readings), Some(dict), Some(other)) = (
        read("zixing.tsv"),
        read(r"unihan\x\Unihan_Readings.txt"),
        read(r"unihan\x\Unihan_DictionaryLikeData.txt"),
        read(r"unihan\x\Unihan_OtherMappings.txt"),
    ) else {
        return;
    };
    let table = CodeTable::from_tsv(&tsv, "current").unwrap();
    let compact = compile_homophones(&table, &readings, &dict, &other);
    eprintln!("compact homophone list: {} chars, {} bytes", compact.lines().count(), compact.len());
    let h = Homophones::from_compact(&compact);
    let ours: Vec<String> = h.slots("我").into_iter().flatten().collect();
    // Common homophones of 我.
    let common = ["餓", "臥", "峨", "啊", "我", "哦", "鵝", "卧", "俄", "涐", "鋨", "硪", "訛", "娥", "蛾", "囮", "莪", "睋", "吪"];
    let shared = common.iter().filter(|c| ours.iter().any(|o| o == *c)).count();
    // They are fairly common characters; ours should list them first.
    let up_front = common.iter().filter(|c| ours.iter().take(common.len()).any(|o| o == *c)).count();
    eprintln!("我: ours {} chars, common 19, shared {shared}, {up_front} in our first 19; ours = {}", ours.len(), ours.concat());
    assert!(shared >= 15, "only {shared} of the 19 common homophones of 我");
    assert!(up_front >= 14, "only {up_front} of the common homophones of 我 are in our first 19");

    let mut e = Engine::new(table, SymbolTable::default()).with_homophones(h);
    press(&mut e, ".92");
    assert_eq!(press(&mut e, "9707"), "");
    let first = ours[0].clone();
    assert_eq!(press(&mut e, "1"), first);
}
