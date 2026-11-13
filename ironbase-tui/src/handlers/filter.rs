//! Key handlers: filter

use crate::app::App;
use crossterm::event::{KeyCode, KeyModifiers};

/// Async filter key handler
pub(crate) async fn handle_filter_key_async(app: &mut App, key: KeyCode, modifiers: KeyModifiers) {
    use crate::app::FilterFocus;

    match (key, modifiers) {
        // Close modal
        (KeyCode::Esc, _) => app.close_modal(),

        // Execute filter search (Ctrl+S)
        (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
            app.execute_filter_async().await;
        }

        // Tab - apply suggestion if showing, otherwise cycle between fields
        (KeyCode::Tab, KeyModifiers::NONE) => {
            if app.filter_state.focus == FilterFocus::Field && app.filter_state.show_suggestions {
                // Apply suggestion and move to operator
                app.filter_state.apply_suggestion();
                app.filter_state.focus = FilterFocus::Operator;
            } else {
                app.filter_state.next_focus();
            }
        }
        (KeyCode::BackTab, _) => {
            app.filter_state.prev_focus();
        }

        // Enter - navigate or add filter
        (KeyCode::Enter, _) => {
            match app.filter_state.focus {
                FilterFocus::Field => {
                    // Ha van suggestion látható, alkalmazzuk és lépjünk tovább
                    if app.filter_state.show_suggestions
                        && !app.filter_state.filtered_suggestions.is_empty()
                    {
                        app.filter_state.apply_suggestion();
                        app.filter_state.focus = FilterFocus::Operator;
                    } else if !app.filter_state.field_input.is_empty() {
                        // Field-nél Enter = lépj Operator-ra, DE csak ha nem üres!
                        app.filter_state.focus = FilterFocus::Operator;
                    }
                }
                FilterFocus::Operator => {
                    // Operator-nál Enter = lépj Value-ra
                    app.filter_state.focus = FilterFocus::Value;
                }
                FilterFocus::Value => {
                    // Value-nál Enter = add filter (ha valid) és vissza Field-re
                    // Exists operátorhoz nem kell érték, máshoz igen
                    let needs_value =
                        app.filter_state.operator != crate::app::FilterOperator::Exists;
                    let has_value = !app.filter_state.value_input.is_empty();

                    if !needs_value || has_value {
                        app.filter_state.add_filter();
                    } else {
                        app.filter_state.error = Some("Ertek megadasa kotelezo!".to_string());
                    }
                }
                FilterFocus::Filters => {
                    // Filters-nél Enter = szerkesztés (Delete = törlés)
                    app.filter_state.edit_selected_filter();
                }
                FilterFocus::SortField => {
                    // SortField-nél Enter = tovább a Field-re
                    app.filter_state.next_focus();
                }
            }
        }

        // Left arrow - cursor left in text fields, prev operator in Operator focus, prev sort field
        (KeyCode::Left, _) => match app.filter_state.focus {
            FilterFocus::Operator => app.filter_state.prev_operator(),
            FilterFocus::Field | FilterFocus::Value => app.filter_state.cursor_left(),
            FilterFocus::SortField => app.filter_state.prev_sort_field(),
            _ => {}
        },

        // Right arrow - cursor right in text fields, next operator in Operator focus, next sort field
        (KeyCode::Right, _) => match app.filter_state.focus {
            FilterFocus::Operator => app.filter_state.next_operator(),
            FilterFocus::Field | FilterFocus::Value => app.filter_state.cursor_right(),
            FilterFocus::SortField => app.filter_state.next_sort_field(),
            _ => {}
        },

        // Space - toggle sort direction in SortField focus
        (KeyCode::Char(' '), _) if app.filter_state.focus == FilterFocus::SortField => {
            app.filter_state.toggle_sort_direction();
        }

        // Home/End - cursor to start/end of text
        (KeyCode::Home, _) => app.filter_state.cursor_home(),
        (KeyCode::End, _) => app.filter_state.cursor_end(),

        // Ctrl+Up/Down - move filter up/down in list
        (KeyCode::Up, KeyModifiers::CONTROL) if app.filter_state.focus == FilterFocus::Filters => {
            app.filter_state.move_filter_up();
        }
        (KeyCode::Down, KeyModifiers::CONTROL)
            if app.filter_state.focus == FilterFocus::Filters =>
        {
            app.filter_state.move_filter_down();
        }

        // Up/Down - navigate suggestions (Field) or filters list (Filters)
        (KeyCode::Up, _) => {
            match app.filter_state.focus {
                FilterFocus::Field => {
                    // Navigate suggestions up
                    if app.filter_state.show_suggestions {
                        app.filter_state.suggestion_up();
                    } else {
                        // Show suggestions if we have them
                        app.filter_state.update_suggestions();
                    }
                }
                FilterFocus::Filters if app.filter_state.selected_filter > 0 => {
                    app.filter_state.selected_filter -= 1;
                }
                _ => {}
            }
        }
        (KeyCode::Down, _) => {
            match app.filter_state.focus {
                FilterFocus::Field => {
                    // Navigate suggestions down
                    if app.filter_state.show_suggestions {
                        app.filter_state.suggestion_down();
                    } else {
                        // Show suggestions if we have them
                        app.filter_state.update_suggestions();
                    }
                }
                FilterFocus::Filters
                    if app.filter_state.selected_filter + 1 < app.filter_state.filters.len() =>
                {
                    app.filter_state.selected_filter += 1;
                }
                _ => {}
            }
        }

        // Delete - remove selected filter
        (KeyCode::Delete, _) if app.filter_state.focus == FilterFocus::Filters => {
            app.filter_state.remove_selected_filter();
        }
        (KeyCode::Char('d'), KeyModifiers::CONTROL)
            if app.filter_state.focus == FilterFocus::Filters =>
        {
            app.filter_state.remove_selected_filter();
        }

        // Backspace - delete character
        (KeyCode::Backspace, _) => {
            app.filter_state.backspace();
        }

        // Character input - only for Field and Value focus
        (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
            match app.filter_state.focus {
                FilterFocus::Field | FilterFocus::Value => {
                    app.filter_state.insert_char(c);
                }
                _ => {}
            }
        }

        _ => {}
    }
}
