//! Access Control List (ACL) for collection-level permissions
//!
//! Provides:
//! - InterfaceType detection (Localhost/Internal/External)
//! - Collection-level ACL rules stored in `_system.acl`
//! - Permission checking for all operations

use crate::adapter::{FindOptions, IronBaseAdapter};
use crate::api_keys::constant_time_compare;
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
            _ => Err(McpError::invalid_params(format!(
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
            // SECURITY FIX: Use constant-time comparison to prevent timing attacks
            Self::ApiKey(key) => caller
                .api_key
                .as_ref()
                .map(|k| constant_time_compare(k.as_bytes(), key.as_bytes()))
                .unwrap_or(false),
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
            return Err(McpError::invalid_params(format!(
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
                    .map_err(|e| McpError::invalid_params(format!("Invalid IP address: {}", e)))?;
                Ok(Self::Ip(ip))
            }
            "iprange" => {
                let net: IpNet = pvalue
                    .parse()
                    .map_err(|e| McpError::invalid_params(format!("Invalid IP range: {}", e)))?;
                Ok(Self::IpRange(net))
            }
            "anyone" => Ok(Self::Anyone),
            _ => Err(McpError::invalid_params(format!(
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
///
/// Uses indexed lookup for O(1) exact match + O(w) wildcard check
/// instead of O(n) linear scan on every permission check.
#[derive(Debug, Clone)]
pub struct AclConfig {
    rules: Vec<CollectionAcl>,
    /// Index for exact collection name matches: "users" -> rules index
    /// O(1) lookup instead of O(n) linear scan
    exact_match_index: std::collections::HashMap<String, usize>,
    /// Indices of wildcard rules (*, prefix.*) in priority order
    /// These are checked only if exact match fails
    wildcard_rules: Vec<usize>,
}

impl AclConfig {
    /// Rebuild indexes after rules change
    /// Must be called after any modification to self.rules
    fn rebuild_indexes(&mut self) {
        self.exact_match_index.clear();
        self.wildcard_rules.clear();

        for (i, rule) in self.rules.iter().enumerate() {
            if rule.collection.contains('*') {
                // Wildcard rule: *, prefix.*, etc.
                self.wildcard_rules.push(i);
            } else {
                // Exact match rule - first one wins (maintains priority)
                self.exact_match_index
                    .entry(rule.collection.clone())
                    .or_insert(i);
            }
        }
    }

    /// Create new AclConfig with rules and build indexes
    fn new_with_rules(rules: Vec<CollectionAcl>) -> Self {
        let mut config = Self {
            rules,
            exact_match_index: std::collections::HashMap::new(),
            wildcard_rules: Vec::new(),
        };
        config.rebuild_indexes();
        config
    }

    /// Built-in rules that cannot be overridden
    fn builtin_rules() -> Vec<CollectionAcl> {
        vec![
            // SECURITY FIX: _system.scripts is now localhost-only
            // Script execution (script_run) is still allowed from internal/external,
            // but direct read access to script code is restricted to localhost.
            // This prevents leaking sensitive business logic or hardcoded credentials.
            CollectionAcl {
                collection: "_system.scripts".to_string(),
                rules: vec![AclRule {
                    principal: Principal::Interface(InterfaceType::Localhost),
                    permissions: Permissions::all(),
                }],
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
        Self::new_with_rules(rules)
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

        Ok(Self::new_with_rules(rules))
    }

    /// Check if operation is permitted
    ///
    /// Performance: O(1) for exact collection match, O(w) for wildcard fallback
    /// where w = number of wildcard rules (typically 2-3).
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
            return Err(McpError::forbidden(format!(
                "'{}' can only be modified from localhost (current: {})",
                collection, caller.interface
            )));
        }

        // 1. Try exact match first - O(1)
        if let Some(&idx) = self.exact_match_index.get(collection) {
            return self.check_rule(&self.rules[idx], collection, caller, required);
        }

        // 2. Fall back to wildcard rules - O(w) where w is small
        for &idx in &self.wildcard_rules {
            let acl = &self.rules[idx];
            if acl.matches_collection(collection) {
                return self.check_rule(acl, collection, caller, required);
            }
        }

        // No matching rule found - deny by default
        Err(McpError::forbidden(format!(
            "'{}': no ACL rule for {} client",
            collection, caller.interface
        )))
    }

    /// Check a single ACL rule against caller and required permission
    fn check_rule(
        &self,
        acl: &CollectionAcl,
        collection: &str,
        caller: &CallerContext,
        required: RequiredPermission,
    ) -> Result<()> {
        // Find first matching principal
        for rule in &acl.rules {
            if rule.principal.matches(caller) {
                if rule.permissions.allows(required) {
                    return Ok(());
                } else {
                    return Err(McpError::forbidden(format!(
                        "'{}': {:?} permission required, not granted for {} client",
                        collection, required, caller.interface
                    )));
                }
            }
        }
        // No matching principal in this rule - deny
        Err(McpError::forbidden(format!(
            "'{}': no ACL rule for {} client",
            collection, caller.interface
        )))
    }

    /// Get the ACL for a specific collection (for listing)
    ///
    /// Performance: O(1) using index lookup
    pub fn get_collection_acl(&self, collection: &str) -> Option<&CollectionAcl> {
        // Use index for O(1) lookup instead of O(n) linear scan
        self.exact_match_index
            .get(collection)
            .map(|&idx| &self.rules[idx])
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
    /// SECURITY FIX: Handle poisoned RwLock gracefully instead of panicking
    pub fn check(
        &self,
        collection: &str,
        caller: &CallerContext,
        required: RequiredPermission,
    ) -> Result<()> {
        let config = self.config.read().map_err(|_| {
            McpError::internal("ACL config lock poisoned - service restart required".to_string())
        })?;
        config.check(collection, caller, required)
    }

    /// Reload ACL from database
    /// SECURITY FIX: Handle poisoned RwLock gracefully
    pub fn reload(&self) -> Result<()> {
        let new_config = AclConfig::load_from_db(&self.adapter)?;
        let mut config = self.config.write().map_err(|_| {
            McpError::internal("ACL config lock poisoned - service restart required".to_string())
        })?;
        *config = new_config;
        Ok(())
    }

    // NOTE: ACL writes (set/delete/cleanup) go through the tool handlers in
    // `tools/acl.rs`, which persist into `_system.acl` via the adapter. The live
    // in-memory config is then refreshed by `IronBaseService::execute_tool`,
    // which calls `reload()` after any successful ACL-mutating tool. There is
    // deliberately ONE write path and ONE reload point — the former duplicate
    // `set_acl`/`delete_acl` helpers (which re-implemented the upsert + reload)
    // had no callers and were removed to keep a single source of truth.

    /// List all ACLs
    /// SECURITY FIX: Handle poisoned RwLock gracefully
    pub fn list_all(&self) -> Vec<CollectionAcl> {
        match self.config.read() {
            Ok(config) => config.list_all().to_vec(),
            Err(_) => {
                tracing::error!("ACL config lock poisoned in list_all");
                Vec::new()
            }
        }
    }

    /// Get ACL for a collection
    /// SECURITY FIX: Handle poisoned RwLock gracefully
    pub fn get_acl(&self, collection: &str) -> Option<CollectionAcl> {
        match self.config.read() {
            Ok(config) => config.get_collection_acl(collection).cloned(),
            Err(_) => {
                tracing::error!("ACL config lock poisoned in get_acl");
                None
            }
        }
    }
}

// ============================================================================
// Tool permission mapping
// ============================================================================

// ----------------------------------------------------------------------------
// Declarative tool authorization policy — SINGLE SOURCE OF TRUTH
// ----------------------------------------------------------------------------
//
// Every tool's authorization profile is declared once in `tool_policy`, across
// four axes:
//   * permission        — ACL level checked against the target collection
//   * system_collection — implicit `_system.*` target for tools that have no
//                         `collection` argument (drives `check_acl`)
//   * localhost         — restrict invocation to loopback callers
//   * admin_key         — additionally require a valid IRONBASE_ADMIN_KEY
//
// The public `get_required_permission` / `get_system_collection_for_tool` /
// `requires_localhost` helpers and the central admin-key gate in
// `dispatch_tool` are all thin projections of this one table, so the four
// previously hand-synchronized lists can no longer drift apart.

/// System collection for scripts
const SYSTEM_SCRIPTS_COLLECTION: &str = "_system.scripts";
/// System collection for API keys
const SYSTEM_APIKEYS_COLLECTION: &str = "_system.api_keys";

/// Authorization policy for a single tool. See the table comment above.
#[derive(Debug, Clone, Copy)]
pub struct ToolPolicy {
    /// ACL permission level required on the target collection.
    pub permission: RequiredPermission,
    /// Implicit `_system.*` collection for tools without a `collection` arg.
    pub system_collection: Option<&'static str>,
    /// Tool may only be invoked from localhost (loopback).
    pub localhost: bool,
    /// Tool additionally requires a valid IRONBASE_ADMIN_KEY. Enforced in
    /// `dispatch_tool` (not `engine.rs`) ON PURPOSE: the stdio transport calls
    /// `dispatch_tool` directly and bypasses `execute_tool`'s ACL/localhost gates,
    /// so admin-key enforcement must live on the shared dispatch path to cover
    /// both transports. Do not "consolidate" it into `check_acl`.
    pub admin_key: bool,
    /// Tool mutates the persisted ACL config (`_system.acl`), so the live
    /// in-memory `AclConfig` must be reloaded after a successful call.
    pub mutates_acl: bool,
}

impl ToolPolicy {
    /// Base policy: ACL permission only (no system scope / localhost / admin key).
    fn new(permission: RequiredPermission) -> Self {
        Self {
            permission,
            system_collection: None,
            localhost: false,
            admin_key: false,
            mutates_acl: false,
        }
    }
    /// Mark an implicit `_system.*` target (tool has no `collection` argument).
    fn scope(mut self, collection: &'static str) -> Self {
        self.system_collection = Some(collection);
        self
    }
    /// Restrict to loopback callers.
    fn local(mut self) -> Self {
        self.localhost = true;
        self
    }
    /// Require IRONBASE_ADMIN_KEY (gated centrally in `dispatch_tool`).
    fn admin_key(mut self) -> Self {
        self.admin_key = true;
        self
    }
    /// Mark as mutating `_system.acl` → triggers a live `AclConfig` reload
    /// after the call (driven from `execute_tool`, not a separate name-list).
    fn mutates_acl(mut self) -> Self {
        self.mutates_acl = true;
        self
    }
}

/// The single authorization table. Unknown tools fall back to the conservative
/// historical default (Read, no system scope, no localhost, no admin key).
pub fn tool_policy(tool_name: &str) -> ToolPolicy {
    use RequiredPermission::{Admin, Read, Write};

    match tool_name {
        // ---- Data read ----
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
        | "find_with_hint"
        | "transaction_status"
        | "fulltext_search"
        | "fuzzy_search" => ToolPolicy::new(Read),

        // ---- Data write ----
        "insert_one"
        | "insert_many"
        | "update_one"
        | "update_many"
        | "delete_one"
        | "delete_many"
        | "begin_transaction"
        | "commit_transaction"
        | "rollback_transaction"
        | "insert_one_tx"
        | "update_one_tx"
        | "delete_one_tx" => ToolPolicy::new(Write),

        // ---- Structure admin (gated by ACL on the `collection` argument) ----
        "collection_create"
        | "collection_drop"
        | "collection_rename"
        | "index_create"
        | "index_drop"
        | "index_create_fulltext"
        | "index_create_fuzzy"
        | "schema_set" => ToolPolicy::new(Admin),

        // ---- Database admin ----
        // db_open was already loopback-only. db_stats/db_compact/db_checkpoint were
        // marked Admin/Read but had NO system collection and NO localhost gate, so
        // `check_acl` was skipped entirely (verified fail-open: db_stats leaked the
        // full collection catalog to an Internal caller). They take no `collection`
        // argument, so — consistent with db_open — they are now loopback-only.
        "db_open" | "db_stats" => ToolPolicy::new(Read).local(),
        "db_compact" | "db_checkpoint" => ToolPolicy::new(Admin).local(),

        // ---- ACL management (_system.acl) ----
        "acl_list" | "acl_get" => ToolPolicy::new(Read).scope(SYSTEM_ACL_COLLECTION),
        "acl_set" | "acl_delete" => ToolPolicy::new(Admin)
            .scope(SYSTEM_ACL_COLLECTION)
            .local()
            .mutates_acl(),
        // acl_cleanup deletes orphan rules from `_system.acl`, so it is gated like
        // acl_set/delete: Admin on the `_system.acl` scope + loopback-only. (It was
        // previously only loopback-gated with a Read level that check_acl never even
        // evaluated, since it carried no scope.)
        "acl_cleanup" => ToolPolicy::new(Admin)
            .scope(SYSTEM_ACL_COLLECTION)
            .local()
            .mutates_acl(),

        // ---- Listener management (_system.listeners) ----
        "listener_list" | "listener_get" => {
            ToolPolicy::new(Read).scope(crate::listener::SYSTEM_LISTENERS_COLLECTION)
        }
        "listener_add" | "listener_delete" | "listener_enable" | "listener_disable" => {
            ToolPolicy::new(Admin)
                .scope(crate::listener::SYSTEM_LISTENERS_COLLECTION)
                .local()
        }

        // ---- Script management (_system.scripts) ----
        "script_run" | "script_exec" | "script_list" | "script_get" | "script_history"
        | "script_stats" | "script_version_get" => {
            ToolPolicy::new(Read).scope(SYSTEM_SCRIPTS_COLLECTION)
        }
        "script_save" | "script_delete" | "script_rollback" | "script_tags_add"
        | "script_tags_remove" => ToolPolicy::new(Admin)
            .scope(SYSTEM_SCRIPTS_COLLECTION)
            .local(),

        // ---- Admin tools (loopback + admin key) ----
        "admin_list_all_collections"
        | "admin_create_system_collection"
        | "admin_set_collection_flags"
        | "admin_drop_protected" => ToolPolicy::new(Admin)
            .scope(SYSTEM_ACL_COLLECTION)
            .local()
            .admin_key(),
        "admin_apikey_create"
        | "admin_apikey_list"
        | "admin_apikey_revoke"
        | "admin_apikey_delete" => ToolPolicy::new(Admin)
            .scope(SYSTEM_APIKEYS_COLLECTION)
            .local()
            .admin_key(),

        // ---- Vector / RAG / embedding (gated by ACL on the `collection` arg,
        //      like their core counterparts). Previously these all fell to the
        //      Read default — leaving structure/config mutations (drop a vector
        //      index, create a RAG collection, change auto-embed config) requiring
        //      only Read. ----
        "index_create_vector"
        | "index_drop_vector"
        | "rag_collection_create"
        | "auto_embed_enable"
        | "auto_embed_disable" => ToolPolicy::new(Admin),
        "rag_document_import" | "embed_document" | "embed_job_cancel" | "index_stats_refresh" => {
            ToolPolicy::new(Write)
        }
        "index_list_vector"
        | "index_stats"
        | "rag_collection_stats"
        | "rag_load_all_chunks"
        | "vector_search"
        | "search"
        | "fulltext_analyze"
        | "embed_text"
        | "embed_batch"
        | "embed_list_models"
        | "embed_cache_stats"
        | "embed_job_list"
        | "embed_job_status"
        | "auto_embed_status" => ToolPolicy::new(Read),
        // Global embedding-cache wipe has no `collection` arg, so ACL can't gate it
        // by collection — loopback-only (like the db_* maintenance ops).
        "embed_cache_clear" => ToolPolicy::new(Admin).local(),

        // ---- Unknown tool: fail CLOSED ----
        // Every shipped tool is classified above. A tool with no explicit policy is
        // treated as Admin + loopback-only, so a newly added tool that someone
        // forgets to classify is denied to remote callers rather than silently
        // inheriting Read (the historical fail-open default).
        _ => ToolPolicy::new(Admin).local(),
    }
}

/// Get required permission for a tool (projection of [`tool_policy`]).
pub fn get_required_permission(tool_name: &str) -> RequiredPermission {
    tool_policy(tool_name).permission
}

/// Extract collection name from tool arguments.
///
/// Falls back to `old_collection` so collection_rename is ACL-gated on the
/// collection being renamed (you need admin rights on the source collection).
pub fn get_collection_from_args(args: &Value) -> Option<String> {
    args.get("collection")
        .or_else(|| args.get("old_collection"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Get the implicit system collection for a tool (projection of [`tool_policy`]).
/// Used for tools that have no `collection` argument (acl_*, script_*, admin_*).
pub fn get_system_collection_for_tool(tool_name: &str) -> Option<&'static str> {
    tool_policy(tool_name).system_collection
}

/// Check if a tool requires localhost access (projection of [`tool_policy`]).
/// These tools administer `_system.*` state and are restricted to loopback.
pub fn requires_localhost(tool_name: &str) -> bool {
    tool_policy(tool_name).localhost
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

    // ------------------------------------------------------------------
    // Declarative tool-policy table: parity with the historical 4 lists
    // plus the db_stats/db_compact/db_checkpoint fail-open fix.
    // ------------------------------------------------------------------

    #[test]
    fn test_tool_policy_permission_parity() {
        use RequiredPermission::*;
        // Read
        for t in [
            "find",
            "find_one",
            "count_documents",
            "distinct",
            "aggregate",
            "explain",
            "index_list",
            "schema_get",
            "collection_list",
            "db_stats",
            "db_open",
            "fulltext_search",
            "fuzzy_search",
            "acl_list",
            "acl_get",
            "listener_list",
            "script_run",
            "script_list",
        ] {
            assert_eq!(get_required_permission(t), Read, "{t} should be Read");
        }
        // Write
        for t in [
            "insert_one",
            "update_many",
            "delete_one",
            "begin_transaction",
            "insert_one_tx",
        ] {
            assert_eq!(get_required_permission(t), Write, "{t} should be Write");
        }
        // Admin
        for t in [
            "collection_create",
            "index_drop",
            "schema_set",
            "db_compact",
            "db_checkpoint",
            "acl_set",
            "acl_delete",
            "listener_add",
            "script_save",
            "admin_drop_protected",
            "admin_apikey_create",
        ] {
            assert_eq!(get_required_permission(t), Admin, "{t} should be Admin");
        }
        // Unknown → fail-closed default (Admin, loopback-only).
        assert_eq!(get_required_permission("totally_unknown_tool"), Admin);
        assert!(
            requires_localhost("totally_unknown_tool"),
            "unknown tool must be loopback-only (fail closed)"
        );
    }

    #[test]
    fn test_tool_policy_system_collection_parity() {
        assert_eq!(
            get_system_collection_for_tool("acl_set"),
            Some(SYSTEM_ACL_COLLECTION)
        );
        assert_eq!(
            get_system_collection_for_tool("acl_get"),
            Some(SYSTEM_ACL_COLLECTION)
        );
        assert_eq!(
            get_system_collection_for_tool("script_save"),
            Some(SYSTEM_SCRIPTS_COLLECTION)
        );
        assert_eq!(
            get_system_collection_for_tool("script_run"),
            Some(SYSTEM_SCRIPTS_COLLECTION)
        );
        assert_eq!(
            get_system_collection_for_tool("admin_apikey_create"),
            Some(SYSTEM_APIKEYS_COLLECTION)
        );
        assert_eq!(
            get_system_collection_for_tool("admin_drop_protected"),
            Some(SYSTEM_ACL_COLLECTION)
        );
        assert_eq!(
            get_system_collection_for_tool("listener_add"),
            Some(crate::listener::SYSTEM_LISTENERS_COLLECTION)
        );
        // acl_cleanup now carries the _system.acl scope (Admin-gated like acl_set/delete).
        assert_eq!(
            get_system_collection_for_tool("acl_cleanup"),
            Some(SYSTEM_ACL_COLLECTION)
        );
        // No implicit scope (acted on via `collection` arg, or none)
        assert_eq!(get_system_collection_for_tool("find"), None);
        assert_eq!(get_system_collection_for_tool("db_stats"), None);
    }

    #[test]
    fn test_tool_policy_localhost_parity_and_failopen_fix() {
        // Historically loopback-only
        for t in [
            "acl_set",
            "acl_delete",
            "acl_cleanup",
            "listener_add",
            "script_save",
            "db_open",
            "admin_list_all_collections",
            "admin_apikey_create",
        ] {
            assert!(requires_localhost(t), "{t} should require localhost");
        }
        // FAIL-OPEN FIX: these had no ACL scope and no localhost gate → check skipped.
        for t in ["db_stats", "db_compact", "db_checkpoint"] {
            assert!(
                requires_localhost(t),
                "{t} must now be loopback-only (fail-open fix)"
            );
        }
        // Unrestricted data ops
        for t in ["find", "insert_one", "fulltext_search", "acl_list"] {
            assert!(!requires_localhost(t), "{t} must not require localhost");
        }
    }

    #[test]
    fn test_tool_policy_admin_key_axis() {
        // Exactly the 8 admin_* tools require the admin key.
        for t in [
            "admin_list_all_collections",
            "admin_create_system_collection",
            "admin_set_collection_flags",
            "admin_drop_protected",
            "admin_apikey_create",
            "admin_apikey_list",
            "admin_apikey_revoke",
            "admin_apikey_delete",
        ] {
            assert!(tool_policy(t).admin_key, "{t} must require admin key");
        }
        // Nothing else does (incl. loopback-only system tools).
        for t in [
            "acl_set",
            "db_compact",
            "db_open",
            "script_save",
            "find",
            "unknown_tool",
        ] {
            assert!(!tool_policy(t).admin_key, "{t} must NOT require admin key");
        }
    }

    #[test]
    fn test_tool_policy_mutates_acl_axis() {
        // Exactly the ACL-write tools mutate _system.acl → drive the live reload.
        for t in ["acl_set", "acl_delete", "acl_cleanup"] {
            assert!(tool_policy(t).mutates_acl, "{t} must mark mutates_acl");
        }
        // Read-only ACL tools and everything else do not.
        for t in [
            "acl_list",
            "acl_get",
            "admin_apikey_create",
            "db_compact",
            "find",
            "unknown_tool",
        ] {
            assert!(!tool_policy(t).mutates_acl, "{t} must NOT mark mutates_acl");
        }
    }

    #[test]
    fn test_tool_policy_vector_rag_embed_classified() {
        use RequiredPermission::{Admin, Read, Write};
        // These previously fell to the Read default despite being mutations.
        for t in [
            "index_create_vector",
            "index_drop_vector",
            "rag_collection_create",
            "auto_embed_enable",
            "auto_embed_disable",
        ] {
            assert_eq!(get_required_permission(t), Admin, "{t} must be Admin");
        }
        for t in [
            "rag_document_import",
            "embed_document",
            "embed_job_cancel",
            "index_stats_refresh",
        ] {
            assert_eq!(get_required_permission(t), Write, "{t} must be Write");
        }
        for t in [
            "search",
            "vector_search",
            "rag_load_all_chunks",
            "fulltext_analyze",
            "embed_text",
            "index_stats",
            "auto_embed_status",
        ] {
            assert_eq!(get_required_permission(t), Read, "{t} must be Read");
        }
        // Global cache wipe has no collection arg → loopback-only.
        assert!(requires_localhost("embed_cache_clear"));
    }

    /// Completeness guard: every shipped tool must be explicitly classified, so
    /// none silently rides the fail-closed default. A new tool added without a
    /// `tool_policy` arm trips this (it would otherwise become Admin+loopback-only
    /// and surprise the author).
    #[test]
    fn test_every_shipped_tool_is_explicitly_classified() {
        let tools = crate::tools::get_tools_list();
        let names: Vec<String> = tools["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .filter_map(|t| t["name"].as_str().map(String::from))
            .collect();
        assert!(names.len() >= 90, "unexpectedly few tools: {}", names.len());
        // A guaranteed-unknown name yields the fail-closed default; no real tool
        // may share that exact (Admin, no-scope, loopback, no-admin-key) shape
        // *unless* it is intentionally one of the no-collection maintenance ops.
        let known_loopback_admin = ["db_compact", "db_checkpoint", "embed_cache_clear"];
        for name in &names {
            let p = tool_policy(name);
            let looks_like_default = p.permission == RequiredPermission::Admin
                && p.localhost
                && p.system_collection.is_none()
                && !p.admin_key;
            if looks_like_default {
                assert!(
                    known_loopback_admin.contains(&name.as_str()),
                    "tool '{name}' matches the fail-closed default shape — add an explicit tool_policy arm"
                );
            }
        }
    }
}
