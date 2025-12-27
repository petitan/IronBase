// src/resources.rs
// MCP Resources implementation - exposes database structure as readable resources

use crate::adapter::IronBaseAdapter;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Resource definition for MCP resources/list
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    /// Unique URI identifying this resource
    pub uri: String,
    /// Human-readable name
    pub name: String,
    /// Optional description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// MIME type of the resource content
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

/// Response for resources/list
#[derive(Debug, Serialize)]
pub struct ResourcesListResponse {
    pub resources: Vec<Resource>,
}

/// Response for resources/read
#[derive(Debug, Serialize)]
pub struct ResourcesReadResponse {
    pub contents: Vec<ResourceContent>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceContent {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// Get list of all available resources from the database
pub fn get_resources_list(adapter: &IronBaseAdapter) -> Value {
    let mut resources = Vec::new();

    // Add database stats resource
    resources.push(Resource {
        uri: "ironbase://stats".to_string(),
        name: "Database Statistics".to_string(),
        description: Some("Overall database statistics and metrics".to_string()),
        mime_type: Some("application/json".to_string()),
    });

    // Add collections as resources
    let collections = adapter.list_collections();
    for coll_name in collections {
        // Skip system collections for non-localhost (they can still be read if authorized)
        if !coll_name.starts_with("_system.") {
            resources.push(Resource {
                uri: format!("ironbase://collections/{}", coll_name),
                name: coll_name.clone(),
                description: Some(format!("Collection: {}", coll_name)),
                mime_type: Some("application/json".to_string()),
            });

            // Add schema resource if collection has a schema
            resources.push(Resource {
                uri: format!("ironbase://schemas/{}", coll_name),
                name: format!("{} schema", coll_name),
                description: Some(format!("JSON Schema for collection {}", coll_name)),
                mime_type: Some("application/schema+json".to_string()),
            });

            // Add indexes resource
            resources.push(Resource {
                uri: format!("ironbase://indexes/{}", coll_name),
                name: format!("{} indexes", coll_name),
                description: Some(format!("Index definitions for collection {}", coll_name)),
                mime_type: Some("application/json".to_string()),
            });
        }
    }

    json!({
        "resources": resources
    })
}

/// Read a specific resource by URI
pub fn read_resource(adapter: &IronBaseAdapter, uri: &str) -> Result<Value, String> {
    // Parse the URI
    if !uri.starts_with("ironbase://") {
        return Err(format!("Invalid resource URI: {}. Must start with ironbase://", uri));
    }

    let path = &uri["ironbase://".len()..];
    let parts: Vec<&str> = path.splitn(2, '/').collect();

    match parts.as_slice() {
        ["stats"] => read_stats_resource(adapter),
        ["collections", name] => read_collection_resource(adapter, name),
        ["schemas", name] => read_schema_resource(adapter, name),
        ["indexes", name] => read_indexes_resource(adapter, name),
        _ => Err(format!("Unknown resource type in URI: {}", uri)),
    }
}

fn read_stats_resource(adapter: &IronBaseAdapter) -> Result<Value, String> {
    let stats = adapter.stats();

    Ok(json!({
        "contents": [{
            "uri": "ironbase://stats",
            "mimeType": "application/json",
            "text": serde_json::to_string_pretty(&stats).unwrap_or_else(|_| "{}".to_string())
        }]
    }))
}

fn read_collection_resource(adapter: &IronBaseAdapter, name: &str) -> Result<Value, String> {
    // Get collection info: document count and sample document structure
    let count = adapter
        .count_documents(name, json!({}))
        .map_err(|e| e.to_string())?;

    // Get one sample document to show structure
    let sample = adapter
        .find_one(name, json!({}))
        .map_err(|e| e.to_string())?;

    let info = json!({
        "collection": name,
        "documentCount": count,
        "sampleDocument": sample
    });

    Ok(json!({
        "contents": [{
            "uri": format!("ironbase://collections/{}", name),
            "mimeType": "application/json",
            "text": serde_json::to_string_pretty(&info).unwrap_or_else(|_| "{}".to_string())
        }]
    }))
}

fn read_schema_resource(adapter: &IronBaseAdapter, name: &str) -> Result<Value, String> {
    let schema = adapter.get_schema(name).map_err(|e| e.to_string())?;

    let schema_text = match schema {
        Some(s) => serde_json::to_string_pretty(&s).unwrap_or_else(|_| "null".to_string()),
        None => "null".to_string(),
    };

    Ok(json!({
        "contents": [{
            "uri": format!("ironbase://schemas/{}", name),
            "mimeType": "application/schema+json",
            "text": schema_text
        }]
    }))
}

fn read_indexes_resource(adapter: &IronBaseAdapter, name: &str) -> Result<Value, String> {
    let indexes = adapter.list_indexes(name).map_err(|e| e.to_string())?;

    Ok(json!({
        "contents": [{
            "uri": format!("ironbase://indexes/{}", name),
            "mimeType": "application/json",
            "text": serde_json::to_string_pretty(&indexes).unwrap_or_else(|_| "[]".to_string())
        }]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resource_serialization() {
        let resource = Resource {
            uri: "ironbase://stats".to_string(),
            name: "Database Statistics".to_string(),
            description: Some("Test".to_string()),
            mime_type: Some("application/json".to_string()),
        };

        let json = serde_json::to_value(&resource).unwrap();
        assert_eq!(json["uri"], "ironbase://stats");
        assert_eq!(json["name"], "Database Statistics");
        assert_eq!(json["mimeType"], "application/json");
    }
}
