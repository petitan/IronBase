//! Collection access of the `db_*` script functions (#42).
//!
//! `script_exec` / `script_run` are authorized on `_system.scripts` only, and
//! the collections a script then read or wrote were never checked: a caller
//! whose rights on a collection an ACL rule restricted could still read or
//! change it through a script. `IronBaseService::execute_tool` sets the
//! caller and an ACL snapshot for the run, and each `db_*` function checks
//! its collection with the permission of the MCP tool it corresponds to.
//! Without a set context (stdio, internal runs, tests) nothing is checked,
//! like direct tool calls on those paths.

use std::cell::RefCell;

use rhai::Dynamic;

use crate::acl::{get_required_permission, AclConfig, CallerContext};

struct ScriptAccess {
    acl: AclConfig,
    caller: CallerContext,
}

thread_local! {
    static SCRIPT_ACCESS: RefCell<Option<ScriptAccess>> = const { RefCell::new(None) };
}

/// Restores the previous script access context on drop
pub struct ScriptAccessGuard {
    previous: Option<ScriptAccess>,
}

impl Drop for ScriptAccessGuard {
    fn drop(&mut self) {
        let previous = self.previous.take();
        SCRIPT_ACCESS.with(|cell| *cell.borrow_mut() = previous);
    }
}

/// Check the `db_*` functions of scripts run on this thread against `acl`
/// for `caller`, until the guard is dropped
pub fn set_script_access(acl: AclConfig, caller: CallerContext) -> ScriptAccessGuard {
    let previous = SCRIPT_ACCESS.with(|cell| cell.replace(Some(ScriptAccess { acl, caller })));
    ScriptAccessGuard { previous }
}

/// The `db_*` error value (`"Error: Access denied: ..."`) when the current
/// caller may not use `collection` as the MCP tool `tool` requires; `None`
/// when allowed or when no access context is set
pub(crate) fn access_denied(tool: &str, collection: &str) -> Option<Dynamic> {
    SCRIPT_ACCESS.with(|cell| {
        let access = cell.borrow();
        let access = access.as_ref()?;
        access
            .acl
            .check(collection, &access.caller, get_required_permission(tool))
            .err()
            .map(|e| Dynamic::from(format!("Error: Access denied: {}", e)))
    })
}
