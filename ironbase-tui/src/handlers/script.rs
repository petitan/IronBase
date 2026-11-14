//! Key handlers: script

use crate::app::App;
use crossterm::event::{KeyCode, KeyModifiers};

/// Async script key handler
pub(crate) async fn handle_script_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    use crate::app::{ScriptConfirmAction, ScriptMode};

    // Handle confirmation dialog first if active
    if let Some(action) = app.script_state.confirm_action {
        match key {
            // Only an explicit 'y' confirms; Enter must not mean "yes" (see modals/confirm.rs)
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.script_state.confirm_action = None;
                match action {
                    ScriptConfirmAction::DiscardChanges => {
                        app.script_state.dirty = false;
                        app.script_state.reset_to_browse();
                    }
                    ScriptConfirmAction::DeleteScript => {
                        app.delete_script_async().await;
                    }
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Enter => {
                app.script_state.confirm_action = None;
            }
            _ => {}
        }
        return;
    }

    match app.script_state.mode {
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
            if app.script_state.selected_script > 0 => {
                app.script_state.selected_script -= 1;
            }
        (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE)
            if app.script_state.selected_script + 1 < app.script_state.scripts.len() => {
                app.script_state.selected_script += 1;
            }
        (KeyCode::Enter, _)
            // Open selected script for editing (load full script from MCP)
            if !app.script_state.scripts.is_empty() => {
                app.load_script_for_edit_async().await;
            }
        (KeyCode::Char('n'), _) => {
            // New script
            app.script_state.start_new();
        }
        (KeyCode::Char('d'), _) | (KeyCode::Delete, _)
            // Delete selected script (with confirmation)
            if !app.script_state.scripts.is_empty() => {
                app.script_state.confirm_action =
                    Some(crate::app::ScriptConfirmAction::DeleteScript);
            }
        (KeyCode::Char('s'), KeyModifiers::CONTROL)
            // Run selected script directly from browse (Ctrl+S)
            if !app.script_state.scripts.is_empty() => {
                // Load the script first, then run
                app.load_script_for_edit_async().await;
                app.run_script_async().await;
            }
        (KeyCode::Char('h'), _)
            // Show history for selected script
            if !app.script_state.scripts.is_empty() => {
                // First load the script to get the name
                app.load_script_for_edit_async().await;
                // Then load history
                app.load_script_history_async().await;
                app.script_state.enter_history();
            }
        (KeyCode::Char('i'), _) => {
            // Inline mode (ad-hoc script)
            app.script_state.enter_inline();
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
            if app.script_state.dirty {
                // Ask for confirmation before discarding changes
                app.script_state.confirm_action =
                    Some(crate::app::ScriptConfirmAction::DiscardChanges);
            } else {
                app.script_state.reset_to_browse();
            }
        }

        // Save & Run (Ctrl+S)
        (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
            // Save first (if not inline)
            if app.script_state.mode != crate::app::ScriptMode::Inline {
                app.save_script_async().await;
                app.load_scripts_async().await;
            }
            // Then run
            app.run_script_async().await;
        }

        // Tab - cycle focus
        (KeyCode::Tab, KeyModifiers::NONE) => {
            app.script_state.next_focus();
        }
        (KeyCode::BackTab, _) => {
            app.script_state.prev_focus();
        }

        // Focus-specific handling
        _ => match app.script_state.focus {
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
            app.script_state.mode = crate::app::ScriptMode::Edit;
            app.script_state.focus = crate::app::ScriptFocus::Editor;
        }
        KeyCode::Up | KeyCode::Char('k')
            if app.script_state.selected_version > 0 => {
                app.script_state.selected_version -= 1;
            }
        KeyCode::Down | KeyCode::Char('j')
            if app.script_state.selected_version + 1 < app.script_state.versions.len() => {
                app.script_state.selected_version += 1;
            }
        KeyCode::Enter => {
            // Load selected version into editor (view mode)
            if let Some(version) = app
                .script_state
                .versions
                .get(app.script_state.selected_version)
            {
                app.script_state.lines = version.code.lines().map(String::from).collect();
                if app.script_state.lines.is_empty() {
                    app.script_state.lines.push(String::new());
                }
                app.script_state.cursor_line = 0;
                app.script_state.cursor_col = 0;
                app.script_state.scroll_offset = 0;
                app.script_state.mode = crate::app::ScriptMode::Edit;
                app.script_state.focus = crate::app::ScriptFocus::Editor;
                app.script_state.message =
                    Some(format!("Verzió v{} betöltve (readonly)", version.version));
                app.script_state.dirty = true; // Mark as dirty so user knows it's not the current version
            }
        }
        KeyCode::Char('r')
            // Rollback to selected version (with confirmation)
            if !app.script_state.versions.is_empty() => {
                app.open_rollback_script_confirm();
            }
        _ => {}
    }
}

/// Handle name field keys
pub(crate) fn handle_script_name_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match (key, modifiers) {
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
            app.script_state.insert_char(c);
        }
        (KeyCode::Backspace, _) => {
            app.script_state.backspace();
        }
        (KeyCode::Left, _) => {
            app.script_state.cursor_left();
        }
        (KeyCode::Right, _) => {
            app.script_state.cursor_right();
        }
        (KeyCode::Enter, _) => {
            app.script_state.next_focus();
        }
        _ => {}
    }
}

/// Handle description field keys
pub(crate) fn handle_script_desc_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match (key, modifiers) {
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
            app.script_state.insert_char(c);
        }
        (KeyCode::Backspace, _) => {
            app.script_state.backspace();
        }
        (KeyCode::Left, _) => {
            app.script_state.cursor_left();
        }
        (KeyCode::Right, _) => {
            app.script_state.cursor_right();
        }
        (KeyCode::Enter, _) => {
            app.script_state.next_focus();
        }
        _ => {}
    }
}

/// Handle tags field keys (chips editor)
pub(crate) fn handle_script_tags_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    if app.script_state.tag_input_active {
        // Typing new tag
        match (key, modifiers) {
            (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
                app.script_state.tag_input.push(c);
            }
            (KeyCode::Backspace, _) => {
                if app.script_state.tag_input.is_empty() {
                    app.script_state.tag_input_active = false;
                } else {
                    app.script_state.tag_input.pop();
                }
            }
            (KeyCode::Enter, _) => {
                app.script_state.add_tag();
            }
            (KeyCode::Esc, _) => {
                app.script_state.tag_input_active = false;
                app.script_state.tag_input.clear();
            }
            _ => {}
        }
    } else {
        // Navigating tags
        match key {
            KeyCode::Left | KeyCode::Char('h')
                if app.script_state.selected_tag > 0 => {
                    app.script_state.selected_tag -= 1;
                }
            KeyCode::Right | KeyCode::Char('l')
                // +1 for the [+] button
                if app.script_state.selected_tag < app.script_state.tags.len() => {
                    app.script_state.selected_tag += 1;
                }
            KeyCode::Enter | KeyCode::Char(' ')
                if app.script_state.selected_tag == app.script_state.tags.len() => {
                    // [+] button - start adding new tag
                    app.script_state.tag_input_active = true;
                }
            KeyCode::Delete | KeyCode::Backspace | KeyCode::Char('x')
                // Delete selected tag
                if app.script_state.selected_tag < app.script_state.tags.len() => {
                    app.script_state.tags.remove(app.script_state.selected_tag);
                    if app.script_state.selected_tag > 0
                        && app.script_state.selected_tag >= app.script_state.tags.len()
                    {
                        app.script_state.selected_tag = app.script_state.tags.len();
                    }
                    app.script_state.dirty = true;
                }
            _ => {}
        }
    }
}

/// Handle editor (code) keys
pub(crate) fn handle_script_editor_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match (key, modifiers) {
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
            app.script_state.insert_char(c);
        }
        (KeyCode::Enter, _) => {
            app.script_state.insert_newline();
        }
        (KeyCode::Tab, KeyModifiers::NONE) => {
            // Insert 4 spaces as tab in editor
            app.script_state.insert_tab();
            app.script_state.insert_tab();
        }
        (KeyCode::Backspace, _) => {
            app.script_state.backspace();
        }
        (KeyCode::Delete, _) => {
            app.script_state.delete_char();
        }
        (KeyCode::Left, _) => {
            app.script_state.cursor_left();
        }
        (KeyCode::Right, _) => {
            app.script_state.cursor_right();
        }
        (KeyCode::Up, _) => {
            app.script_state.cursor_up();
        }
        (KeyCode::Down, _) => {
            app.script_state.cursor_down();
        }
        (KeyCode::Home, _) => {
            app.script_state.cursor_home();
        }
        (KeyCode::End, _) => {
            app.script_state.cursor_end();
        }
        (KeyCode::PageUp, _) => {
            for _ in 0..10 {
                app.script_state.cursor_up();
            }
        }
        (KeyCode::PageDown, _) => {
            for _ in 0..10 {
                app.script_state.cursor_down();
            }
        }
        _ => {}
    }
}

/// Handle params field keys
pub(crate) fn handle_script_params_key(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    match (key, modifiers) {
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
            app.script_state.insert_char(c);
        }
        (KeyCode::Backspace, _) => {
            app.script_state.backspace();
        }
        (KeyCode::Left, _) => {
            app.script_state.cursor_left();
        }
        (KeyCode::Right, _) => {
            app.script_state.cursor_right();
        }
        _ => {}
    }
}
