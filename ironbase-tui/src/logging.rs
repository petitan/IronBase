//! File logging (`tracing`). Nothing is ever written to the terminal: the
//! only writer is the log file.
//!
//! Rules for callers: never log an API key, an admin key or a header value;
//! tool arguments only at debug level and only through [`redact_arguments`].

use serde_json::Value;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Environment variable that overrides the log level (e.g. `debug`)
pub const LOG_LEVEL_ENV: &str = "TUTI_TUI_LOG";

/// A log file larger than this at startup is rotated to `<name>.1`
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// Only this crate's events are written (lib and binary share the prefix):
/// HTTP-stack internals can carry header values, so they never reach the file
const LOG_TARGET: &str = "tuti_tui";

/// What is written in place of a redacted argument value
const REDACTED: &str = "[REDACTED]";

/// Argument fields whose value is never logged (compared lowercase)
const SECRET_FIELDS: [&str; 7] = [
    "admin_key",
    "api_key",
    "key",
    "password",
    "secret",
    "token",
    "authorization",
];

/// Default log file: state dir (Linux) or local data dir (macOS, Windows),
/// in the same `ironbase-tui` directory name the config uses
pub fn default_log_path() -> Option<PathBuf> {
    log_path_in(dirs::state_dir().or_else(dirs::data_local_dir))
}

fn log_path_in(base: Option<PathBuf>) -> Option<PathBuf> {
    base.map(|p| p.join("ironbase-tui").join("tuti-tui.log"))
}

/// Log level from the `TUTI_TUI_LOG` value: `info` when unset, an error for
/// an unknown level name
pub fn parse_level(value: Option<&str>) -> Result<LevelFilter, String> {
    match value.map(str::trim) {
        None | Some("") => Ok(LevelFilter::INFO),
        Some(s) => s.parse::<LevelFilter>().map_err(|_| {
            format!(
                "Invalid log level: '{}'. Use off/error/warn/info/debug/trace",
                s
            )
        }),
    }
}

/// Open the log file for appending (owner-only on unix), rotating it first
/// if it has grown past the size limit
pub fn open_log_file(path: &Path) -> std::io::Result<File> {
    open_log_file_bounded(path, MAX_LOG_BYTES)
}

fn open_log_file_bounded(path: &Path, max_bytes: u64) -> std::io::Result<File> {
    // Create parent directory
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Size bound: one previous generation is kept, an older one is replaced
    if std::fs::metadata(path).is_ok_and(|m| m.len() > max_bytes) {
        let mut rotated = path.as_os_str().to_owned();
        rotated.push(".1");
        std::fs::rename(path, PathBuf::from(rotated))?;
    }

    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);

    // Owner-only from the start, and an existing file with wider
    // permissions is narrowed too (same as the config file)
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let file = options.mode(0o600).open(path)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        options.open(path)
    }
}

/// Subscriber that writes this crate's events at `level` and above to `file`
pub fn subscriber(file: File, level: LevelFilter) -> impl tracing::Subscriber + Send + Sync {
    tracing_subscriber::registry()
        .with(Targets::new().with_target(LOG_TARGET, level))
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(Arc::new(file))
                .with_ansi(false),
        )
}

/// Install the process-wide file logger. On error (the file cannot be
/// opened, or a logger is already installed) nothing is logged; the caller
/// decides whether to tell the user.
pub fn init(path: &Path, level: LevelFilter) -> anyhow::Result<()> {
    let file = open_log_file(path).map_err(|e| anyhow::anyhow!("{}: {}", path.display(), e))?;
    subscriber(file, level)
        .try_init()
        .map_err(|e| anyhow::anyhow!("{}", e))
}

/// Copy of tool arguments that is safe to log: the value of every secret
/// field (`admin_key`, ...) is replaced, at any nesting depth
pub fn redact_arguments(arguments: &Value) -> Value {
    match arguments {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(name, value)| {
                    let value = if SECRET_FIELDS.contains(&name.to_lowercase().as_str()) {
                        Value::String(REDACTED.to_string())
                    } else {
                        redact_arguments(value)
                    };
                    (name.clone(), value)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact_arguments).collect()),
        other => other.clone(),
    }
}

/// Mask credentials embedded in URLs (`scheme://user:pass@host`) inside free text.
///
/// Error messages from the HTTP client quote the full server URL; a URL that
/// carries userinfo must not reach the log file.
pub fn redact_url_credentials(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find("://") {
        let (head, tail) = rest.split_at(pos + 3);
        out.push_str(head);
        let authority_end = tail
            .find(|c: char| c == '/' || c == '?' || c == '#' || c.is_whitespace())
            .unwrap_or(tail.len());
        let authority = &tail[..authority_end];
        match authority.rfind('@') {
            Some(at) => {
                out.push_str("***");
                out.push_str(&authority[at..]);
            }
            None => out.push_str(authority),
        }
        rest = &tail[authority_end..];
    }
    out.push_str(rest);
    out
}

/// The host of a URL - the only part of the server URL that is logged (a
/// URL can carry credentials or a key in its userinfo, path or query)
pub fn url_host(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
        .unwrap_or_else(|| "?".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{app_with_server, FakeServer};

    fn temp_log_path(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("tuti-tui-log-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("nested").join("tuti-tui.log")
    }

    fn cleanup(path: &Path) {
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn test_log_path_keeps_the_config_directory_name() {
        assert_eq!(log_path_in(None), None);
        let path = log_path_in(Some(PathBuf::from("/state"))).unwrap();
        assert_eq!(path, PathBuf::from("/state/ironbase-tui/tuti-tui.log"));
    }

    #[test]
    fn test_parse_level() {
        assert_eq!(parse_level(None), Ok(LevelFilter::INFO));
        assert_eq!(parse_level(Some("")), Ok(LevelFilter::INFO));
        assert_eq!(parse_level(Some("debug")), Ok(LevelFilter::DEBUG));
        assert_eq!(parse_level(Some(" WARN ")), Ok(LevelFilter::WARN));
        assert_eq!(parse_level(Some("off")), Ok(LevelFilter::OFF));
        assert!(parse_level(Some("chatty")).unwrap_err().contains("chatty"));
    }

    /// The parent directory is created and the file is appended to, not
    /// truncated, by a second open
    #[test]
    fn test_open_log_file_creates_and_appends() {
        use std::io::Write;
        let path = temp_log_path("append");
        open_log_file(&path).unwrap().write_all(b"one\n").unwrap();
        open_log_file(&path).unwrap().write_all(b"two\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");
        cleanup(&path);
    }

    #[cfg(unix)]
    #[test]
    fn test_log_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_log_path("mode");

        // New file
        open_log_file(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        // Existing world-readable file is narrowed
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        open_log_file(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        cleanup(&path);
    }

    #[test]
    fn test_oversized_log_file_is_rotated() {
        let path = temp_log_path("rotate");
        let rotated = path.with_file_name("tuti-tui.log.1");

        // At the limit: kept
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "12345678").unwrap();
        open_log_file_bounded(&path, 8).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 8);
        assert!(!rotated.exists());

        // Over the limit: moved aside, a new file is started
        std::fs::write(&path, "123456789").unwrap();
        open_log_file_bounded(&path, 8).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
        assert_eq!(std::fs::read_to_string(&rotated).unwrap(), "123456789");

        // The previous generation is replaced, not accumulated
        std::fs::write(&path, "abcdefghij").unwrap();
        open_log_file_bounded(&path, 8).unwrap();
        assert_eq!(std::fs::read_to_string(&rotated).unwrap(), "abcdefghij");
        cleanup(&path);
    }

    #[test]
    fn test_redact_arguments() {
        let args = serde_json::json!({
            "admin_key": "adm-secret",
            "name": "ci",
            "nested": {"Authorization": "Bearer sk-secret", "list": [{"token": "t-secret"}]}
        });
        let redacted = redact_arguments(&args);
        assert_eq!(redacted["admin_key"], REDACTED);
        assert_eq!(redacted["name"], "ci");
        assert_eq!(redacted["nested"]["Authorization"], REDACTED);
        assert_eq!(redacted["nested"]["list"][0]["token"], REDACTED);
        assert!(!redacted.to_string().contains("secret"));
    }

    #[test]
    fn test_url_host_drops_credentials_path_and_query() {
        assert_eq!(
            url_host("https://user:pw@db.example:9000/mcp?key=sk-1"),
            "db.example"
        );
        assert_eq!(url_host("http://127.0.0.1:8080"), "127.0.0.1");
        assert_eq!(url_host("not a url"), "?");
    }

    /// Tool-call outcomes reach the log file; at info level without the
    /// arguments
    #[tokio::test]
    async fn test_tool_call_outcomes_are_logged() {
        let path = temp_log_path("calls");
        let file = open_log_file(&path).unwrap();
        let _guard = tracing::subscriber::set_default(subscriber(file, LevelFilter::INFO));

        let server = FakeServer::new(vec![("users", vec![serde_json::json!({"name": "Alice"})])]);
        let mut app = app_with_server(&server).await;
        server.fail("collection_list", "catalog unavailable");
        assert!(app.refresh_collections_async().await.is_err());
        app.set_error("Nem sikerult betolteni");

        let log = std::fs::read_to_string(&path).unwrap();
        let line = |needle: &str| {
            log.lines()
                .find(|l| l.contains(needle))
                .unwrap_or_else(|| panic!("no '{}' line in:\n{}", needle, log))
                .to_string()
        };
        let ok = line("tool=\"find\"");
        assert!(ok.contains("INFO") && ok.contains("outcome=\"ok\""), "{ok}");
        assert!(ok.contains("duration_ms="), "{ok}");
        let failed = line("catalog unavailable");
        assert!(failed.contains("WARN"), "{failed}");
        assert!(failed.contains("tool=\"collection_list\""), "{failed}");
        assert!(failed.contains("outcome=\"error\""), "{failed}");
        assert!(line("Nem sikerult betolteni").contains("WARN"));
        // No arguments (collection name, documents) at info level
        assert!(!log.contains("users") && !log.contains("Alice"), "{log}");
        cleanup(&path);
    }

    /// Even at debug level, where arguments are logged, the admin key's
    /// value never reaches the file
    #[tokio::test]
    async fn test_admin_key_is_not_logged_at_debug_level() {
        let path = temp_log_path("redact");
        let file = open_log_file(&path).unwrap();
        let _guard = tracing::subscriber::set_default(subscriber(file, LevelFilter::DEBUG));

        let server = FakeServer::new(vec![]);
        let app = app_with_server(&server).await;
        server.respond(
            "admin_apikey_create",
            serde_json::json!({"key": "sk-new-key"}),
        );
        let db = app.db.as_ref().unwrap();
        db.create_api_key("adm-top-secret", "ci-runner")
            .await
            .unwrap();
        server.fail("admin_apikey_list", "invalid admin key");
        assert!(db.list_api_keys("adm-top-secret").await.is_err());

        // The key did reach the server...
        assert_eq!(
            server.calls_to("admin_apikey_create")[0]["admin_key"],
            "adm-top-secret"
        );
        // ...the arguments were logged, the key (and the created key in the
        // result) was not
        let log = std::fs::read_to_string(&path).unwrap();
        assert!(log.contains("admin_apikey_create"), "{log}");
        assert!(log.contains("ci-runner"), "{log}");
        assert!(log.contains(REDACTED), "{log}");
        assert!(log.contains("invalid admin key"), "{log}");
        assert!(!log.contains("adm-top-secret"), "{log}");
        assert!(!log.contains("sk-new-key"), "{log}");
        cleanup(&path);
    }

    #[test]
    fn test_redact_url_credentials() {
        assert_eq!(
            redact_url_credentials("error sending request for url (https://bob:s3cret@db.example.com:8080/mcp): timed out"),
            "error sending request for url (https://***@db.example.com:8080/mcp): timed out"
        );
        // No userinfo: unchanged, including an '@' later in the path or text
        let plain = "GET http://127.0.0.1:8080/mcp?mail=a@b.hu failed, see https://example.com/x";
        assert_eq!(redact_url_credentials(plain), plain);
        assert_eq!(redact_url_credentials("no url here"), "no url here");
        assert_eq!(redact_url_credentials("é://ő:ű@hőst/á"), "é://***@hőst/á");
    }
}
