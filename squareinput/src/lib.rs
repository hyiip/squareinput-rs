//! Platform-independent core of a shape-based Chinese input method: the key
//! state machine, the code and symbol tables, auto-promotion and homophones.
//!
//! Nothing here touches a platform's input method framework. Each frontend
//! (the Windows TSF text service, the macOS input method) feeds the engine
//! numpad keys, draws the [`View`] it returns with [`ui::paint`], and keeps
//! its data through [`store`].

mod engine;
mod homophone;
pub mod labels;
mod learn;
mod pua;
pub mod store;
mod table;
pub mod ui;

pub use engine::{COMMON_CHARS, Cell, Engine, Key, Outcome, Screen, View};
pub use homophone::{Homophones, Layout, compile as compile_homophones, unihan_tiers};
pub use learn::{PROMOTE_FROM, PROMOTE_TO, Promotions};
pub use pua::{Cleaned, clean as clean_private_use};
pub use table::{BLANK, Code, CodeTable, PAGE, SymbolTable, TableError};
