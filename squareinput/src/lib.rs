//! Platform-independent core of a 字形 input method: the key
//! state machine, the code and symbol tables, 自動調前 learning and 同音字.
//!
//! Nothing here touches Windows. The TSF text service feeds it numpad keys
//! and draws the [`View`] it returns.

mod engine;
mod homophone;
mod learn;
mod pua;
mod table;

pub use engine::{COMMON_CHARS, Cell, Engine, Key, Outcome, Screen, View};
pub use homophone::{Homophones, compile as compile_homophones, unihan_tiers};
pub use learn::{PROMOTE_FROM, PROMOTE_TO, Promotions};
pub use pua::{Cleaned, clean as clean_private_use};
pub use table::{BLANK, Code, CodeTable, PAGE, SymbolTable, TableError};
