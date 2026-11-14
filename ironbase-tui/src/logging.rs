//! File logging (`tracing`). Nothing is ever written to the terminal: the
//! only writer is the log file.
//!
//! Rules for callers: never log an API key, an admin key or a header value;
//! tool arguments only at debug level and only through [`redact_arguments`];
//! every dynamic text (error text, server message, serialised arguments)
//! only through [`sanitize_for_log`].

use serde_json::Value;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, RwLock};
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Environment variable that overrides the log level (e.g. `debug`)
pub const LOG_LEVEL_ENV: &str = "TUTI_TUI_LOG";

/// A log file that is, or would grow, larger than this is rotated to
/// `<name>.1` - at startup and while running
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// Only this crate's events are written (lib and binary share the prefix):
/// HTTP-stack internals can carry header values, so they never reach the file
const LOG_TARGET: &str = "tuti_tui";

/// What is written in place of a redacted argument value
const REDACTED: &str = "[REDACTED]";

/// An argument field whose normalised name (lowercase, without `_` and `-`)
/// contains one of these is never logged; so is one that ends in `key`
const SECRET_NAME_PARTS: [&str; 7] = [
    "password",
    "secret",
    "token",
    "authorization",
    "apikey",
    "adminkey",
    "credential",
];

/// Shorter values are not registered as secrets: masking them would blank
/// out ordinary substrings of unrelated text
const MIN_SECRET_LEN: usize = 6;

/// Secret values known to this process (API key, admin key, a created key):
/// masked wherever they occur in logged text
static KNOWN_SECRETS: OnceLock<RwLock<Vec<String>>> = OnceLock::new();

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

/// The log file: appends, and keeps the size bound while running. The byte
/// count starts from the file length at open, so a write costs no extra
/// syscall until the limit is reached.
pub struct LogFile {
    path: PathBuf,
    file: File,
    written: u64,
    max_bytes: u64,
}

/// Open the log file for appending (owner-only on unix), rotating it first
/// if it has grown past the size limit
pub fn open_log_file(path: &Path) -> std::io::Result<LogFile> {
    open_log_file_bounded(path, MAX_LOG_BYTES)
}

fn open_log_file_bounded(path: &Path, max_bytes: u64) -> std::io::Result<LogFile> {
    // Create parent directory (a bare file name has an empty parent, which
    // `create_dir_all` accepts)
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // A failed rotation never disables logging: the current file is kept
    let _ = rotate_if_larger(path, max_bytes);
    let file = open_append(path)?;
    let written = file.metadata()?.len();
    Ok(LogFile {
        path: path.to_path_buf(),
        file,
        written,
        max_bytes,
    })
}

/// Size bound, shared by startup and the running writer: a file larger than
/// `threshold` is moved to `<name>.1`. One previous generation is kept, an
/// older one is replaced. The size is read here, right before the rename:
/// another process may have rotated the file already.
fn rotate_if_larger(path: &Path, threshold: u64) -> std::io::Result<()> {
    if !std::fs::metadata(path).is_ok_and(|m| m.len() > threshold) {
        return Ok(());
    }

    // Narrowed before the rename: the previous generation must not keep a
    // wider mode than the live file
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }

    let mut rotated = path.as_os_str().to_owned();
    rotated.push(".1");
    std::fs::rename(path, PathBuf::from(rotated))
}

fn open_append(path: &Path) -> std::io::Result<File> {
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

impl LogFile {
    /// Start a new file before a write that would pass the limit. On any
    /// failure the current file stays in use and the next attempt comes
    /// after another `max_bytes`: logging is never disabled by a rotation.
    fn rotate(&mut self, incoming: u64) {
        let reopened = rotate_if_larger(&self.path, self.max_bytes.saturating_sub(incoming))
            .and_then(|()| open_append(&self.path))
            .and_then(|file| file.metadata().map(|m| (file, m.len())));
        match reopened {
            Ok((file, len)) => {
                self.file = file;
                self.written = len;
            }
            Err(_) => self.written = 0,
        }
    }
}

impl Write for LogFile {
    /// One call is one log line: it is written whole, never split between
    /// two generations
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let incoming = buf.len() as u64;
        if self.written > 0 && self.written.saturating_add(incoming) > self.max_bytes {
            self.rotate(incoming);
        }
        self.file.write_all(buf)?;
        self.written = self.written.saturating_add(incoming);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

/// Subscriber that writes this crate's events at `level` and above to `file`
pub fn subscriber(file: LogFile, level: LevelFilter) -> impl tracing::Subscriber + Send + Sync {
    tracing_subscriber::registry()
        .with(Targets::new().with_target(LOG_TARGET, level))
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(Mutex::new(file))
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

/// Whether an argument field with this name holds a secret. Matched on the
/// normalised name, so `access_token`, `client-secret`, `apiKey` and
/// `adminKey` are all covered; any name ending in `key` is treated as one.
fn is_secret_field(name: &str) -> bool {
    let name: String = name
        .chars()
        .filter(|c| *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect();
    name.ends_with("key") || SECRET_NAME_PARTS.iter().any(|part| name.contains(part))
}

/// Copy of tool arguments that is safe to log: the value of every secret
/// field (`admin_key`, ...) is replaced, at any nesting depth
pub fn redact_arguments(arguments: &Value) -> Value {
    match arguments {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(name, value)| {
                    let value = if is_secret_field(name) {
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

/// Remember a secret value (API key, admin key): from now on it is masked in
/// every text that goes through [`sanitize_for_log`]. Values shorter than
/// [`MIN_SECRET_LEN`] are ignored.
pub fn register_secret(secret: &str) {
    if secret.len() < MIN_SECRET_LEN {
        return;
    }
    let mut secrets = KNOWN_SECRETS
        .get_or_init(Default::default)
        .write()
        .unwrap_or_else(|e| e.into_inner());
    if !secrets.iter().any(|s| s == secret) {
        secrets.push(secret.to_string());
        // Longest first: a secret that contains another one is masked whole
        secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    }
}

fn mask_known_secrets(text: &str) -> String {
    let Some(secrets) = KNOWN_SECRETS.get() else {
        return text.to_string();
    };
    let secrets = secrets.read().unwrap_or_else(|e| e.into_inner());
    let mut out = text.to_string();
    for secret in secrets.iter() {
        if out.contains(secret.as_str()) {
            out = out.replace(secret.as_str(), REDACTED);
        }
    }
    out
}

/// Escape control characters (newline, carriage return, tab, ESC, the other
/// C0/C1 codes) and the Unicode line/paragraph separators: a text from the
/// server cannot start a new log line or drive the terminal that shows the log
fn escape_control_chars(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() || c == '\u{2028}' || c == '\u{2029}' {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    out
}

/// The one way a dynamic text (error text, server message, serialised
/// arguments) reaches the log: known secret values are masked, URLs lose
/// their userinfo and query, control characters are escaped.
///
/// Not recognised: a secret this process never held (e.g. in a broken config
/// file) and a secret in a URL *path*.
pub fn sanitize_for_log(text: &str) -> String {
    escape_control_chars(&redact_url_credentials(&mask_known_secrets(text)))
}

/// Mask what can carry a credential in URLs inside free text: the userinfo
/// (`scheme://user:pass@host`) and the whole query and fragment
/// (`?token=...`). Scheme, host and path are kept.
///
/// Error messages from the HTTP client quote the full server URL. The query
/// is masked up to the next whitespace: nothing else ends a URL for sure in
/// free text, so a closing bracket or quote after it is masked with it.
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

        // Path, then query/fragment
        let path_end = rest
            .find(|c: char| c == '?' || c == '#' || c.is_whitespace())
            .unwrap_or(rest.len());
        out.push_str(&rest[..path_end]);
        rest = &rest[path_end..];
        if let Some(mark) = rest.chars().next().filter(|c| *c == '?' || *c == '#') {
            out.push(mark);
            out.push_str("***");
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            rest = &rest[end..];
        }
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

    /// A bare relative name (`--log-file tuti.log`) has an empty parent;
    /// creating it is a no-op, not an error
    #[test]
    fn test_bare_file_name_needs_no_directory() {
        let parent = Path::new("tuti.log").parent().unwrap();
        assert_eq!(parent, Path::new(""));
        assert!(std::fs::create_dir_all(parent).is_ok());
    }

    /// The previous generation is owner-only even if the file it was made
    /// from had a wider mode
    #[cfg(unix)]
    #[test]
    fn test_rotated_log_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_log_path("rotate-mode");
        let rotated = path.with_file_name("tuti-tui.log.1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "123456789").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        open_log_file_bounded(&path, 8).unwrap();
        let mode = std::fs::metadata(&rotated).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        cleanup(&path);
    }

    /// The size bound holds while running, not only at startup
    #[test]
    fn test_log_file_is_rotated_while_running() {
        let path = temp_log_path("rotate-live");
        let rotated = path.with_file_name("tuti-tui.log.1");
        let mut log = open_log_file_bounded(&path, 32).unwrap();
        for i in 0..7 {
            // 10 bytes a line: three fit, the fourth starts a new file
            log.write_all(format!("line-{:04}\n", i).as_bytes())
                .unwrap();
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "line-0006\n");
        assert_eq!(
            std::fs::read_to_string(&rotated).unwrap(),
            "line-0003\nline-0004\nline-0005\n"
        );
        // A line longer than the limit is still written, whole
        let long = "x".repeat(40) + "\n";
        log.write_all(long.as_bytes()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), long);
        cleanup(&path);
    }

    /// A rotation that cannot be done (here: `.1` is a non-empty directory)
    /// does not disable logging - at startup or while running
    #[test]
    fn test_failed_rotation_keeps_logging() {
        let path = temp_log_path("rotate-fail");
        let rotated = path.with_file_name("tuti-tui.log.1");
        std::fs::create_dir_all(rotated.join("blocker")).unwrap();
        std::fs::write(&path, "123456789\n").unwrap();

        let mut log = open_log_file_bounded(&path, 8).unwrap();
        log.write_all(b"after\n").unwrap();
        log.write_all(b"again\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "123456789\nafter\nagain\n"
        );
        cleanup(&path);
    }

    /// Another process rotated the file first: the size is re-read before
    /// the rename, so its new file is not moved over the previous generation
    #[test]
    fn test_rotation_rechecks_the_size() {
        let path = temp_log_path("rotate-race");
        let rotated = path.with_file_name("tuti-tui.log.1");
        let mut log = open_log_file_bounded(&path, 32).unwrap();
        log.write_all(b"first-process-line-1\n").unwrap();

        // The other process: rotates and starts a new file
        std::fs::rename(&path, &rotated).unwrap();
        std::fs::write(&path, "other\n").unwrap();

        log.write_all(b"first-process-line-2\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&rotated).unwrap(),
            "first-process-line-1\n"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "other\nfirst-process-line-2\n"
        );
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

    /// A credential in a URL query string (or fragment) must not reach the
    /// log; scheme, host and path stay
    #[test]
    fn test_url_query_is_masked() {
        assert_eq!(
            redact_url_credentials("GET https://db.example/mcp?token=sk-1&x=2 failed"),
            "GET https://db.example/mcp?*** failed"
        );
        assert_eq!(
            redact_url_credentials(
                "http://h:8080?api_key=sk-1 and https://u:p@h2/p#access_token=t"
            ),
            "http://h:8080?*** and https://***@h2/p#***"
        );
        // The HTTP client's wording: the bracket after the query goes with it
        assert_eq!(
            redact_url_credentials(
                "error sending request for url (https://h/mcp?key=sk-1): timed out"
            ),
            "error sending request for url (https://h/mcp?*** timed out"
        );
        // A '?' outside a URL is left alone
        assert_eq!(redact_url_credentials("why? http://h/p"), "why? http://h/p");
    }

    /// A value registered as a secret is masked wherever it occurs; short
    /// values are not registered
    #[test]
    fn test_registered_secret_is_masked() {
        register_secret("sk-registered-0f3a");
        register_secret("sk-registered-0f3a-longer");
        register_secret("ab12");
        assert_eq!(
            sanitize_for_log("invalid key 'sk-registered-0f3a-longer' (sk-registered-0f3a), ab12"),
            "invalid key '[REDACTED]' ([REDACTED]), ab12"
        );
    }

    /// A server message cannot forge a log line or send terminal escapes
    #[test]
    fn test_control_characters_are_escaped() {
        assert_eq!(
            sanitize_for_log(
                "boom\n2026-01-01T00:00:00Z  INFO forged\r\tx\u{1b}[31m\u{85}\u{2028}"
            ),
            "boom\\n2026-01-01T00:00:00Z  INFO forged\\r\\tx\\u{1b}[31m\\u{85}\\u{2028}"
        );
        assert_eq!(sanitize_for_log("árvíztűrő \"x\""), "árvíztűrő \"x\"");
    }

    /// End to end: a tool error that echoes a registered key, carries a URL
    /// query and a newline is one harmless line in the file; so is the
    /// status-bar error
    #[tokio::test]
    async fn test_server_error_text_is_sanitized_in_the_log() {
        let path = temp_log_path("sanitize");
        let file = open_log_file(&path).unwrap();
        let _guard = tracing::subscriber::set_default(subscriber(file, LevelFilter::DEBUG));
        register_secret("sk-echoed-7c1d");

        let server = FakeServer::new(vec![]);
        let mut app = app_with_server(&server).await;
        let message = "bad key sk-echoed-7c1d at http://h/mcp?token=tok-9911\nFORGED line";
        server.fail("collection_list", message);
        let error = app.refresh_collections_async().await.unwrap_err();
        app.set_error(format!("Nem sikerult: {}", error));
        // The user still sees the original text
        assert!(app
            .error_message
            .as_ref()
            .unwrap()
            .contains("sk-echoed-7c1d"));
        // Debug argument line: a registered value under an unsuspicious name
        server.respond("admin_apikey_create", serde_json::json!({}));
        let db = app.db.as_ref().unwrap();
        db.create_api_key("adm-x", "sk-echoed-7c1d").await.unwrap();

        let log = std::fs::read_to_string(&path).unwrap();
        assert!(log.contains("admin_apikey_create"), "{log}");
        assert!(log.contains("FORGED"), "{log}");
        assert!(!log.contains("sk-echoed-7c1d"), "{log}");
        assert!(!log.contains("tok-9911"), "{log}");
        assert!(!log.lines().any(|l| l.starts_with("FORGED")), "{log}");
        cleanup(&path);
    }

    #[test]
    fn test_secret_field_names() {
        for name in [
            "admin_key",
            "api_key",
            "key",
            "password",
            "secret",
            "token",
            "Authorization",
            "access_token",
            "client_secret",
            "apiKey",
            "adminKey",
            "x-api-key",
            "db_password",
            "credentials",
            "refresh-token",
        ] {
            assert!(is_secret_field(name), "{name}");
        }
        // What this client sends as ordinary arguments stays readable
        for name in [
            "collection",
            "query",
            "index_name",
            "field",
            "fields",
            "filter",
            "document",
            "name",
            "id",
            "job_id",
            "keys",
            "limit",
            "code",
            "path",
            "provider",
            "text_field",
        ] {
            assert!(!is_secret_field(name), "{name}");
        }
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
        // No userinfo, no query: unchanged, including an '@' later in the path or text
        let plain = "GET http://127.0.0.1:8080/mcp/a@b.hu failed, see https://example.com/x";
        assert_eq!(redact_url_credentials(plain), plain);
        assert_eq!(redact_url_credentials("no url here"), "no url here");
        assert_eq!(redact_url_credentials("é://ő:ű@hőst/á"), "é://***@hőst/á");
    }
}
