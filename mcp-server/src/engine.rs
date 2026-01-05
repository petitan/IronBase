//! Service layer for IronBase MCP Server
//!
//! Provides protocol-independent business logic:
//! - Authentication (API key validation)
//! - Authorization (ACL checks)
//! - Tool dispatch
//!
//! This layer can be called from any transport (HTTP, stdio, WebSocket, gRPC).

use crate::acl::{
    get_collection_from_args, get_required_permission, get_system_collection_for_tool,
    requires_localhost, AclManager, CallerContext, InterfaceType,
};
use crate::adapter::IronBaseAdapter;
use crate::api_keys::ApiKeyCache;
use crate::request_deadline;
use crate::scripting::LimitsManager;
use crate::tools::dispatch_tool;
use crate::ServerInfo;
use serde_json::Value;
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
}

impl ServiceContext {
    /// Create a new service context
    pub fn new(caller: CallerContext, is_initialized: bool, deadline: Option<Instant>) -> Self {
        Self {
            caller,
            is_initialized,
            deadline,
        }
    }

    /// Create a localhost context (for stdio/internal calls)
    pub fn localhost() -> Self {
        Self {
            caller: CallerContext::localhost(),
            is_initialized: true,
            deadline: None,
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
}

impl IronBaseService {
    /// Create a new IronBase service
    pub fn new(
        adapter: Arc<IronBaseAdapter>,
        acl_manager: AclManager,
        api_key_cache: ApiKeyCache,
        server_info: ServerInfo,
        require_api_key: bool,
        limits_manager: Arc<LimitsManager>,
    ) -> Self {
        Self {
            adapter,
            acl_manager,
            api_key_cache,
            server_info,
            require_api_key,
            limits_manager,
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
        let _deadline_guard = request_deadline::set_deadline(ctx.deadline);

        // Get current dynamic limits from LimitsManager
        let script_limits = self.limits_manager.get_limits();

        match dispatch_tool(
            &request.name,
            request.arguments.clone(),
            &self.adapter,
            Some(&self.api_key_cache),
            Some(&self.server_info),
            Some(&script_limits),
        ) {
            Ok(result) => ToolResult::Success(result),
            Err(e) => ToolResult::Error {
                code: -32000,
                message: e.to_string(),
            },
        }
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

    /// Check ACL permissions for collection operations
    fn check_acl(&self, ctx: &ServiceContext, request: &ToolRequest) -> Result<(), String> {
        // Determine collection to check:
        // 1. From tool arguments (regular operations)
        // 2. From tool name (system tools like acl_*, listener_*)
        let collection_to_check = get_collection_from_args(&request.arguments)
            .or_else(|| get_system_collection_for_tool(&request.name).map(String::from));

        if let Some(collection) = collection_to_check {
            let required = get_required_permission(&request.name);
            self.acl_manager
                .check(&collection, &ctx.caller, required)
                .map_err(|e| format!("Access denied: {}", e))?;
        }

        Ok(())
    }
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
}
