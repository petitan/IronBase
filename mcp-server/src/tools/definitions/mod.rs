//! Tool JSON definitions for MCP tools/list
//!
//! This module is organized by domain for better maintainability:
//! - `common` - Reusable field definitions (DRY)
//! - `database` - Database management (db_open, db_stats, etc.)
//! - `collection` - Collection management
//! - `document` - Document CRUD operations
//! - `index` - Index management and search
//! - `schema` - Schema validation
//! - `script` - Rhai script management
//! - `transaction` - Transaction management
//! - `admin` - Admin operations (require IRONBASE_ADMIN_KEY)
//! - `acl` - Access Control List management
//! - `listener` - HTTP/HTTPS listener configuration

mod acl;
mod admin;
mod collection;
pub mod common;
mod database;
mod document;
mod index;
mod listener;
mod schema;
mod script;
mod transaction;

use serde_json::{json, Value};

/// Get all tool definitions as JSON
///
/// Returns a JSON object with a "tools" array containing all available
/// MCP tool definitions organized by category.
pub fn get_all_tools_json() -> Value {
    let mut tools = Vec::with_capacity(64);

    // Database Management
    tools.extend(database::tools());

    // Collection Management
    tools.extend(collection::tools());

    // Document CRUD
    tools.extend(document::tools());

    // Index Management & Search
    tools.extend(index::tools());

    // Schema Management
    tools.extend(schema::tools());

    // Script Management
    tools.extend(script::tools());

    // Transaction Management
    tools.extend(transaction::tools());

    // Admin Operations
    tools.extend(admin::tools());

    // ACL Management
    tools.extend(acl::tools());

    // Listener Management
    tools.extend(listener::tools());

    json!({ "tools": tools })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_tools_have_required_fields() {
        let tools_json = get_all_tools_json();
        let tools = tools_json["tools"].as_array().unwrap();

        for tool in tools {
            assert!(tool.get("name").is_some(), "Tool missing name: {:?}", tool);
            assert!(
                tool.get("title").is_some(),
                "Tool missing title: {:?}",
                tool
            );
            assert!(
                tool.get("description").is_some(),
                "Tool missing description: {:?}",
                tool
            );
            assert!(
                tool.get("inputSchema").is_some(),
                "Tool missing inputSchema: {:?}",
                tool
            );
        }
    }

    #[test]
    fn test_tool_count() {
        let tools_json = get_all_tools_json();
        let tools = tools_json["tools"].as_array().unwrap();

        // Verify we have the expected number of tools
        // Database: 4, Collection: 3, Document: 11, Index: 10, Schema: 2,
        // Script: 12, Transaction: 7, Admin: 8, ACL: 5, Listener: 6 = 68 total
        assert!(
            tools.len() >= 60,
            "Expected at least 60 tools, got {}",
            tools.len()
        );
    }

    #[test]
    fn test_no_duplicate_tool_names() {
        let tools_json = get_all_tools_json();
        let tools = tools_json["tools"].as_array().unwrap();

        let mut names: Vec<&str> = tools
            .iter()
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
            .collect();

        let original_len = names.len();
        names.sort();
        names.dedup();

        assert_eq!(names.len(), original_len, "Duplicate tool names detected");
    }

    #[test]
    fn test_input_schema_structure() {
        let tools_json = get_all_tools_json();
        let tools = tools_json["tools"].as_array().unwrap();

        for tool in tools {
            let name = tool
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("unknown");
            let schema = tool.get("inputSchema").expect("Missing inputSchema");

            // Every schema should have type = "object"
            assert_eq!(
                schema.get("type").and_then(|t| t.as_str()),
                Some("object"),
                "Tool '{}' inputSchema type is not 'object'",
                name
            );

            // Every schema should have properties (even if empty)
            assert!(
                schema.get("properties").is_some(),
                "Tool '{}' missing properties in inputSchema",
                name
            );
        }
    }
}
