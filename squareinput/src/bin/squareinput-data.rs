//! Build the input method's data directory from the harvested tables, Unihan
//! and character frequency lists: copies the code table without its
//! private-use characters and the punctuation table (after checking they
//! load), and compiles the compact homophone list.
//!
//! Usage: squareinput-data [--local DIR] [--out DIR]
//!   --local  harvest and Unihan files (default: local, beside the workspace)
//!   --out    data directory (default: the input method's, see store::data_dir)
//!
//! The promotions log in the output directory is left alone.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use squareinput::{CodeTable, Homophones, SymbolTable, clean_private_use, compile_homophones};

const TAG: &str = "current";

/// Character frequency lists in `freq` under the local directory, with their
/// weights: Cantonese Wikipedia, and Common Voice zh-HK and yue sentences
/// (spoken Hong Kong Cantonese, but a far smaller sample, so it counts half).
const FREQUENCY_LISTS: [(&str, f64); 2] = [("zhyuewiki-chars.tsv", 1.0), ("commonvoice-chars.tsv", 0.5)];

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn run() -> Result<(), String> {
    let mut local = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../local"));
    let mut out = squareinput::store::data_dir().ok_or("no data directory (HOME or APPDATA is not set)")?;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next().ok_or(format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--local" => local = value.into(),
            "--out" => out = value.into(),
            _ => return Err(format!("unknown argument {flag}")),
        }
    }

    let codes = clean_private_use(&read(&local.join("codes.tsv"))?);
    let table = CodeTable::from_tsv(&codes.tsv, TAG).map_err(|e| format!("codes.tsv: {e}"))?;
    let punct = read(&local.join("punct.tsv"))?;
    let symbols = SymbolTable::from_tsv(&punct, TAG).map_err(|e| format!("punct.tsv: {e}"))?;
    if table.len() != 810 || symbols.slots(".").is_empty() {
        return Err(format!("unexpected tables: {} codes, {} punctuation slots", table.len(), symbols.slots(".").len()));
    }

    // Frequency lists are optional: without them, Unihan alone ranks homophones.
    let mut lists = Vec::new();
    for (name, weight) in FREQUENCY_LISTS {
        match read(&local.join("freq").join(name)) {
            Ok(text) => lists.push((text, weight)),
            Err(e) => println!("skipping frequency list: {e}"),
        }
    }
    let lists: Vec<(&str, f64)> = lists.iter().map(|(text, weight)| (text.as_str(), *weight)).collect();

    let unihan = local.join("unihan").join("x");
    let compact = compile_homophones(
        &table,
        &read(&unihan.join("Unihan_Readings.txt"))?,
        &read(&unihan.join("Unihan_DictionaryLikeData.txt"))?,
        &read(&unihan.join("Unihan_OtherMappings.txt"))?,
        &lists,
    );
    let homophones = Homophones::from_compact(&compact);

    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    write(&out.join("codes.tsv"), &codes.tsv)?;
    write(&out.join("punct.tsv"), &punct)?;
    write(&out.join("homophones.tsv"), &compact)?;
    println!(
        "wrote {}: {} codes, {} punctuation slots, homophones for {} characters ({} for 我)",
        out.display(),
        table.len(),
        symbols.slots(".").len(),
        compact.lines().count(),
        homophones.slots("我").iter().flatten().count(),
    );
    println!(
        "private-use characters: {} replaced, {} blanked on page 1, {} removed from later pages",
        codes.mapped, codes.blanked, codes.removed,
    );
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("squareinput-data: {e}");
            ExitCode::FAILURE
        }
    }
}
