//! Phosphor line icons (regular weight), from the bundled `Phosphor.ttf`.
//!
//! The font sits right after the text font in every family, so these can be
//! mixed into any label, e.g. `format!("{} Add paper", icons::PLUS)`.

pub const ARCHIVE: &str = "\u{E00C}";
pub const ARROW_LEFT: &str = "\u{E058}";
pub const ARROW_RIGHT: &str = "\u{E06C}";
pub const ARROW_SQUARE_OUT: &str = "\u{E5DE}";
pub const ARROWS_CLOCKWISE: &str = "\u{E094}";
pub const BOOK_OPEN_TEXT: &str = "\u{E8F2}";
pub const BOOKS: &str = "\u{E758}";
pub const BRACKETS_CURLY: &str = "\u{E860}";
pub const CARET_DOWN: &str = "\u{E136}";
pub const CARET_RIGHT: &str = "\u{E13A}";
pub const CARET_UP: &str = "\u{E13C}";
pub const CHECK: &str = "\u{E182}";
pub const CHECK_CIRCLE: &str = "\u{E184}";
pub const CIRCLE_DASHED: &str = "\u{E602}";
pub const CIRCLE_HALF: &str = "\u{E18C}";
pub const CLOCK: &str = "\u{E19A}";
pub const COLUMNS: &str = "\u{E546}";
pub const COPY: &str = "\u{E1CA}";
pub const DATABASE: &str = "\u{E1DE}";
pub const DOTS_SIX_VERTICAL: &str = "\u{EAE2}";
pub const DOWNLOAD_SIMPLE: &str = "\u{E20C}";
pub const EXPORT: &str = "\u{EAF0}";
pub const EYE: &str = "\u{E220}";
pub const FILE_PDF: &str = "\u{E702}";
pub const FILE_PLUS: &str = "\u{E236}";
pub const FILE_TEXT: &str = "\u{E23A}";
pub const FUNNEL: &str = "\u{E266}";
pub const FOLDER_OPEN: &str = "\u{E256}";
pub const FOLDER_SIMPLE_PLUS: &str = "\u{E25E}";
pub const GEAR_SIX: &str = "\u{E272}";
pub const GLOBE: &str = "\u{E288}";
pub const GRAPH: &str = "\u{EB58}";
pub const HOUSE: &str = "\u{E2C2}";
pub const INFO: &str = "\u{E2CE}";
pub const KEY: &str = "\u{E2D6}";
pub const KEYBOARD: &str = "\u{E2D8}";
pub const LIGHTBULB: &str = "\u{E2DC}";
pub const LINK: &str = "\u{E2E2}";
pub const LINK_BREAK: &str = "\u{E2E4}";
pub const MAGIC_WAND: &str = "\u{E6B6}";
pub const MAGNIFYING_GLASS: &str = "\u{E30C}";
pub const MOON: &str = "\u{E330}";
pub const NOTE_PENCIL: &str = "\u{E34C}";
pub const PENCIL_SIMPLE: &str = "\u{E3B4}";
pub const PLUS: &str = "\u{E3D4}";
pub const PUSH_PIN: &str = "\u{E3E2}";
pub const QUESTION: &str = "\u{E3E8}";
pub const QUOTES: &str = "\u{E660}";
pub const SORT_ASCENDING: &str = "\u{E444}";
pub const SORT_DESCENDING: &str = "\u{E446}";
pub const SPARKLE: &str = "\u{E6A2}";
pub const SUN: &str = "\u{E472}";
pub const TABLE: &str = "\u{E476}";
pub const TAG: &str = "\u{E478}";
pub const TRASH: &str = "\u{E4A6}";
pub const TREE_STRUCTURE: &str = "\u{E67C}";
pub const TRAY: &str = "\u{E4AA}";
pub const ARROW_COUNTER_CLOCKWISE: &str = "\u{E038}";
pub const WARNING: &str = "\u{E4E0}";
pub const X: &str = "\u{E4F6}";
pub const X_CIRCLE: &str = "\u{E4F8}";

#[cfg(test)]
pub const ALL: &[&str] = &[
    ARCHIVE, ARROW_LEFT, ARROW_SQUARE_OUT, ARROWS_CLOCKWISE, BOOK_OPEN_TEXT, BOOKS, BRACKETS_CURLY, CARET_DOWN,
    CARET_RIGHT, CARET_UP, CHECK, CHECK_CIRCLE, CIRCLE_DASHED, CIRCLE_HALF, CLOCK, COLUMNS, COPY, DATABASE,
    DOTS_SIX_VERTICAL, DOWNLOAD_SIMPLE, EXPORT, EYE, FILE_PDF, FILE_PLUS, FILE_TEXT, FOLDER_OPEN,
    FOLDER_SIMPLE_PLUS, GEAR_SIX, GLOBE, GRAPH, HOUSE, INFO, KEY, LINK, LINK_BREAK, MAGIC_WAND,
    MAGNIFYING_GLASS, MOON, NOTE_PENCIL, PENCIL_SIMPLE, PLUS, PUSH_PIN, QUOTES, SORT_ASCENDING,
    SORT_DESCENDING, SUN, TABLE, TAG, TRASH, TRAY, ARROW_COUNTER_CLOCKWISE, WARNING, X, ARROW_RIGHT, FUNNEL,
    LIGHTBULB, QUESTION, SPARKLE, TREE_STRUCTURE, X_CIRCLE, KEYBOARD,
];
