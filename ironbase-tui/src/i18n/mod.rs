//! UI language: every text the user reads comes from [`t()`], a
//! [`Messages`] implementation per language. A text is a trait method, so a
//! language that misses one does not compile, and a text with values in it
//! takes them as typed arguments.
//!
//! The language is process-wide (like a locale): it is set once at startup
//! from the config or the system, and read wherever text is drawn, without
//! threading it through every render function.

mod en;
mod hu;

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU8, Ordering};

pub use en::En;
pub use hu::Hu;

/// A UI language (`language = "hu"` / `"en"` in the config)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Hu,
    En,
}

impl Language {
    /// The language of a locale string such as `hu_HU.UTF-8`; English for
    /// anything but Hungarian
    pub fn from_locale(locale: &str) -> Self {
        if locale.to_ascii_lowercase().starts_with("hu") {
            Language::Hu
        } else {
            Language::En
        }
    }

    /// The system's language: the first set of `LC_ALL`, `LC_MESSAGES`,
    /// `LANG` (the POSIX order); English when none is set
    pub fn system() -> Self {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|name| std::env::var(name).ok())
            .find(|value| !value.is_empty())
            .map(|locale| Self::from_locale(&locale))
            .unwrap_or(Language::En)
    }
}

/// Hungarian until `set_language` (the texts the tests check are Hungarian)
static CURRENT: AtomicU8 = AtomicU8::new(Language::Hu as u8);

/// Switch the UI language (at startup)
pub fn set_language(language: Language) {
    CURRENT.store(language as u8, Ordering::Relaxed);
}

pub fn language() -> Language {
    if CURRENT.load(Ordering::Relaxed) == Language::En as u8 {
        Language::En
    } else {
        Language::Hu
    }
}

/// The texts of the current language
pub fn t() -> &'static dyn Messages {
    match language() {
        Language::Hu => &Hu,
        Language::En => &En,
    }
}

/// Every text of the UI, one method each
pub trait Messages: Sync {
    // === Header ===
    /// `IronBase v{version} | {db} ({collections} coll, {docs} docs)`
    fn header_db(&self, version: &str, db: &str, collections: usize, docs: usize) -> String;
    fn pane_collections(&self) -> &'static str;
    fn pane_documents(&self) -> &'static str;
    fn pane_detail(&self) -> &'static str;

    // === Command bar ===
    fn cmd_panel(&self) -> &'static str;
    fn cmd_navigate(&self) -> &'static str;
    fn cmd_select(&self) -> &'static str;
    fn cmd_page(&self) -> &'static str;
    fn cmd_scroll(&self) -> &'static str;
    fn cmd_refresh(&self) -> &'static str;
    fn cmd_filter(&self) -> &'static str;
    fn cmd_search(&self) -> &'static str;
    fn cmd_actions(&self) -> &'static str;
    fn cmd_edit(&self) -> &'static str;
    fn cmd_delete(&self) -> &'static str;
    fn cmd_copy(&self) -> &'static str;
    fn cmd_help(&self) -> &'static str;
    fn cmd_quit(&self) -> &'static str;
    /// The error shown in the command bar
    fn bar_error(&self, error: &str) -> String;
    fn bar_error_details(&self) -> &'static str;

    // === Panes ===
    fn collections_title(&self) -> &'static str;
    fn no_collections(&self) -> &'static str;
    fn indexes_title(&self) -> &'static str;
    fn no_indexes(&self) -> &'static str;
    fn documents_title(&self, page: usize, pages: usize, filtered: bool) -> String;
    fn no_documents(&self) -> &'static str;
    fn select_collection(&self) -> &'static str;
    /// The detail pane title; `matches` is (current, total) while searching
    fn detail_title(&self, matches: Option<(usize, usize)>) -> String;
    fn select_document(&self) -> &'static str;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_of_a_locale() {
        assert_eq!(Language::from_locale("hu_HU.UTF-8"), Language::Hu);
        assert_eq!(Language::from_locale("HU"), Language::Hu);
        assert_eq!(Language::from_locale("en_US.UTF-8"), Language::En);
        assert_eq!(Language::from_locale("C"), Language::En);
    }

    /// Both languages have every text, and they are not left untranslated
    #[test]
    fn test_both_languages_have_their_own_texts() {
        let (hu, en): (&dyn Messages, &dyn Messages) = (&Hu, &En);
        assert_ne!(hu.no_collections(), en.no_collections());
        assert_ne!(
            hu.documents_title(1, 2, true),
            en.documents_title(1, 2, true)
        );
        assert_eq!(en.detail_title(Some((2, 5))), " Details [2/5] ");
        assert_eq!(hu.detail_title(None), " Részletek ");
    }
}
