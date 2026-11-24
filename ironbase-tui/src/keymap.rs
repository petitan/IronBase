//! The key map of the main screen and the actions menu: one table each,
//! read by the key handler, the help and the actions menu, so a key and its
//! description cannot drift apart.
//!
//! A letter is matched by the character it produces, not by the Shift flag:
//! `F` opens the full-text search whether it comes from Shift+F or from
//! CapsLock (some terminals do not report Shift for a capital letter).

use crate::state::Pane;
use crossterm::event::{KeyCode, KeyModifiers};

/// A key of a binding
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// A character, with or without Shift
    Char(char),
    /// Ctrl + a letter
    Ctrl(char),
    /// A non-character key (arrows, Tab, …)
    Code(KeyCode),
}

impl Key {
    pub fn matches(self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        match self {
            Key::Ctrl(c) => modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char(c),
            Key::Char(c) => code == KeyCode::Char(c),
            Key::Code(expected) => code == expected,
        }
    }

    /// The key as the help shows it: `Shift+F` for a capital letter
    pub fn label(self) -> String {
        match self {
            Key::Char(c) if c.is_ascii_uppercase() => format!("Shift+{c}"),
            Key::Char(c) => c.to_string(),
            Key::Ctrl(c) => format!("Ctrl+{}", c.to_ascii_uppercase()),
            Key::Code(code) => match code {
                KeyCode::Tab => "Tab".to_string(),
                KeyCode::BackTab => "Shift+Tab".to_string(),
                KeyCode::Up => "↑".to_string(),
                KeyCode::Down => "↓".to_string(),
                KeyCode::PageUp => "PgUp".to_string(),
                KeyCode::PageDown => "PgDn".to_string(),
                KeyCode::Home => "Home".to_string(),
                KeyCode::End => "End".to_string(),
                KeyCode::Esc => "Esc".to_string(),
                KeyCode::Enter => "Enter".to_string(),
                other => format!("{other:?}"),
            },
        }
    }
}

/// What a key does
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    NextPane,
    PrevPane,
    Up,
    Down,
    PageUp,
    PageDown,
    GoToStart,
    GoToEnd,
    ClearFilter,
    OpenSearch,
    OpenActions,
    OpenHelp,
    ServerInfo,
    UpdateCheck,
    Rag,
    Filter,
    Fulltext,
    VectorSearch,
    AclRules,
    Listeners,
    ApiKeys,
    ErrorDetails,
    RefreshCollections,
    Refresh,
    NextTheme,
    InsertDocument,
    EditDocument,
    DeleteDocument,
    CopyDocument,
    NextMatch,
    PrevMatch,
    DeleteCollection,
    EditAcl,
    // Only in the actions menu
    NewCollection,
    Indexes,
    Query,
    Export,
    Scripts,
    OpenDatabase,
}

/// A group of the help
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Navigation,
    General,
    Documents,
}

/// What a key's availability depends on
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Context {
    pub pane: Pane,
    pub has_error: bool,
    pub has_filter: bool,
    pub has_matches: bool,
}

impl Action {
    /// Whether the action applies in `ctx`; an action that does not leaves
    /// its key to the next binding (`e`: error details, else edit)
    pub fn available(self, ctx: &Context) -> bool {
        let document_pane = matches!(ctx.pane, Pane::Documents | Pane::Detail);
        match self {
            Action::ClearFilter => ctx.has_filter,
            Action::ErrorDetails => ctx.has_error,
            Action::EditDocument | Action::DeleteDocument | Action::CopyDocument => document_pane,
            Action::InsertDocument => matches!(ctx.pane, Pane::Collections | Pane::Documents),
            Action::NextMatch | Action::PrevMatch => ctx.pane == Pane::Detail && ctx.has_matches,
            Action::DeleteCollection | Action::EditAcl => ctx.pane == Pane::Collections,
            _ => true,
        }
    }
}

/// A row of the key map
#[derive(Debug, Clone, Copy)]
pub struct Binding {
    pub keys: &'static [Key],
    pub action: Action,
    pub section: Section,
}

const fn bind(keys: &'static [Key], action: Action, section: Section) -> Binding {
    Binding {
        keys,
        action,
        section,
    }
}

use Key::{Char, Code, Ctrl};
use Section::{Documents, General, Navigation};

/// The main screen's keys, in the order they are tried and listed
pub const MAIN: &[Binding] = &[
    bind(&[Code(KeyCode::Tab)], Action::NextPane, Navigation),
    bind(&[Code(KeyCode::BackTab)], Action::PrevPane, Navigation),
    bind(&[Code(KeyCode::Up), Char('k')], Action::Up, Navigation),
    bind(&[Code(KeyCode::Down), Char('j')], Action::Down, Navigation),
    bind(&[Code(KeyCode::PageUp)], Action::PageUp, Navigation),
    bind(&[Code(KeyCode::PageDown)], Action::PageDown, Navigation),
    bind(
        &[Code(KeyCode::Home), Char('g')],
        Action::GoToStart,
        Navigation,
    ),
    bind(
        &[Code(KeyCode::End), Char('G')],
        Action::GoToEnd,
        Navigation,
    ),
    bind(&[Code(KeyCode::Esc)], Action::ClearFilter, Navigation),
    bind(&[Ctrl('c'), Char('q')], Action::Quit, General),
    bind(&[Char('/')], Action::OpenSearch, General),
    bind(&[Char('a')], Action::OpenActions, General),
    bind(&[Char('?')], Action::OpenHelp, General),
    bind(&[Char('f')], Action::Filter, General),
    bind(&[Char('F')], Action::Fulltext, General),
    bind(&[Char('V')], Action::VectorSearch, General),
    bind(&[Char('E')], Action::Rag, General),
    bind(&[Char('r')], Action::Refresh, General),
    bind(&[Char('R')], Action::RefreshCollections, General),
    bind(&[Char('e')], Action::ErrorDetails, General),
    bind(&[Char('t')], Action::NextTheme, General),
    bind(&[Char('I')], Action::ServerInfo, General),
    bind(&[Char('U')], Action::UpdateCheck, General),
    bind(&[Char('K')], Action::ApiKeys, General),
    bind(&[Char('A')], Action::AclRules, General),
    bind(&[Char('L')], Action::Listeners, General),
    bind(&[Char('i')], Action::InsertDocument, Documents),
    bind(&[Char('e')], Action::EditDocument, Documents),
    bind(&[Char('d')], Action::DeleteDocument, Documents),
    bind(&[Char('y')], Action::CopyDocument, Documents),
    bind(&[Char('n')], Action::NextMatch, Documents),
    bind(&[Char('N')], Action::PrevMatch, Documents),
    bind(&[Char('D')], Action::DeleteCollection, Documents),
    bind(&[Char('P')], Action::EditAcl, Documents),
];

/// The actions menu: its own keys (a menu key need not be the main one)
pub const ACTIONS_MENU: &[(char, Action)] = &[
    ('i', Action::InsertDocument),
    ('e', Action::EditDocument),
    ('d', Action::DeleteDocument),
    ('y', Action::CopyDocument),
    ('c', Action::NewCollection),
    ('D', Action::DeleteCollection),
    ('x', Action::Indexes),
    ('r', Action::Query),
    ('f', Action::Filter),
    ('o', Action::Export),
    ('s', Action::Scripts),
    ('b', Action::OpenDatabase),
];

/// The main screen's action for a key, if any applies in `ctx`
pub fn resolve(code: KeyCode, modifiers: KeyModifiers, ctx: &Context) -> Option<Action> {
    MAIN.iter()
        .filter(|binding| binding.keys.iter().any(|key| key.matches(code, modifiers)))
        .map(|binding| binding.action)
        .find(|action| action.available(ctx))
}

/// The actions menu's action for a key
pub fn menu_action(code: KeyCode) -> Option<Action> {
    ACTIONS_MENU
        .iter()
        .find(|(key, _)| code == KeyCode::Char(*key))
        .map(|(_, action)| *action)
}

/// The keys of a binding as the help shows them: `↑/k`
pub fn keys_label(keys: &[Key]) -> String {
    keys.iter()
        .map(|key| key.label())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(pane: Pane) -> Context {
        Context {
            pane,
            has_error: false,
            has_filter: false,
            has_matches: false,
        }
    }

    /// A capital letter works with or without the Shift flag (CapsLock)
    #[test]
    fn test_capital_letters_match_without_the_shift_flag() {
        let c = ctx(Pane::Collections);
        for modifiers in [KeyModifiers::SHIFT, KeyModifiers::NONE] {
            assert_eq!(
                resolve(KeyCode::Char('F'), modifiers, &c),
                Some(Action::Fulltext)
            );
            assert_eq!(
                resolve(KeyCode::Char('D'), modifiers, &c),
                Some(Action::DeleteCollection)
            );
        }
    }

    /// `e` opens the error details while there is an error, else edits
    #[test]
    fn test_a_key_falls_through_to_the_next_available_binding() {
        let mut c = ctx(Pane::Documents);
        assert_eq!(
            resolve(KeyCode::Char('e'), KeyModifiers::NONE, &c),
            Some(Action::EditDocument)
        );
        c.has_error = true;
        assert_eq!(
            resolve(KeyCode::Char('e'), KeyModifiers::NONE, &c),
            Some(Action::ErrorDetails)
        );
        // Nothing applies: no action (Collections pane, no error)
        assert_eq!(
            resolve(
                KeyCode::Char('e'),
                KeyModifiers::NONE,
                &ctx(Pane::Collections)
            ),
            None
        );
        // Esc clears a filter only when there is one
        assert_eq!(resolve(KeyCode::Esc, KeyModifiers::NONE, &c), None);
        c.has_filter = true;
        assert_eq!(
            resolve(KeyCode::Esc, KeyModifiers::NONE, &c),
            Some(Action::ClearFilter)
        );
    }

    #[test]
    fn test_ctrl_c_quits_and_plain_c_does_nothing() {
        let c = ctx(Pane::Collections);
        assert_eq!(
            resolve(KeyCode::Char('c'), KeyModifiers::CONTROL, &c),
            Some(Action::Quit)
        );
        assert_eq!(resolve(KeyCode::Char('c'), KeyModifiers::NONE, &c), None);
    }

    /// No key of the main table is listed twice for the same context unless
    /// the availability conditions tell them apart
    #[test]
    fn test_shared_keys_have_distinct_conditions() {
        let contexts = [Pane::Collections, Pane::Documents, Pane::Detail].map(|pane| Context {
            pane,
            has_error: true,
            has_filter: true,
            has_matches: true,
        });
        for (i, a) in MAIN.iter().enumerate() {
            for b in &MAIN[i + 1..] {
                if !a.keys.iter().any(|key| b.keys.contains(key)) {
                    continue;
                }
                // The later one must be reachable somewhere the earlier is not
                let reachable = contexts
                    .iter()
                    .chain(
                        contexts
                            .map(|c| Context {
                                has_error: false,
                                ..c
                            })
                            .iter(),
                    )
                    .any(|c| b.action.available(c) && !a.action.available(c));
                assert!(reachable, "{:?} hides {:?}", a.action, b.action);
            }
        }
    }

    #[test]
    fn test_key_labels() {
        assert_eq!(keys_label(&[Code(KeyCode::Up), Char('k')]), "↑/k");
        assert_eq!(keys_label(&[Ctrl('c'), Char('q')]), "Ctrl+C/q");
        assert_eq!(Char('F').label(), "Shift+F");
    }

    #[test]
    fn test_menu_keys_are_unique() {
        for (i, (key, _)) in ACTIONS_MENU.iter().enumerate() {
            assert!(!ACTIONS_MENU[i + 1..].iter().any(|(other, _)| other == key));
        }
        assert_eq!(menu_action(KeyCode::Char('x')), Some(Action::Indexes));
        assert_eq!(menu_action(KeyCode::Char('z')), None);
    }
}
