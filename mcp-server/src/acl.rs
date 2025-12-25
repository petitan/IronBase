//! Access Control List (ACL) for collection-level permissions
//!
//! Provides:
//! - InterfaceType detection (Localhost/Internal/External)
//! - Collection-level ACL rules stored in `_system.acl`
//! - Permission checking for all operations

use crate::adapter::{FindOptions, IronBaseAdapter};
use crate::error::{McpError, Result};
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

// ============================================================================
// InterfaceType - Client origin classification
// ============================================================================

/// Classification of client connection origin
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InterfaceType {
    /// Loopback address (127.0.0.1, ::1)
    Localhost,
    /// Private network (10.x.x.x, 172.16-31.x.x, 192.168.x.x)
    Internal,
    /// Public internet (everything else)
    External,
}

impl InterfaceType {
    /// Determine interface type from IP address
    pub fn from_ip(ip: IpAddr) -> Self {
        match ip {
            IpAddr::V4(v4) => {
                if v4.is_loopback() {
                    Self::Localhost
                } else if v4.is_private() {
                    Self::Internal
                } else {
                    Self::External
                }
            }
            IpAddr::V6(v6) => {
                if v6.is_loopback() {
                    Self::Localhost
                } else {
                    // IPv6 private ranges are complex, treat as external by default
                    // unless it's a mapped IPv4 address
                    if let Some(v4) = v6.to_ipv4_mapped() {
                        Self::from_ip(IpAddr::V4(v4))
                    } else {
                        Self::External
                    }
                }
            }
        }
    }

    /// Determine interface type from socket address
    pub fn from_socket_addr(addr: SocketAddr) -> Self {
        Self::from_ip(addr.ip())
    }
}

impl std::fmt::Display for InterfaceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Localhost => write!(f, "localhost"),
            Self::Internal => write!(f, "internal"),
            Self::External => write!(f, "external"),
        }
    }
}

impl std::str::FromStr for InterfaceType {
    type Err = McpError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "localhost" => Ok(Self::Localhost),
            "internal" => Ok(Self::Internal),
            "external" => Ok(Self::External),
            _ => Err(McpError::InvalidParams(format!(
                "Invalid interface type: {}",
                s
            ))),
        }
    }
}

// ============================================================================
// CallerContext - Information about the calling client
// ============================================================================

/// Context about the client making a request
#[derive(Debug, Clone)]
pub struct CallerContext {
    /// Interface type determined from client IP
    pub interface: InterfaceType,
    /// Client's remote address
    pub remote_addr: Option<SocketAddr>,
    /// API key if provided
    pub api_key: Option<String>,
}

impl CallerContext {
    /// Create context from socket address and optional API key
    pub fn new(remote_addr: Option<SocketAddr>, api_key: Option<String>) -> Self {
        let interface = remote_addr
            .map(InterfaceType::from_socket_addr)
            .unwrap_or(InterfaceType::Localhost); // Default to localhost if no address

        Self {
            interface,
            remote_addr,
            api_key,
        }
    }

    /// Create localhost context (for stdio/internal calls)
    pub fn localhost() -> Self {
        Self {
            interface: InterfaceType::Localhost,
            remote_addr: None,
            api_key: None,
        }
    }
}

// ============================================================================
// Permission types
// ============================================================================

/// Required permission for an operation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequiredPermission {
    Read,
    Write,
    Admin,
}

impl RequiredPermission {
    pub fn is_write(&self) -> bool {
        matches!(self, Self::Write | Self::Admin)
    }
}

/// Permissions granted to a principal
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Permissions {
    pub read: bool,
    pub write: bool,
    pub admin: bool,
}

impl Permissions {
    /// All permissions granted
    pub fn all() -> Self {
        Self {
            read: true,
            write: true,
            admin: true,
        }
    }

    /// Read-only permissions
    pub fn read_only() -> Self {
        Self {
            read: true,
            write: false,
            admin: false,
        }
    }

    /// No permissions (deny)
    pub fn none() -> Self {
        Self::default()
    }

    /// Check if permission is granted
    /// Hierarchy: admin > write > read
    pub fn allows(&self, required: RequiredPermission) -> bool {
        match required {
            RequiredPermission::Read => self.read || self.write || self.admin,
            RequiredPermission::Write => self.write || self.admin,
            RequiredPermission::Admin => self.admin,
        }
    }

    /// Parse from string like "read,write" or "read,write,admin"
    pub fn parse(s: &str) -> Self {
        let s = s.to_lowercase();
        if s == "deny" || s == "none" {
            return Self::none();
        }

        let mut perms = Self::default();
        for part in s.split(',') {
            match part.trim() {
                "read" => perms.read = true,
                "write" => {
                    perms.read = true; // write implies read
                    perms.write = true;
                }
                "admin" => {
                    perms.read = true;
                    perms.write = true;
                    perms.admin = true;
                }
                "all" => return Self::all(),
                _ => {}
            }
        }
        perms
    }
}

// ============================================================================
// Principal - Who is making the request
// ============================================================================

/// Principal (who) for ACL rules
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum Principal {
    /// Match by interface type
    #[serde(rename = "interface")]
    Interface(InterfaceType),
    /// Match by API key
    #[serde(rename = "apikey")]
    ApiKey(String),
    /// Match by exact IP address
    #[serde(rename = "ip")]
    Ip(IpAddr),
    /// Match by IP range (CIDR notation)
    #[serde(rename = "iprange")]
    IpRange(IpNet),
    /// Match anyone
    #[serde(rename = "anyone")]
    Anyone,
}

impl Principal {
    /// Check if this principal matches the caller context
    pub fn matches(&self, caller: &CallerContext) -> bool {
        match self {
            Self::Interface(iface) => caller.interface == *iface,
            Self::ApiKey(key) => caller.api_key.as_ref() == Some(key),
            Self::Ip(ip) => caller.remote_addr.map(|a| a.ip() == *ip).unwrap_or(false),
            Self::IpRange(net) => caller
                .remote_addr
                .map(|a| net.contains(&a.ip()))
                .unwrap_or(false),
            Self::Anyone => true,
        }
    }

    /// Parse from string like "interface:internal" or "apikey:abc123"
    pub fn parse(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(McpError::InvalidParams(format!(
                "Invalid principal format: {}. Expected 'type:value'",
                s
            )));
        }

        let (ptype, pvalue) = (parts[0], parts[1]);
        match ptype {
            "interface" => Ok(Self::Interface(pvalue.parse()?)),
            "apikey" => Ok(Self::ApiKey(pvalue.to_string())),
            "ip" => {
                let ip: IpAddr = pvalue
                    .parse()
                    .map_err(|e| McpError::InvalidParams(format!("Invalid IP address: {}", e)))?;
                Ok(Self::Ip(ip))
            }
            "iprange" => {
                let net: IpNet = pvalue
                    .parse()
                    .map_err(|e| McpError::InvalidParams(format!("Invalid IP range: {}", e)))?;
                Ok(Self::IpRange(net))
            }
            "anyone" => Ok(Self::Anyone),
            _ => Err(McpError::InvalidParams(format!(
                "Unknown principal type: {}",
                ptype
            ))),
        }
    }
}

// ============================================================================
// ACL Rules
// ============================================================================

/// A single ACL rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AclRule {
    pub principal: Principal,
    pub permissions: Permissions,
}

/// ACL for a collection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionAcl {
    /// Collection name or pattern ("*" for all, "_system.*" for system collections)
    pub collection: String,
    /// Rules in order of precedence (first match wins)
    pub rules: Vec<AclRule>,
}

impl CollectionAcl {
    /// Check if this ACL applies to the given collection name
    pub fn matches_collection(&self, collection: &str) -> bool {
        if self.collection == "*" {
            return true;
        }
        if self.collection.ends_with(".*") {
            let prefix = &self.collection[..self.collection.len() - 2];
            return collection.starts_with(prefix);
        }
        self.collection == collection
    }
}

// ============================================================================
// ACL Configuration
// ============================================================================

/// System collection for ACL storage
pub const SYSTEM_ACL_COLLECTION: &str = "_system.acl";

/// System collection for listener configuration
pub const SYSTEM_LISTENERS_COLLECTION: &str = "_system.listeners";

/// ACL configuration loaded from database
#[derive(Debug, Clone)]
pub struct AclConfig {
    rules: Vec<CollectionAcl>,
}

impl AclConfig {
    /// Built-in rules that cannot be overridden
    fn builtin_rules() -> Vec<CollectionAcl> {
        vec![
            // _system.scripts - allow read from internal/external for script execution
            // Write/admin still requires localhost (enforced by requires_localhost check)
            CollectionAcl {
                collection: "_system.scripts".to_string(),
                rules: vec![
                    AclRule {
                        principal: Principal::Interface(InterfaceType::Localhost),
                        permissions: Permissions::all(),
                    },
                    AclRule {
                        principal: Principal::Interface(InterfaceType::Internal),
                        permissions: Permissions::read_only(),
                    },
                    AclRule {
                        principal: Principal::Interface(InterfaceType::External),
                        permissions: Permissions::read_only(),
                    },
                ],
            },
            // Other _system.* collections are protected - only localhost
            CollectionAcl {
                collection: "_system.*".to_string(),
                rules: vec![AclRule {
                    principal: Principal::Interface(InterfaceType::Localhost),
                    permissions: Permissions::all(),
                }],
            },
        ]
    }

    /// Default rules for normal collections (when no ACL is defined)
    fn default_rules() -> Vec<CollectionAcl> {
        vec![CollectionAcl {
            collection: "*".to_string(),
            rules: vec![
                // Localhost gets full access
                AclRule {
                    principal: Principal::Interface(InterfaceType::Localhost),
                    permissions: Permissions::all(),
                },
                // Internal network gets read/write
                AclRule {
                    principal: Principal::Interface(InterfaceType::Internal),
                    permissions: Permissions {
                        read: true,
                        write: true,
                        admin: false,
                    },
                },
                // External gets read only
                AclRule {
                    principal: Principal::Interface(InterfaceType::External),
                    permissions: Permissions::read_only(),
                },
            ],
        }]
    }

    /// Create empty config (for testing or when DB not available)
    pub fn empty() -> Self {
        let mut rules = Self::builtin_rules();
        rules.extend(Self::default_rules());
        Self { rules }
    }

    /// Load ACL configuration from database
    pub fn load_from_db(adapter: &IronBaseAdapter) -> Result<Self> {
        // Start with builtin rules
        let mut rules = Self::builtin_rules();

        // Try to load from _system.acl collection
        match adapter.find(SYSTEM_ACL_COLLECTION, json!({}), FindOptions::default()) {
            Ok(result) => {
                for doc in result.documents {
                    if let Ok(acl) = serde_json::from_value::<CollectionAcl>(doc) {
                        rules.push(acl);
                    }
                }
            }
            Err(_) => {
                // Collection doesn't exist yet - use defaults
            }
        }

        // Add default rules at the end (lowest priority)
        rules.extend(Self::default_rules());

        Ok(Self { rules })
    }

    /// Check if operation is permitted
    pub fn check(
        &self,
        collection: &str,
        caller: &CallerContext,
        required: RequiredPermission,
    ) -> Result<()> {
        // Special case: _system.* collections can only be modified from localhost
        if collection.starts_with("_system.")
            && required.is_write()
            && caller.interface != InterfaceType::Localhost
        {
            return Err(McpError::Forbidden(format!(
                "'{}' can only be modified from localhost (current: {})",
                collection, caller.interface
            )));
        }

        // Find matching ACL rules
        for acl in &self.rules {
            if acl.matches_collection(collection) {
                // Find first matching principal
                for rule in &acl.rules {
                    if rule.principal.matches(caller) {
                        if rule.permissions.allows(required) {
                            return Ok(());
                        } else {
                            return Err(McpError::Forbidden(format!(
                                "'{}': {:?} permission required, not granted for {} client",
                                collection, required, caller.interface
                            )));
                        }
                    }
                }
            }
        }

        // No matching rule found - deny by default
        Err(McpError::Forbidden(format!(
            "'{}': no ACL rule for {} client",
            collection, caller.interface
        )))
    }

    /// Get the ACL for a specific collection (for listing)
    pub fn get_collection_acl(&self, collection: &str) -> Option<&CollectionAcl> {
        self.rules.iter().find(|acl| acl.collection == collection)
    }

    /// Get all ACL rules (for listing)
    pub fn list_all(&self) -> &[CollectionAcl] {
        &self.rules
    }
}

/// Thread-safe ACL manager
pub struct AclManager {
    config: std::sync::RwLock<AclConfig>,
    adapter: Arc<IronBaseAdapter>,
}

impl AclManager {
    /// Create new ACL manager
    pub fn new(adapter: Arc<IronBaseAdapter>) -> Self {
        let config = AclConfig::load_from_db(&adapter).unwrap_or_else(|_| AclConfig::empty());
        Self {
            config: std::sync::RwLock::new(config),
            adapter,
        }
    }

    /// Check permission
    pub fn check(
        &self,
        collection: &str,
        caller: &CallerContext,
        required: RequiredPermission,
    ) -> Result<()> {
        let config = self.config.read().unwrap();
        config.check(collection, caller, required)
    }

    /// Reload ACL from database
    pub fn reload(&self) -> Result<()> {
        let new_config = AclConfig::load_from_db(&self.adapter)?;
        let mut config = self.config.write().unwrap();
        *config = new_config;
        Ok(())
    }

    /// Set ACL for a collection
    pub fn set_acl(&self, collection: &str, rules: Vec<AclRule>) -> Result<()> {
        let acl = CollectionAcl {
            collection: collection.to_string(),
            rules,
        };

        let doc = serde_json::to_value(&acl)?;

        // Upsert into _system.acl
        let filter = json!({ "collection": collection });
        let existing = self
            .adapter
            .find_one(SYSTEM_ACL_COLLECTION, filter.clone())?;

        if existing.is_some() {
            self.adapter
                .update_one(SYSTEM_ACL_COLLECTION, filter, json!({ "$set": doc }))?;
        } else {
            self.adapter.insert_one(SYSTEM_ACL_COLLECTION, doc)?;
        }

        // Reload config
        self.reload()
    }

    /// Delete ACL for a collection (reverts to default)
    pub fn delete_acl(&self, collection: &str) -> Result<()> {
        let filter = json!({ "collection": collection });
        self.adapter.delete_one(SYSTEM_ACL_COLLECTION, filter)?;
        self.reload()
    }

    /// List all ACLs
    pub fn list_all(&self) -> Vec<CollectionAcl> {
        let config = self.config.read().unwrap();
        config.list_all().to_vec()
    }

    /// Get ACL for a collection
    pub fn get_acl(&self, collection: &str) -> Option<CollectionAcl> {
        let config = self.config.read().unwrap();
        config.get_collection_acl(collection).cloned()
    }
}

// ============================================================================
// Tool permission mapping
// ============================================================================

/// Get required permission for a tool
pub fn get_required_permission(tool_name: &str) -> RequiredPermission {
    match tool_name {
        // Read operations
        "find"
        | "find_one"
        | "count_documents"
        | "distinct"
        | "aggregate"
        | "explain"
        | "index_list"
        | "index_list_fulltext"
        | "schema_get"
        | "collection_list"
        | "db_stats"
        | "db_open"
        | "find_with_hint"
        | "transaction_status"
        | "fulltext_search"
        | "fuzzy_search" => RequiredPermission::Read,

        // Write operations
        "insert_one" | "insert_many" | "update_one" | "update_many" | "delete_one"
        | "delete_many" => RequiredPermission::Write,

        // Admin operations (structure changes)
        "collection_create"
        | "collection_drop"
        | "index_create"
        | "index_drop"
        | "index_create_fulltext"
        | "index_create_fuzzy"
        | "schema_set"
        | "db_compact"
        | "db_checkpoint" => RequiredPermission::Admin,

        // Transaction operations (write)
        "begin_transaction"
        | "commit_transaction"
        | "rollback_transaction"
        | "insert_one_tx"
        | "update_one_tx"
        | "delete_one_tx" => RequiredPermission::Write,

        // ACL operations
        "acl_list" | "acl_get" => RequiredPermission::Read,
        "acl_set" | "acl_delete" => RequiredPermission::Admin,

        // Listener operations
        "listener_list" | "listener_get" => RequiredPermission::Read,
        "listener_add" | "listener_delete" | "listener_enable" | "listener_disable" => {
            RequiredPermission::Admin
        }

        // Script operations - creation/modification requires Admin
        "script_save" | "script_delete" | "script_rollback" | "script_tags_add"
        | "script_tags_remove" => RequiredPermission::Admin,
        // Script execution and reading requires only Read permission
        "script_run" | "script_exec" | "script_list" | "script_get" | "script_history"
        | "script_stats" | "script_version_get" => RequiredPermission::Read,

        // Admin tools (localhost only, all require Admin permission)
        "admin_list_all_collections"
        | "admin_create_system_collection"
        | "admin_set_collection_flags"
        | "admin_drop_protected"
        | "admin_apikey_create"
        | "admin_apikey_list"
        | "admin_apikey_revoke"
        | "admin_apikey_delete" => RequiredPermission::Admin,

        // Default to read for unknown tools
        _ => RequiredPermission::Read,
    }
}

/// Extract collection name from tool arguments
pub fn get_collection_from_args(args: &Value) -> Option<String> {
    args.get("collection")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Get the system collection for admin tools that don't have a collection argument
/// Returns the _system.* collection that the tool operates on
/// System collection for scripts
const SYSTEM_SCRIPTS_COLLECTION: &str = "_system.scripts";
/// System collection for API keys
const SYSTEM_APIKEYS_COLLECTION: &str = "_system.api_keys";

pub fn get_system_collection_for_tool(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        "acl_list" | "acl_get" | "acl_set" | "acl_delete" => Some(SYSTEM_ACL_COLLECTION),
        "listener_list" | "listener_get" | "listener_add" | "listener_delete"
        | "listener_enable" | "listener_disable" => {
            Some(crate::listener::SYSTEM_LISTENERS_COLLECTION)
        }
        // Script operations use _system.scripts
        "script_save" | "script_get" | "script_list" | "script_delete" | "script_run"
        | "script_exec" | "script_history" | "script_rollback" | "script_tags_add"
        | "script_tags_remove" | "script_stats" | "script_version_get" => {
            Some(SYSTEM_SCRIPTS_COLLECTION)
        }
        // API key operations use _system.api_keys
        "admin_apikey_create"
        | "admin_apikey_list"
        | "admin_apikey_revoke"
        | "admin_apikey_delete" => Some(SYSTEM_APIKEYS_COLLECTION),
        // Other admin operations use _system.acl (system config)
        "admin_list_all_collections"
        | "admin_create_system_collection"
        | "admin_set_collection_flags"
        | "admin_drop_protected" => Some(SYSTEM_ACL_COLLECTION),
        _ => None,
    }
}

/// Check if a tool requires localhost access (system administration tools)
/// These tools modify _system.* collections and are restricted to localhost
pub fn requires_localhost(tool_name: &str) -> bool {
    matches!(
        tool_name,
        // ACL management
        "acl_set" | "acl_delete" | "acl_cleanup"
            // Listener management
            | "listener_add"
            | "listener_delete"
            | "listener_enable"
            | "listener_disable"
            // Script management (modify operations)
            | "script_save"
            | "script_delete"
            | "script_rollback"
            | "script_tags_add"
            | "script_tags_remove"
            // Admin tools (all require localhost)
            | "admin_list_all_collections"
            | "admin_create_system_collection"
            | "admin_set_collection_flags"
            | "admin_drop_protected"
            | "admin_apikey_create"
            | "admin_apikey_list"
            | "admin_apikey_revoke"
            | "admin_apikey_delete"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn test_interface_type_from_ip() {
        // Localhost
        assert_eq!(
            InterfaceType::from_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))),
            InterfaceType::Localhost
        );
        assert_eq!(
            InterfaceType::from_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)),
            InterfaceType::Localhost
        );

        // Internal networks
        assert_eq!(
            InterfaceType::from_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100))),
            InterfaceType::Internal
        );
        assert_eq!(
            InterfaceType::from_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))),
            InterfaceType::Internal
        );
        assert_eq!(
            InterfaceType::from_ip(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))),
            InterfaceType::Internal
        );

        // External
        assert_eq!(
            InterfaceType::from_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))),
            InterfaceType::External
        );
        assert_eq!(
            InterfaceType::from_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))),
            InterfaceType::External
        );
    }

    #[test]
    fn test_permissions_parse() {
        let perms = Permissions::parse("read");
        assert!(perms.read);
        assert!(!perms.write);
        assert!(!perms.admin);

        let perms = Permissions::parse("read,write");
        assert!(perms.read);
        assert!(perms.write);
        assert!(!perms.admin);

        let perms = Permissions::parse("all");
        assert!(perms.read);
        assert!(perms.write);
        assert!(perms.admin);

        let perms = Permissions::parse("deny");
        assert!(!perms.read);
        assert!(!perms.write);
        assert!(!perms.admin);
    }

    #[test]
    fn test_principal_matching() {
        let caller = CallerContext {
            interface: InterfaceType::Internal,
            remote_addr: Some("192.168.1.100:12345".parse().unwrap()),
            api_key: Some("test_key".to_string()),
        };

        assert!(Principal::Interface(InterfaceType::Internal).matches(&caller));
        assert!(!Principal::Interface(InterfaceType::External).matches(&caller));
        assert!(Principal::ApiKey("test_key".to_string()).matches(&caller));
        assert!(!Principal::ApiKey("wrong_key".to_string()).matches(&caller));
        assert!(Principal::Anyone.matches(&caller));
    }

    #[test]
    fn test_collection_acl_matching() {
        let acl = CollectionAcl {
            collection: "_system.*".to_string(),
            rules: vec![],
        };

        assert!(acl.matches_collection("_system.acl"));
        assert!(acl.matches_collection("_system.scripts"));
        assert!(!acl.matches_collection("users"));

        let acl_all = CollectionAcl {
            collection: "*".to_string(),
            rules: vec![],
        };
        assert!(acl_all.matches_collection("anything"));
    }
}
