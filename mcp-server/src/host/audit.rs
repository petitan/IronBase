// Audit logging for MCP operations

use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Audit logger - logs all MCP operations for compliance and debugging
pub struct AuditLogger {
    writer: Arc<Mutex<BufWriter<File>>>,
    path: PathBuf,
}

impl AuditLogger {
    /// Create a new audit logger
    pub fn new(path: PathBuf) -> Result<Self, std::io::Error> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;

        Ok(Self {
            writer: Arc::new(Mutex::new(BufWriter::new(file))),
            path,
        })
    }

    /// Log an audit entry
    pub fn log(&self, entry: AuditEntry) -> Result<(), std::io::Error> {
        let json = serde_json::to_string(&entry)?;
        let mut writer = self.writer.lock().unwrap();
        writeln!(writer, "{}", json)?;
        writer.flush()?;
        Ok(())
    }

    /// Log a command execution
    pub fn log_command(
        &self,
        command: &str,
        api_key_name: &str,
        params: serde_json::Value,
        result: CommandResult,
    ) -> Result<String, std::io::Error> {
        let entry = AuditEntry {
            audit_id: Self::generate_audit_id(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            event_type: AuditEventType::Command,
            api_key_name: api_key_name.to_string(),
            command: Some(command.to_string()),
            document_id: params
                .get("document_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            block_label: params
                .get("block_label")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            details: Some(params),
            result,
        };

        let audit_id = entry.audit_id.clone();
        self.log(entry)?;
        Ok(audit_id)
    }

    /// Log an authentication event
    pub fn log_auth(
        &self,
        api_key_name: &str,
        success: bool,
        reason: Option<String>,
    ) -> Result<(), std::io::Error> {
        let details = reason.as_ref().map(|r| serde_json::json!({ "reason": r }));
        let error_message = reason.clone().unwrap_or_else(|| "Authentication failed".to_string());

        let entry = AuditEntry {
            audit_id: Self::generate_audit_id(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            event_type: if success {
                AuditEventType::AuthSuccess
            } else {
                AuditEventType::AuthFailure
            },
            api_key_name: api_key_name.to_string(),
            command: None,
            document_id: None,
            block_label: None,
            details,
            result: if success {
                CommandResult::Success
            } else {
                CommandResult::Error {
                    message: error_message,
                }
            },
        };

        self.log(entry)
    }

    /// Log a rate limit event
    pub fn log_rate_limit(
        &self,
        api_key_name: &str,
        command: &str,
    ) -> Result<(), std::io::Error> {
        let entry = AuditEntry {
            audit_id: Self::generate_audit_id(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            event_type: AuditEventType::RateLimitExceeded,
            api_key_name: api_key_name.to_string(),
            command: Some(command.to_string()),
            document_id: None,
            block_label: None,
            details: None,
            result: CommandResult::Error {
                message: "Rate limit exceeded".to_string(),
            },
        };

        self.log(entry)
    }

    /// Generate a unique audit ID
    fn generate_audit_id() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let random = rand::random::<u32>();
        format!("audit_{}_{:08x}", timestamp, random)
    }

    /// Get the audit log path
    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

/// Audit log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub audit_id: String,
    pub timestamp: String,
    pub event_type: AuditEventType,
    pub api_key_name: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_label: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,

    pub result: CommandResult,
}

/// Type of audit event
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AuditEventType {
    Command,
    AuthSuccess,
    AuthFailure,
    RateLimitExceeded,
}

/// Result of a command
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum CommandResult {
    Success,
    Error { message: String },
}

/// Audit query for retrieving log entries
pub struct AuditQuery {
    pub document_id: Option<String>,
    pub block_label: Option<String>,
    pub api_key_name: Option<String>,
    pub command: Option<String>,
    pub limit: Option<usize>,
}

impl AuditQuery {
    pub fn new() -> Self {
        Self {
            document_id: None,
            block_label: None,
            api_key_name: None,
            command: None,
            limit: Some(100),
        }
    }

    pub fn document(mut self, document_id: String) -> Self {
        self.document_id = Some(document_id);
        self
    }

    pub fn block(mut self, block_label: String) -> Self {
        self.block_label = Some(block_label);
        self
    }

    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }
}

impl Default for AuditQuery {
    fn default() -> Self {
        Self::new()
    }
}

/// Read audit log entries (for mcp_docjl_get_audit_log command)
pub fn read_audit_log(
    path: &PathBuf,
    query: AuditQuery,
) -> Result<Vec<AuditEntry>, std::io::Error> {
    use std::io::{BufRead, BufReader};

    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let mut entries = Vec::new();
    let mut count = 0;
    let limit = query.limit.unwrap_or(100);

    // Read from end of file (most recent first)
    let lines: Vec<String> = reader.lines().collect::<Result<Vec<_>, _>>()?;

    for line in lines.iter().rev() {
        if count >= limit {
            break;
        }

        if let Ok(entry) = serde_json::from_str::<AuditEntry>(line) {
            let matches = query.document_id.as_ref().map_or(true, |doc_id| {
                entry.document_id.as_ref() == Some(doc_id)
            }) && query.block_label.as_ref().map_or(true, |label| {
                entry.block_label.as_ref() == Some(label)
            }) && query.api_key_name.as_ref().map_or(true, |name| {
                &entry.api_key_name == name
            }) && query.command.as_ref().map_or(true, |cmd| {
                entry.command.as_ref() == Some(cmd)
            });

            if matches {
                entries.push(entry);
                count += 1;
            }
        }
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_audit_logger() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_path_buf();

        let logger = AuditLogger::new(path.clone()).unwrap();

        let entry = AuditEntry {
            audit_id: "test_123".to_string(),
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            event_type: AuditEventType::Command,
            api_key_name: "test_key".to_string(),
            command: Some("mcp_docjl_get_document".to_string()),
            document_id: Some("doc_1".to_string()),
            block_label: None,
            details: None,
            result: CommandResult::Success,
        };

        logger.log(entry.clone()).unwrap();

        // Read back
        let entries = read_audit_log(&path, AuditQuery::new()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].audit_id, "test_123");
    }

    #[test]
    fn test_audit_query_filter() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_path_buf();

        let logger = AuditLogger::new(path.clone()).unwrap();

        // Log multiple entries
        for i in 1..=5 {
            let entry = AuditEntry {
                audit_id: format!("test_{}", i),
                timestamp: "2024-01-01T00:00:00Z".to_string(),
                event_type: AuditEventType::Command,
                api_key_name: "test_key".to_string(),
                command: Some("mcp_docjl_get_document".to_string()),
                document_id: Some(format!("doc_{}", i)),
                block_label: None,
                details: None,
                result: CommandResult::Success,
            };
            logger.log(entry).unwrap();
        }

        // Query for doc_3
        let query = AuditQuery::new().document("doc_3".to_string());
        let entries = read_audit_log(&path, query).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].document_id, Some("doc_3".to_string()));
    }

    #[test]
    fn test_audit_limit() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_path_buf();

        let logger = AuditLogger::new(path.clone()).unwrap();

        // Log 10 entries
        for i in 1..=10 {
            let entry = AuditEntry {
                audit_id: format!("test_{}", i),
                timestamp: "2024-01-01T00:00:00Z".to_string(),
                event_type: AuditEventType::Command,
                api_key_name: "test_key".to_string(),
                command: Some("mcp_docjl_get_document".to_string()),
                document_id: None,
                block_label: None,
                details: None,
                result: CommandResult::Success,
            };
            logger.log(entry).unwrap();
        }

        // Query with limit 5
        let query = AuditQuery::new().limit(5);
        let entries = read_audit_log(&path, query).unwrap();
        assert_eq!(entries.len(), 5);

        // Should get most recent entries (10, 9, 8, 7, 6)
        assert_eq!(entries[0].audit_id, "test_10");
        assert_eq!(entries[4].audit_id, "test_6");
    }
}
