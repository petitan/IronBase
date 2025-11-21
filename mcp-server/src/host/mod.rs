// MCP Host - JSON-RPC server and security layer

pub mod audit;
pub mod security;

pub use audit::{AuditEntry, AuditLogger, AuditQuery, CommandResult, read_audit_log};
pub use security::{ApiKey, AuthError, AuthManager, RateLimitConfig};
