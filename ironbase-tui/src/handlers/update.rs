//! Key handlers: update

use crate::app::App;
use crossterm::event::KeyCode;

/// Handle update modal keys
pub(crate) fn handle_update_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('U') => {
            app.close_modal();
        }
        _ => {}
    }
}

/// Open update check modal and fetch latest version from GitHub
pub(crate) async fn handle_update_open(app: &mut App) {
    // Get current version from MCP server
    let current_version = if let Some(ref db) = app.db {
        match db.get_server_version().await {
            Ok(version) => version,
            Err(_) => "unknown".to_string(),
        }
    } else {
        "unknown".to_string()
    };

    app.open_update(current_version);

    // Fetch latest version from GitHub API
    match fetch_github_latest_release().await {
        Ok((latest_version, download_url, release_notes)) => {
            app.update_update_state(latest_version, download_url, release_notes);
        }
        Err(e) => {
            app.set_update_error(format!("GitHub API hiba: {}", e));
        }
    }
}

/// Fetch latest release info from GitHub
pub(crate) async fn fetch_github_latest_release() -> Result<(String, String, Option<String>), String>
{
    let url = "https://api.github.com/repos/petitan/IronBase/releases/latest";

    let client = reqwest::Client::builder()
        .user_agent("IronBase-TUI")
        .build()
        .map_err(|e| e.to_string())?;

    let response = client.get(url).send().await.map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }

    let json: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;

    let tag_name = json
        .get("tag_name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let html_url = json
        .get("html_url")
        .and_then(|v| v.as_str())
        .unwrap_or("https://github.com/petitan/IronBase/releases")
        .to_string();

    let body = json
        .get("body")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok((tag_name, html_url, body))
}
