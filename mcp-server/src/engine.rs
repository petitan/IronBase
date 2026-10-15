//! Service layer for IronBase MCP Server
//!
//! Provides protocol-independent business logic:
//! - Authentication (API key validation)
//! - Authorization (ACL checks)
//! - Tool dispatch
//!
//! This layer can be called from any transport (HTTP, stdio, WebSocket, gRPC).

use crate::acl::{
    collections_from_args, get_required_permission, get_system_collection_for_tool,
    requires_localhost, tool_policy, AclManager, CallerContext, InterfaceType,
};
use crate::adapter::IronBaseAdapter;
use crate::api_keys::ApiKeyCache;
use crate::embedding::EmbeddingManager;
use crate::jobs::JobManager;
use crate::scripting::LimitsManager;
use crate::tools::dispatch_tool;
use crate::ServerInfo;
use serde_json::Value;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

// ============================================================================
// Service Context - Protocol-independent request context
// ============================================================================

/// Protocol-independent request context
#[derive(Debug, Clone)]
pub struct ServiceContext {
    /// Caller information (interface type, remote address, API key)
    pub caller: CallerContext,
    /// Whether the server is initialized (MCP lifecycle)
    pub is_initialized: bool,
    /// Deadline for cooperative cancellation (if set)
    pub deadline: Option<Instant>,
    /// Cancellation flag for cooperative cancellation
    /// Set to true externally when request should be cancelled (timeout, client disconnect)
    pub cancel_flag: Option<Arc<AtomicBool>>,
}

impl ServiceContext {
    /// Create a new service context
    pub fn new(caller: CallerContext, is_initialized: bool, deadline: Option<Instant>) -> Self {
        Self {
            caller,
            is_initialized,
            deadline,
            cancel_flag: None,
        }
    }

    /// Create a new service context with cancellation support
    pub fn with_cancel_flag(
        caller: CallerContext,
        is_initialized: bool,
        deadline: Option<Instant>,
        cancel_flag: Arc<AtomicBool>,
    ) -> Self {
        Self {
            caller,
            is_initialized,
            deadline,
            cancel_flag: Some(cancel_flag),
        }
    }

    /// Create a localhost context (for stdio/internal calls)
    pub fn localhost() -> Self {
        Self {
            caller: CallerContext::localhost(),
            is_initialized: true,
            deadline: None,
            cancel_flag: None,
        }
    }
}

// ============================================================================
// Tool Request/Result - Protocol-independent tool invocation
// ============================================================================

/// Tool execution request
#[derive(Debug, Clone)]
pub struct ToolRequest {
    /// Tool name (e.g., "find", "insert_one")
    pub name: String,
    /// Tool arguments as JSON
    pub arguments: Value,
}

impl ToolRequest {
    pub fn new(name: impl Into<String>, arguments: Value) -> Self {
        Self {
            name: name.into(),
            arguments,
        }
    }
}

/// Tool execution result
#[derive(Debug)]
pub enum ToolResult {
    /// Successful execution with result value
    Success(Value),
    /// Execution error with code and message
    Error { code: i32, message: String },
    /// Access denied (authentication or authorization failure)
    AccessDenied(String),
}

impl ToolResult {
    /// Check if result is success
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success(_))
    }

    /// Check if result is an error
    pub fn is_error(&self) -> bool {
        !self.is_success()
    }
}

// ============================================================================
// IronBaseService - Central service layer
// ============================================================================

/// Central service layer for IronBase operations
///
/// This layer handles:
/// - API key validation
/// - ACL permission checks
/// - Tool dispatch
/// - Dynamic resource limits (memory-based)
///
/// It is protocol-independent and can be used from HTTP, stdio, or other transports.
pub struct IronBaseService {
    adapter: Arc<IronBaseAdapter>,
    acl_manager: AclManager,
    api_key_cache: ApiKeyCache,
    server_info: ServerInfo,
    require_api_key: bool,
    limits_manager: Arc<LimitsManager>,
    embedding_manager: Option<Arc<EmbeddingManager>>,
    job_manager: Option<Arc<JobManager>>,
}

impl IronBaseService {
    /// Create a new IronBase service
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        adapter: Arc<IronBaseAdapter>,
        acl_manager: AclManager,
        api_key_cache: ApiKeyCache,
        server_info: ServerInfo,
        require_api_key: bool,
        limits_manager: Arc<LimitsManager>,
        embedding_manager: Option<Arc<EmbeddingManager>>,
        job_manager: Option<Arc<JobManager>>,
    ) -> Self {
        Self {
            adapter,
            acl_manager,
            api_key_cache,
            server_info,
            require_api_key,
            limits_manager,
            embedding_manager,
            job_manager,
        }
    }

    /// Get the limits manager
    pub fn limits_manager(&self) -> &Arc<LimitsManager> {
        &self.limits_manager
    }

    /// Get the adapter for direct access
    pub fn adapter(&self) -> &Arc<IronBaseAdapter> {
        &self.adapter
    }

    /// Get the ACL manager
    pub fn acl_manager(&self) -> &AclManager {
        &self.acl_manager
    }

    /// Get the API key cache
    pub fn api_key_cache(&self) -> &ApiKeyCache {
        &self.api_key_cache
    }

    /// Get the server info
    pub fn server_info(&self) -> &ServerInfo {
        &self.server_info
    }

    /// Check if API key is required
    pub fn require_api_key(&self) -> bool {
        self.require_api_key
    }

    /// Get the embedding manager
    pub fn embedding_manager(&self) -> &Option<Arc<EmbeddingManager>> {
        &self.embedding_manager
    }

    /// Execute a tool with full authentication and authorization checks
    ///
    /// This is the main entry point for tool execution from any transport.
    /// SECURITY FIX #3: ACL config is snapshotted at request start to prevent TOCTOU attacks.
    /// This ensures the same ACL rules are used for both checking and execution.
    pub fn execute_tool(&self, ctx: &ServiceContext, request: &ToolRequest) -> ToolResult {
        // 1. API key validation
        if let Err(msg) = self.check_api_key(ctx, &request.name) {
            return ToolResult::AccessDenied(msg);
        }

        // 2. Localhost requirement check
        if let Err(msg) = self.check_localhost_requirement(ctx, &request.name) {
            return ToolResult::AccessDenied(msg);
        }

        // 3. ACL permission check
        // SECURITY: ACL check happens immediately before dispatch with no gaps
        // The AclManager uses internal locking to ensure consistent reads
        if let Err(msg) = self.check_acl(ctx, request) {
            return ToolResult::AccessDenied(msg);
        }

        // 4. Execute the tool (ACL was just verified - minimal TOCTOU window)
        // Note: ExecutionContext (deadline + cancel_flag) is already set by http_server.rs
        // via set_execution_context() in spawn_blocking, so no need to set it here.

        // Get current dynamic limits from LimitsManager
        let script_limits = self.limits_manager.get_limits();

        let result = match dispatch_tool(
            &request.name,
            request.arguments.clone(),
            &self.adapter,
            Some(&self.api_key_cache),
            Some(&self.server_info),
            Some(&script_limits),
            ctx.cancel_flag.clone(),
            &self.embedding_manager,
            &self.job_manager,
        ) {
            Ok(result) => ToolResult::Success(result),
            Err(e) => ToolResult::Error {
                code: e.code.code(),
                message: e.to_string(),
            },
        };

        // ACL-mutating tools persist into _system.acl via the adapter but do not
        // touch the live in-memory AclConfig that check_acl() reads. Without this
        // reload the new rule only takes effect after a restart (the handler still
        // reports success → silent "change not applied" bug). Reload here, at the
        // layer that owns the manager, so the change is active on the next request.
        // The "which tools mutate ACL" knowledge lives on the declarative
        // tool-policy table (`mutates_acl`), not a second scattered name-list.
        if result.is_success() && tool_policy(&request.name).mutates_acl {
            if let Err(e) = self.acl_manager.reload() {
                tracing::error!(
                    tool = %request.name,
                    "ACL reload failed: {} — rule persisted but NOT live until restart",
                    e
                );
            }
        }

        result
    }

    /// Check API key if required
    fn check_api_key(&self, ctx: &ServiceContext, tool_name: &str) -> Result<(), String> {
        if !self.require_api_key {
            return Ok(());
        }

        // Admin operations use admin_key, not api_key
        if tool_name.starts_with("admin_") {
            return Ok(());
        }

        match &ctx.caller.api_key {
            Some(key) if self.api_key_cache.validate(key, &self.adapter) => Ok(()),
            Some(_) => Err("Invalid API key".to_string()),
            None => Err(
                "API key required. Provide via 'Authorization: Bearer <key>' header or 'api_key' parameter."
                    .to_string(),
            ),
        }
    }

    /// Check localhost requirement for system tools
    fn check_localhost_requirement(
        &self,
        ctx: &ServiceContext,
        tool_name: &str,
    ) -> Result<(), String> {
        if requires_localhost(tool_name) && ctx.caller.interface != InterfaceType::Localhost {
            return Err(format!(
                "'{}' can only be called from localhost (current: {})",
                tool_name, ctx.caller.interface
            ));
        }
        Ok(())
    }

    /// Authorize an MCP `resources/read` like the equivalent tool call: API
    /// key, localhost gate and ACL. The resource routes used to read the
    /// database without any of these (MCP audit 2026-10-14 M1).
    pub fn authorize_resource(&self, ctx: &ServiceContext, uri: &str) -> Result<(), String> {
        let path = uri.strip_prefix("ironbase://").unwrap_or(uri);
        let (tool, args) = match path.split_once('/') {
            None if path == "stats" => ("db_stats", serde_json::json!({})),
            Some(("collections", name)) => ("find_one", serde_json::json!({"collection": name})),
            Some(("schemas", name)) => ("schema_get", serde_json::json!({"collection": name})),
            Some(("indexes", name)) => ("index_list", serde_json::json!({"collection": name})),
            // Unknown resource: read_resource rejects it
            _ => return Ok(()),
        };
        let request = ToolRequest::new(tool, args);
        self.check_api_key(ctx, tool)?;
        self.check_localhost_requirement(ctx, tool)?;
        self.check_acl(ctx, &request)
    }

    /// Check ACL permissions for collection operations
    fn check_acl(&self, ctx: &ServiceContext, request: &ToolRequest) -> Result<(), String> {
        // A tool with an implicit `_system.*` scope (acl_*, script_*, admin_*,
        // listener_*) is always checked on that scope: a `collection` argument
        // the caller adds must not replace it. Any other tool is checked on
        // every collection its arguments name (MCP audit 2026-10-14 M2).
        let collections_to_check = match get_system_collection_for_tool(&request.name) {
            Some(scope) => vec![scope.to_string()],
            None => collections_from_args(&request.arguments),
        };

        let required = get_required_permission(&request.name);
        self.acl_manager
            .check_all(&collections_to_check, &ctx.caller, required)
            .map_err(|e| format!("Access denied: {}", e))?;

        for collection in collections_to_check {
            // SECURITY FIX: Insert operations on non-existent collections are rejected.
            // Collections must be created explicitly via collection_create.
            // This prevents:
            // 1. Bypassing collection_create permission check through implicit creation
            // 2. Accidental collection creation due to typos (e.g., "userss" instead of "users")
            // 3. Transaction rollback inconsistency (collection remains after rollback)
            if is_collection_creating_insert(&request.name)
                && !self.adapter.collection_exists(&collection)
            {
                return Err(format!(
                    "Collection '{}' does not exist. Use collection_create to create it first.",
                    collection
                ));
            }
        }

        Ok(())
    }
}

// ============================================================================
// Helper functions
// ============================================================================

/// Check if a tool is an insert operation that can implicitly create collections
fn is_collection_creating_insert(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "insert_one" | "insert_many" | "transaction_insert_one"
    )
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_context_localhost() {
        let ctx = ServiceContext::localhost();
        assert_eq!(ctx.caller.interface, InterfaceType::Localhost);
        assert!(ctx.is_initialized);
    }

    #[test]
    fn test_tool_request_new() {
        let req = ToolRequest::new("find", serde_json::json!({"collection": "users"}));
        assert_eq!(req.name, "find");
        assert_eq!(req.arguments["collection"], "users");
    }

    #[test]
    fn test_tool_result_is_success() {
        let success = ToolResult::Success(serde_json::json!({"ok": true}));
        let error = ToolResult::Error {
            code: -1,
            message: "test".to_string(),
        };
        let denied = ToolResult::AccessDenied("test".to_string());

        assert!(success.is_success());
        assert!(!error.is_success());
        assert!(!denied.is_success());
    }

    #[test]
    fn test_is_collection_creating_insert() {
        // Insert operations that can implicitly create collections
        assert!(is_collection_creating_insert("insert_one"));
        assert!(is_collection_creating_insert("insert_many"));
        assert!(is_collection_creating_insert("transaction_insert_one"));

        // Non-insert operations
        assert!(!is_collection_creating_insert("find"));
        assert!(!is_collection_creating_insert("update_one"));
        assert!(!is_collection_creating_insert("delete_one"));
        assert!(!is_collection_creating_insert("collection_create"));
        assert!(!is_collection_creating_insert("transaction_update_one"));
        assert!(!is_collection_creating_insert("transaction_delete_one"));
    }

    /// A service over a fresh database, default ACL, no API key required.
    fn test_service(dir: &std::path::Path) -> IronBaseService {
        test_service_with_key(dir, false)
    }

    fn test_service_with_key(dir: &std::path::Path, require_api_key: bool) -> IronBaseService {
        let adapter = Arc::new(IronBaseAdapter::new(dir.join("acl.mlite")).unwrap());
        adapter
            .insert_one("users", serde_json::json!({"name": "a"}))
            .unwrap();
        IronBaseService::new(
            adapter.clone(),
            AclManager::new(adapter),
            ApiKeyCache::new(60, false),
            ServerInfo {
                protocol: "http".into(),
                host: "localhost".into(),
                port: 0,
                require_api_key,
            },
            require_api_key,
            LimitsManager::new(),
            None,
            None,
        )
    }

    fn external() -> ServiceContext {
        let addr: std::net::SocketAddr = "8.8.8.8:4000".parse().unwrap();
        ServiceContext::new(CallerContext::new(Some(addr), None), true, None)
    }

    /// M2: a `collection` argument the caller adds does not replace a tool's
    /// `_system.*` scope (script code ran with a readable decoy collection).
    #[test]
    fn test_decoy_collection_does_not_replace_system_scope() {
        let dir = tempfile::tempdir().unwrap();
        let service = test_service(dir.path());
        for (tool, args) in [
            (
                "script_exec",
                serde_json::json!({"collection": "users", "code": "1"}),
            ),
            ("acl_list", serde_json::json!({"collection": "users"})),
        ] {
            let result = service.execute_tool(&external(), &ToolRequest::new(tool, args));
            assert!(
                matches!(result, ToolResult::AccessDenied(_)),
                "{tool} ran for an external caller with a decoy collection"
            );
        }
        // Localhost keeps its access
        let result = service.execute_tool(
            &ServiceContext::localhost(),
            &ToolRequest::new("script_exec", serde_json::json!({"code": "1"})),
        );
        assert!(result.is_success(), "localhost script_exec denied");
    }

    /// M2: every collection argument is checked, not only the first one.
    #[test]
    fn test_collections_from_args_lists_every_target() {
        let args = serde_json::json!({
            "collection": "mine",
            "old_collection": "victim",
            "new_collection": "renamed"
        });
        assert_eq!(
            crate::acl::collections_from_args(&args),
            vec!["mine", "victim", "renamed"]
        );
    }

    /// M1: `resources/read` is authorized like the equivalent tool.
    #[test]
    fn test_resource_read_is_authorized() {
        let dir = tempfile::tempdir().unwrap();
        let service = test_service(dir.path());
        for uri in [
            "ironbase://collections/_system.api_keys",
            "ironbase://stats",
        ] {
            assert!(
                service.authorize_resource(&external(), uri).is_err(),
                "{uri} readable by an external caller"
            );
        }
        // Default ACL: external callers may read normal collections
        assert!(service
            .authorize_resource(&external(), "ironbase://collections/users")
            .is_ok());
        assert!(service
            .authorize_resource(&ServiceContext::localhost(), "ironbase://stats")
            .is_ok());
    }

    /// M1: with `require_api_key`, a resource read without a key is denied.
    #[test]
    fn test_resource_read_requires_api_key() {
        let dir = tempfile::tempdir().unwrap();
        let service = test_service_with_key(dir.path(), true);
        let err = service
            .authorize_resource(&external(), "ironbase://collections/users")
            .unwrap_err();
        assert!(err.contains("API key required"), "{err}");
    }
}
