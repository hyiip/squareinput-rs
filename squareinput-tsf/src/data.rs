//! Tables and learning data under `%APPDATA%\squareinput` (see [`data_dir`]):
//!
//! - `zixing.tsv`, `punct.tsv`: the harvested tables (harvest TSV format)
//! - `homophones.tsv`: the compact 同音字 list made by `squareinput-data`
//! - `promotions.tsv`: the 自動調前 log, appended to by every process
//!
//! Tables are read once per process and shared by all its threads. The
//! promotions log is re-read whenever a thread starts typing, so moves made in
//! other applications are picked up.
//!
//! Errors go to `%LOCALAPPDATA%\squareinput\squareinput.log`; without data the input
//! method stays inert and lets every key through.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use squareinput::{CodeTable, Engine, Homophones, Promotions, SymbolTable};

/// Rows of the harvest TSVs to use.
const TAG: &str = "current";

struct Data {
    table: CodeTable,
    symbols: Arc<SymbolTable>,
    homophones: Arc<Homophones>,
}

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(|d| PathBuf::from(d).join("squareinput"))
}

/// The data directory: `SQUAREINPUT_DATA` if set (the test tools point it at a
/// scratch copy so they never touch the real 自動調前 log), else `%APPDATA%\squareinput`.
fn data_dir() -> Option<PathBuf> {
    std::env::var_os("SQUAREINPUT_DATA").map(PathBuf::from).or_else(|| env_dir("APPDATA"))
}

/// Log only when the SQUAREINPUT_DEBUG environment variable is set (the test tools set it).
pub fn debug(message: impl FnOnce() -> String) {
    static ON: OnceLock<bool> = OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("SQUAREINPUT_DEBUG").is_some()) {
        log(&message());
    }
}

pub fn log(message: &str) {
    let Some(dir) = env_dir("LOCALAPPDATA") else { return };
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(dir.join("squareinput.log")) {
        let exe = std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
        let _ = writeln!(f, "[{}] {message}", exe.unwrap_or_default());
    }
}

fn load() -> Result<Data, String> {
    let dir = data_dir().ok_or("APPDATA is not set")?;
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).map_err(|e| format!("{name}: {e}"));
    let table = CodeTable::from_tsv(&read("zixing.tsv")?, TAG).map_err(|e| format!("zixing.tsv: {e}"))?;
    let symbols = SymbolTable::from_tsv(&read("punct.tsv")?, TAG).map_err(|e| format!("punct.tsv: {e}"))?;
    // 同音字 is optional.
    let homophones = read("homophones.tsv").map(|s| Homophones::from_compact(&s)).unwrap_or_default();
    Ok(Data { table, symbols: Arc::new(symbols), homophones: Arc::new(homophones) })
}

fn data() -> Option<&'static Data> {
    static DATA: OnceLock<Option<Data>> = OnceLock::new();
    DATA.get_or_init(|| load().inspect_err(|e| log(&format!("cannot load data: {e}"))).ok()).as_ref()
}

/// A fresh engine with the current promotions applied, or None without data.
pub fn new_engine() -> Option<Engine> {
    let data = data()?;
    let promotions = data_dir()
        .and_then(|d| std::fs::read_to_string(d.join("promotions.tsv")).ok())
        .map(|s| Promotions::parse(&s))
        .unwrap_or_default();
    Some(
        Engine::new(data.table.clone(), data.symbols.clone())
            .with_homophones(data.homophones.clone())
            .with_promotions(promotions),
    )
}

/// A one-line setting file in the data directory, e.g. `corner`.
pub fn read_setting(name: &str) -> Option<String> {
    let text = std::fs::read_to_string(data_dir()?.join(name)).ok()?;
    Some(text.trim().to_owned())
}

pub fn write_setting(name: &str, value: &str) {
    let Some(dir) = data_dir() else { return };
    if let Err(e) = std::fs::write(dir.join(name), value) {
        log(&format!("cannot save {name}: {e}"));
    }
}

/// Append one 自動調前 move to the shared log.
pub fn save_promotion(line: &str) {
    let Some(dir) = data_dir() else { return };
    let result = OpenOptions::new().create(true).append(true).open(dir.join("promotions.tsv")).and_then(|mut f| f.write_all(line.as_bytes()));
    if let Err(e) = result {
        log(&format!("cannot save promotion: {e}"));
    }
}
