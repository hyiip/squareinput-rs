//! The input method's data directory (see [`data_dir`]) and what lives in it:
//!
//! - `codes.tsv`, `punct.tsv`: the tables (harvest TSV format)
//! - `homophones.tsv`: the compact homophone list made by `squareinput-data`
//! - `associations.tsv`: the compact list of associated characters, likewise
//! - `promotions.tsv`: the auto-promotion log, appended to by every process
//! - one-line settings such as `corner` and `mode`
//!
//! Tables are read once per process and shared by all its threads. The
//! promotions log is re-read for every new engine, so moves made in other
//! processes are picked up.
//!
//! Errors go to `squareinput.log` in [`log_dir`]; without data the input
//! method stays inert and lets every key through.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use crate::{Associations, CodeTable, Engine, Homophones, Promotions, SymbolTable};

/// Rows of the harvest TSVs to use.
const TAG: &str = "current";
const NAME: &str = "squareinput";

struct Data {
    table: CodeTable,
    symbols: Arc<SymbolTable>,
    homophones: Arc<Homophones>,
    associations: Arc<Associations>,
}

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn home(rest: &str) -> Option<PathBuf> {
    env_path("HOME").map(|h| h.join(rest))
}

/// The data directory: `SQUAREINPUT_DATA` if set (the test tools point it at
/// a scratch copy so they never touch the real auto-promotion log), else
/// `%APPDATA%\squareinput` on Windows, `~/Library/Application
/// Support/squareinput` on macOS and `$XDG_DATA_HOME/squareinput` (by
/// default `~/.local/share/squareinput`) elsewhere.
pub fn data_dir() -> Option<PathBuf> {
    if let Some(dir) = env_path("SQUAREINPUT_DATA") {
        return Some(dir);
    }
    let base = if cfg!(windows) {
        env_path("APPDATA")
    } else if cfg!(target_os = "macos") {
        home("Library/Application Support")
    } else {
        env_path("XDG_DATA_HOME").or_else(|| home(".local/share"))
    };
    base.map(|b| b.join(NAME))
}

/// Where the log goes: `%LOCALAPPDATA%\squareinput` on Windows,
/// `~/Library/Logs/squareinput` on macOS and `$XDG_STATE_HOME/squareinput`
/// (by default `~/.local/state/squareinput`) elsewhere.
pub fn log_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        env_path("LOCALAPPDATA")
    } else if cfg!(target_os = "macos") {
        home("Library/Logs")
    } else {
        env_path("XDG_STATE_HOME").or_else(|| home(".local/state"))
    };
    base.map(|b| b.join(NAME))
}

/// Log only when the SQUAREINPUT_DEBUG environment variable is set (the test tools set it).
pub fn debug(message: impl FnOnce() -> String) {
    static ON: OnceLock<bool> = OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("SQUAREINPUT_DEBUG").is_some()) {
        log(&message());
    }
}

pub fn log(message: &str) {
    let Some(dir) = log_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(dir.join("squareinput.log")) {
        let exe = std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
        let _ = writeln!(f, "[{}] {message}", exe.unwrap_or_default());
    }
}

fn load() -> Result<Data, String> {
    let dir = data_dir().ok_or("no data directory (HOME or APPDATA is not set)")?;
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).map_err(|e| format!("{name}: {e}"));
    let table = CodeTable::from_tsv(&read("codes.tsv")?, TAG).map_err(|e| format!("codes.tsv: {e}"))?;
    let symbols = SymbolTable::from_tsv(&read("punct.tsv")?, TAG).map_err(|e| format!("punct.tsv: {e}"))?;
    // Homophones and associated characters are optional.
    let homophones = read("homophones.tsv").map(|s| Homophones::from_compact(&s)).unwrap_or_default();
    let associations = read("associations.tsv").map(|s| Associations::from_compact(&s)).unwrap_or_default();
    Ok(Data { table, symbols: Arc::new(symbols), homophones: Arc::new(homophones), associations: Arc::new(associations) })
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
            .with_associations(data.associations.clone())
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

/// Setting holding the mode, "chinese" or "english".
const MODE: &str = "mode";

/// Whether the user has switched to English with a tap of Shift (see
/// [`crate::ShiftTap`]). Frontends re-read it when focus moves, so a switch
/// in one application carries over to the next.
pub fn english() -> bool {
    read_setting(MODE).as_deref() == Some("english")
}

pub fn save_english(english: bool) {
    write_setting(MODE, if english { "english" } else { "chinese" });
}

/// Append one auto-promotion move to the shared log.
pub fn save_promotion(line: &str) {
    let Some(dir) = data_dir() else { return };
    let result = OpenOptions::new().create(true).append(true).open(dir.join("promotions.tsv")).and_then(|mut f| f.write_all(line.as_bytes()));
    if let Err(e) = result {
        log(&format!("cannot save promotion: {e}"));
    }
}
