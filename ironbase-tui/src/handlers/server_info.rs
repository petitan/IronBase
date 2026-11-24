//! Key handlers: server_info

use crate::app::{App, OpenModal};
use crate::i18n::t;
use crossterm::event::KeyCode;
use ironbase_mcp_types::params::NoParams;
use ironbase_mcp_types::tools::DbStats;

/// Server info modal key handler with scrolling support
pub(crate) fn handle_server_info_key(app: &mut App, key: KeyCode) {
    let total_lines = match app.open_modal() {
        Some(OpenModal::ServerInfo(state)) => state.total_lines,
        _ => 0,
    };
    let max_scroll = total_lines.saturating_sub(20);
    match key {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('I') => {
            app.server_info_scroll = 0;
            app.close_modal();
        }
        KeyCode::Down | KeyCode::Char('j') if app.server_info_scroll < max_scroll => {
            app.server_info_scroll += 1;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.server_info_scroll = app.server_info_scroll.saturating_sub(1);
        }
        KeyCode::PageDown => {
            app.server_info_scroll = (app.server_info_scroll + 10).min(max_scroll);
        }
        KeyCode::PageUp => {
            app.server_info_scroll = app.server_info_scroll.saturating_sub(10);
        }
        KeyCode::Home | KeyCode::Char('g') => {
            app.server_info_scroll = 0;
        }
        KeyCode::End | KeyCode::Char('G') => {
            app.server_info_scroll = max_scroll;
        }
        _ => {}
    }
}

/// Open server info modal and load data from MCP server
pub(crate) async fn handle_server_info_open(app: &mut App) {
    app.open_server_info();

    // Load data from MCP server
    if let Some(ref db) = app.db {
        // Fetch all three in parallel
        let (stats_result, tools_result, prompts_result) = tokio::join!(
            db.call::<DbStats>(&NoParams {}),
            db.tools_list(),
            db.prompts_list()
        );

        match (stats_result, tools_result, prompts_result) {
            (Ok(stats), Ok(tools), Ok(prompts)) => {
                app.update_server_info(stats, tools, prompts);
            }
            (Err(e), _, _) => {
                app.set_server_info_error(format!("db_stats: {}", t().error(&e)));
            }
            (_, Err(e), _) => {
                app.set_server_info_error(format!("tools/list: {}", t().error(&e)));
            }
            (_, _, Err(e)) => {
                app.set_server_info_error(format!("prompts/list: {}", t().error(&e)));
            }
        }
    } else {
        app.set_server_info_error(t().no_connection().to_string());
    }
}
