//! ACL management tool handlers

use crate::acl::{Permissions, Principal, SYSTEM_ACL_COLLECTION};
use crate::adapter::{FindOptions, IronBaseAdapter};
use crate::error::{McpError, Result};
use serde_json::{json, Value};
use std::sync::Arc;

use super::helpers::{get_array, get_string};

/// Dispatch ACL tool calls
pub fn dispatch(name: &str, params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    match name {
        "acl_list" => handle_acl_list(adapter),
        "acl_get" => handle_acl_get(params, adapter),
        "acl_set" => handle_acl_set(params, adapter),
        "acl_delete" => handle_acl_delete(params, adapter),
        "acl_cleanup" => handle_acl_cleanup(adapter),
        _ => Err(McpError::InvalidParams(format!(
            "Unknown ACL tool: {}",
            name
        ))),
    }
}

fn handle_acl_list(adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    match adapter.find(SYSTEM_ACL_COLLECTION, json!({}), FindOptions::default()) {
        Ok(result) => Ok(json!({
            "rules": result.documents,
            "count": result.documents.len(),
            "note": "Built-in rules (_system.* protection) are not shown here"
        })),
        Err(_) => {
            // Collection doesn't exist yet - no custom rules
            Ok(json!({
                "rules": [],
                "count": 0,
                "note": "No custom ACL rules defined. Default rules apply."
            }))
        }
    }
}

fn handle_acl_get(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collection = get_string(&params, "collection")?;

    match adapter.find_one(SYSTEM_ACL_COLLECTION, json!({"collection": collection})) {
        Ok(Some(doc)) => Ok(doc),
        Ok(None) => Ok(json!({
            "collection": collection,
            "rules": null,
            "note": "No custom ACL for this collection. Default rules apply."
        })),
        Err(_) => Ok(json!({
            "collection": collection,
            "rules": null,
            "note": "No custom ACL for this collection. Default rules apply."
        })),
    }
}

fn handle_acl_set(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collection = get_string(&params, "collection")?;
    let rules_arr = get_array(&params, "rules")?;

    // Validate that collection exists (except for wildcard "*")
    if collection != "*" {
        let collections = adapter.list_collections();
        if !collections.contains(&collection) {
            return Err(McpError::InvalidParams(format!(
                "Collection '{}' does not exist. Create it first before setting ACL.",
                collection
            )));
        }
    }

    // Parse rules
    let mut parsed_rules: Vec<Value> = Vec::new();
    for rule_value in rules_arr {
        let principal_str = rule_value
            .get("principal")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("Rule missing 'principal'".into()))?;

        let permissions_str = rule_value
            .get("permissions")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("Rule missing 'permissions'".into()))?;

        // Validate principal format
        let principal = Principal::parse(principal_str)?;
        let permissions = Permissions::parse(permissions_str);

        parsed_rules.push(json!({
            "principal": principal,
            "permissions": permissions
        }));
    }

    let acl_doc = json!({
        "collection": collection,
        "rules": parsed_rules
    });

    // Upsert into _system.acl
    let filter = json!({"collection": collection});
    match adapter.find_one(SYSTEM_ACL_COLLECTION, filter.clone()) {
        Ok(Some(_)) => {
            // Update existing
            adapter.update_one(SYSTEM_ACL_COLLECTION, filter, json!({"$set": acl_doc}))?;
        }
        _ => {
            // Insert new
            adapter.insert_one(SYSTEM_ACL_COLLECTION, acl_doc)?;
        }
    }

    Ok(json!({
        "success": true,
        "collection": collection,
        "rules_count": parsed_rules.len(),
        "note": "ACL updated. Changes take effect on next request."
    }))
}

fn handle_acl_delete(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collection = get_string(&params, "collection")?;

    // Prevent deleting built-in rules
    if collection == "_system.*" {
        return Err(McpError::InvalidParams(
            "Cannot delete built-in _system.* ACL rules".into(),
        ));
    }

    let filter = json!({"collection": collection});
    let deleted = adapter.delete_one(SYSTEM_ACL_COLLECTION, filter)?;

    Ok(json!({
        "success": true,
        "collection": collection,
        "deleted": deleted > 0,
        "note": if deleted > 0 {
            "ACL deleted. Default rules now apply."
        } else {
            "No custom ACL found for this collection."
        }
    }))
}

fn handle_acl_cleanup(adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let existing_collections = adapter.list_collections();

    // Get all ACL rules
    let acl_result = adapter.find(SYSTEM_ACL_COLLECTION, json!({}), FindOptions::default())?;

    let mut orphans: Vec<String> = Vec::new();
    for doc in &acl_result.documents {
        if let Some(coll) = doc.get("collection").and_then(|v| v.as_str()) {
            // Skip wildcard rules
            if coll == "*" {
                continue;
            }
            // Check if collection exists
            if !existing_collections.contains(&coll.to_string()) {
                orphans.push(coll.to_string());
            }
        }
    }

    // Delete orphan ACLs
    let mut deleted_count = 0;
    for orphan in &orphans {
        let filter = json!({"collection": orphan});
        if adapter
            .delete_one(SYSTEM_ACL_COLLECTION, filter)
            .unwrap_or(0)
            > 0
        {
            deleted_count += 1;
        }
    }

    Ok(json!({
        "success": true,
        "orphans_found": orphans.len(),
        "orphans_deleted": deleted_count,
        "collections": orphans
    }))
}
