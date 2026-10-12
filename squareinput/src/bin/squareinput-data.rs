//! Build the input method's data directory from the harvested tables and
//! Unihan: copies the 字形 and punctuation tables (after checking they load)
//! and compiles the compact 同音字 list.
//!
//! Usage: squareinput-data [--local DIR] [--out DIR]
//!   --local  harvest and Unihan files (default: local, beside the workspace)
//!   --out    data directory (default %APPDATA%\squareinput)
//!
//! The promotions log in the output directory is left alone.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use squareinput::{CodeTable, Homophones, SymbolTable, compile_homophones};

const TAG: &str = "current";

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn run() -> Result<(), String> {
    let mut local = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../local"));
    let mut out = std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("squareinput")).ok_or("APPDATA is not set")?;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next().ok_or(format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--local" => local = value.into(),
            "--out" => out = value.into(),
            _ => return Err(format!("unknown argument {flag}")),
        }
    }

    let zixing = read(&local.join("zixing.tsv"))?;
    let table = CodeTable::from_tsv(&zixing, TAG).map_err(|e| format!("zixing.tsv: {e}"))?;
    let punct = read(&local.join("punct.tsv"))?;
    let symbols = SymbolTable::from_tsv(&punct, TAG).map_err(|e| format!("punct.tsv: {e}"))?;
    if table.len() != 810 || symbols.slots(".").is_empty() {
        return Err(format!("unexpected tables: {} codes, {} punctuation slots", table.len(), symbols.slots(".").len()));
    }

    let unihan = local.join("unihan").join("x");
    let compact = compile_homophones(
        &table,
        &read(&unihan.join("Unihan_Readings.txt"))?,
        &read(&unihan.join("Unihan_DictionaryLikeData.txt"))?,
        &read(&unihan.join("Unihan_OtherMappings.txt"))?,
    );
    let homophones = Homophones::from_compact(&compact);

    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    write(&out.join("zixing.tsv"), &zixing)?;
    write(&out.join("punct.tsv"), &punct)?;
    write(&out.join("homophones.tsv"), &compact)?;
    println!(
        "wrote {}: {} codes, {} punctuation slots, 同音字 for {} characters ({} homophones of 我)",
        out.display(),
        table.len(),
        symbols.slots(".").len(),
        compact.lines().count(),
        homophones.slots("我").iter().flatten().count(),
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
