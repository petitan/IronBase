//! Key handlers: script

use crate::app::App;
use crate::i18n::t;
use crate::state::open_state;
use crate::widgets::Input;
use crossterm::event::{KeyCode, KeyModifiers};

/// Async script key handler
pub(crate) async fn handle_script_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    use crate::app::{ScriptConfirmAction, ScriptMode};

    // Handle confirmation dialog first if active
    if let Some(action) = open_state!(app, Script).confirm_action {
        match key {
            // Only an explicit 'y' confirms; Enter must not mean "yes" (see modals/confirm.rs)
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                open_state!(app, Script).confirm_action = None;
                match action {
                    ScriptConfirmAction::DiscardChanges => {
                        open_state!(app, Script).dirty = false;
                        open_state!(app, Script).reset_to_browse();
                    }
                    ScriptConfirmAction::DeleteScript => {
                        app.delete_script_async().await;
                    }
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Enter => {
                open_state!(app, Script).confirm_action = None;
            }
            _ => {}
        }
        return;
    }

    match open_state!(app, Script).mode {
        ScriptMode::Browse => handle_script_browse_key(app, key, modifiers).await,
        ScriptMode::Edit | ScriptMode::New | ScriptMode::Inline => {
            handle_script_edit_key(app, key, modifiers).await
        }
        ScriptMode::History => handle_script_history_key(app, key).await,
    }
}

/// Handle keys in Browse mode (script list)
pub(crate) async fn handle_script_browse_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match (key, modifiers) {
        (KeyCode::Esc, _) | (KeyCode::Char('q'), _) => app.close_modal(),
        (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE)
            if open_state!(app, Script).selected_script > 0 => {
                open_state!(app, Script).selected_script -= 1;
            }
        (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE)
            if open_state!(app, Script).selected_script + 1 < open_state!(app, Script).scripts.len() => {
                open_state!(app, Script).selected_script += 1;
            }
        (KeyCode::Enter, _)
            // Open selected script for editing (load full script from MCP)
            if !open_state!(app, Script).scripts.is_empty() => {
                app.load_script_for_edit_async().await;
            }
        (KeyCode::Char('n'), _) => {
            // New script
            open_state!(app, Script).start_new();
        }
        (KeyCode::Char('d'), _) | (KeyCode::Delete, _)
            // Delete selected script (with confirmation)
            if !open_state!(app, Script).scripts.is_empty() => {
                open_state!(app, Script).confirm_action =
                    Some(crate::app::ScriptConfirmAction::DeleteScript);
            }
        (KeyCode::Char('s'), KeyModifiers::CONTROL)
            // Run selected script directly from browse (Ctrl+S)
            if !open_state!(app, Script).scripts.is_empty() => {
                // Load the script first, then run
                app.load_script_for_edit_async().await;
                app.run_script_async().await;
            }
        (KeyCode::Char('h'), _)
            // Show history for selected script
            if !open_state!(app, Script).scripts.is_empty() => {
                // First load the script to get the name
                app.load_script_for_edit_async().await;
                // Then load history
                app.load_script_history_async().await;
                open_state!(app, Script).enter_history();
            }
        (KeyCode::Char('i'), _) => {
            // Inline mode (ad-hoc script)
            open_state!(app, Script).enter_inline();
        }
        (KeyCode::Char('r'), _) => {
            // Refresh script list
            app.load_scripts_async().await;
        }
        _ => {}
    }
}

/// Handle keys in Edit/New/Inline mode
pub(crate) async fn handle_script_edit_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    use crate::app::ScriptFocus;

    match (key, modifiers) {
        // Exit/Back
        (KeyCode::Esc, _) => {
            if open_state!(app, Script).dirty {
                // Ask for confirmation before discarding changes
                open_state!(app, Script).confirm_action =
                    Some(crate::app::ScriptConfirmAction::DiscardChanges);
            } else {
                open_state!(app, Script).reset_to_browse();
            }
        }

        // Save & Run (Ctrl+S)
        (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
            // Save first (if not inline)
            if open_state!(app, Script).mode != crate::app::ScriptMode::Inline {
                app.save_script_async().await;
                app.load_scripts_async().await;
            }
            // Then run
            app.run_script_async().await;
        }

        // Tab - cycle focus
        (KeyCode::Tab, KeyModifiers::NONE) => {
            open_state!(app, Script).next_focus();
        }
        (KeyCode::BackTab, _) => {
            open_state!(app, Script).prev_focus();
        }

        // Focus-specific handling
        _ => match open_state!(app, Script).focus {
            ScriptFocus::Name => handle_script_name_key(app, key, modifiers),
            ScriptFocus::Description => handle_script_desc_key(app, key, modifiers),
            ScriptFocus::Tags => handle_script_tags_key(app, key, modifiers),
            ScriptFocus::Editor => handle_script_editor_key(app, key, modifiers),
            ScriptFocus::Params => handle_script_params_key(app, key, modifiers),
            _ => {}
        },
    }
}

/// Handle keys in History mode
pub(crate) async fn handle_script_history_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc | KeyCode::Char('q') => {
            // Back to edit mode
            open_state!(app, Script).mode = crate::app::ScriptMode::Edit;
            open_state!(app, Script).focus = crate::app::ScriptFocus::Editor;
        }
        KeyCode::Up | KeyCode::Char('k')
            if open_state!(app, Script).selected_version > 0 => {
                open_state!(app, Script).selected_version -= 1;
            }
        KeyCode::Down | KeyCode::Char('j')
            if open_state!(app, Script).selected_version + 1 < open_state!(app, Script).versions.len() => {
                open_state!(app, Script).selected_version += 1;
            }
        KeyCode::Enter => {
            // Load selected version into editor (view mode)
            let state = open_state!(app, Script);
            if let Some(version) = state.versions.get(state.selected_version) {
                let (code, number) = (version.code.clone(), version.version);
                state.editor = crate::widgets::TextArea::new(&code);
                state.mode = crate::app::ScriptMode::Edit;
                state.focus = crate::app::ScriptFocus::Editor;
                state.message = Some(t().version_loaded_readonly(number));
                state.dirty = true; // Mark as dirty so user knows it's not the current version
            }
        }
        KeyCode::Char('r')
            // Rollback to selected version (with confirmation)
            if !open_state!(app, Script).versions.is_empty() => {
                app.open_rollback_script_confirm();
            }
        _ => {}
    }
}

/// Handle name field keys
pub(crate) fn handle_script_name_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match key {
        KeyCode::Enter => open_state!(app, Script).next_focus(),
        _ => {
            open_state!(app, Script).handle_field_input(Input::Key(key, modifiers));
        }
    }
}

/// Handle description field keys
pub(crate) fn handle_script_desc_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match key {
        KeyCode::Enter => open_state!(app, Script).next_focus(),
        _ => {
            open_state!(app, Script).handle_field_input(Input::Key(key, modifiers));
        }
    }
}

/// Handle tags field keys (chips editor)
pub(crate) fn handle_script_tags_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    if open_state!(app, Script).tag_input_active {
        // Typing new tag
        match key {
            // Backspace on an empty tag leaves the input
            KeyCode::Backspace if open_state!(app, Script).tag_input.is_empty() => {
                open_state!(app, Script).tag_input_active = false;
            }
            KeyCode::Enter => {
                open_state!(app, Script).add_tag();
            }
            _ => {
                open_state!(app, Script).handle_field_input(Input::Key(key, modifiers));
            }
        }
    } else {
        // Navigating tags
        match key {
            KeyCode::Left | KeyCode::Char('h')
                if open_state!(app, Script).selected_tag > 0 => {
                    open_state!(app, Script).selected_tag -= 1;
                }
            KeyCode::Right | KeyCode::Char('l')
                // +1 for the [+] button
                if open_state!(app, Script).selected_tag < open_state!(app, Script).tags.len() => {
                    open_state!(app, Script).selected_tag += 1;
                }
            KeyCode::Enter | KeyCode::Char(' ')
                if open_state!(app, Script).selected_tag == open_state!(app, Script).tags.len() => {
                    // [+] button - start adding new tag
                    open_state!(app, Script).tag_input_active = true;
                }
            KeyCode::Delete | KeyCode::Backspace | KeyCode::Char('x')
                // Delete selected tag
                if open_state!(app, Script).selected_tag < open_state!(app, Script).tags.len() => {
                    let state = open_state!(app, Script);
                    state.tags.remove(state.selected_tag);
                    // The last tag went: the one before it (not the [+] button)
                    if state.selected_tag >= state.tags.len() {
                        state.selected_tag = state.tags.len().saturating_sub(1);
                    }
                    state.dirty = true;
                }
            _ => {}
        }
    }
}

/// Handle editor (code) keys
pub(crate) fn handle_script_editor_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    open_state!(app, Script).handle_editor_input(Input::Key(key, modifiers));
}

/// Handle params field keys
pub(crate) fn handle_script_params_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    open_state!(app, Script).handle_field_input(Input::Key(key, modifiers));
}
