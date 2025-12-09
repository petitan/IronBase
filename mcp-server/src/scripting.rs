//! Script management and execution for IronBase MCP Server
//!
//! Provides CRUD operations for scripts stored in _system.scripts collection.
//! Scripts are stored as JSON documents with name, code, and description.
//!
//! Also provides Rhai script execution engine with database bindings.

use crate::adapter::{IronBaseAdapter, FindOptions as AdapterFindOptions, SCRIPTS_COLLECTION};
use crate::error::{McpError, Result};
use base64::Engine as Base64Engine;
use parking_lot::Mutex;
use rhai::{Dynamic, Engine, EvalAltResult, Map, Scope};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

/// Script metadata returned by list
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptInfo {
    pub name: String,
    pub description: Option<String>,
    pub created_at: Option<String>,
}

/// Full script with code
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Script {
    pub name: String,
    pub code: String,
    pub description: Option<String>,
    pub created_at: Option<String>,
}

/// Script Manager - CRUD operations for scripts
pub struct ScriptManager {
    adapter: Arc<IronBaseAdapter>,
}

impl ScriptManager {
    /// Create a new ScriptManager
    pub fn new(adapter: Arc<IronBaseAdapter>) -> Self {
        Self { adapter }
    }

    /// Save a script (insert or update)
    pub fn save(&self, name: &str, code: &str, description: Option<&str>) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();

        // Check if script exists
        let existing = self.get(name)?;

        if existing.is_some() {
            // Update existing script
            self.adapter.update_one(
                SCRIPTS_COLLECTION,
                json!({"_id": name}),
                json!({
                    "$set": {
                        "code": code,
                        "description": description,
                        "updated_at": now
                    }
                }),
            )?;
        } else {
            // Insert new script
            let doc = json!({
                "_id": name,
                "code": code,
                "description": description,
                "created_at": now
            });
            self.adapter.insert_one(SCRIPTS_COLLECTION, doc)?;
        }

        Ok(())
    }

    /// List all scripts (without code)
    pub fn list(&self) -> Result<Vec<ScriptInfo>> {
        let docs = self.adapter.find(
            SCRIPTS_COLLECTION,
            json!({}),
            crate::adapter::FindOptions {
                projection: Some(json!({"code": 0})), // Exclude code for listing
                ..Default::default()
            },
        )?;

        let scripts = docs
            .documents
            .into_iter()
            .filter_map(|doc| {
                Some(ScriptInfo {
                    name: doc.get("_id")?.as_str()?.to_string(),
                    description: doc.get("description").and_then(|v| v.as_str()).map(String::from),
                    created_at: doc.get("created_at").and_then(|v| v.as_str()).map(String::from),
                })
            })
            .collect();

        Ok(scripts)
    }

    /// Get a script by name (with code)
    pub fn get(&self, name: &str) -> Result<Option<Script>> {
        let doc = self.adapter.find_one(SCRIPTS_COLLECTION, json!({"_id": name}))?;

        match doc {
            Some(doc) => {
                let script = Script {
                    name: doc.get("_id").and_then(|v| v.as_str()).unwrap_or(name).to_string(),
                    code: doc
                        .get("code")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    description: doc.get("description").and_then(|v| v.as_str()).map(String::from),
                    created_at: doc.get("created_at").and_then(|v| v.as_str()).map(String::from),
                };
                Ok(Some(script))
            }
            None => Ok(None),
        }
    }

    /// Delete a script by name
    pub fn delete(&self, name: &str) -> Result<bool> {
        let count = self.adapter.delete_one(SCRIPTS_COLLECTION, json!({"_id": name}))?;
        Ok(count > 0)
    }
}

// ============================================================
// Rhai Script Execution Engine
// ============================================================

/// Maximum script execution time in milliseconds
const MAX_EXECUTION_TIME_MS: u64 = 60_000; // 60 seconds

/// Maximum number of operations per script
const MAX_OPERATIONS: u64 = 1_000_000;

/// Result of script execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptResult {
    pub result: Value,
    pub logs: Vec<String>,
}

/// Rhai script execution engine with IronBase bindings
pub struct RhaiEngine {
    adapter: Arc<IronBaseAdapter>,
}

impl RhaiEngine {
    /// Create a new RhaiEngine
    pub fn new(adapter: Arc<IronBaseAdapter>) -> Self {
        Self { adapter }
    }

    /// Run a script with optional parameters
    pub fn run(&self, code: &str, params: Option<Value>) -> Result<ScriptResult> {
        let mut engine = Engine::new();

        // Security: Disable dangerous operations
        engine.set_max_operations(MAX_OPERATIONS);

        // Create logs collector (Arc<Mutex> for thread safety)
        let logs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let logs_clone = logs.clone();

        // Register print function to capture logs
        engine.on_print(move |s| {
            logs_clone.lock().push(s.to_string());
        });

        // Register db module with database operations
        let adapter = self.adapter.clone();
        self.register_db_functions(&mut engine, adapter)?;

        // Register utility functions (base64, etc.)
        Self::register_utility_functions(&mut engine);

        // Create scope with params
        let mut scope = Scope::new();
        if let Some(params) = params {
            let params_dynamic = json_to_dynamic(&params);
            scope.push("params", params_dynamic);
        }

        // Execute the script
        let start = std::time::Instant::now();
        let result = engine.eval_with_scope::<Dynamic>(&mut scope, code);

        // Check timeout
        if start.elapsed().as_millis() > MAX_EXECUTION_TIME_MS as u128 {
            return Err(McpError::ScriptError("Script execution timed out".into()));
        }

        match result {
            Ok(value) => {
                let json_result = dynamic_to_json(&value);
                Ok(ScriptResult {
                    result: json_result,
                    logs: logs.lock().clone(),
                })
            }
            Err(e) => Err(McpError::ScriptError(format_rhai_error(&e))),
        }
    }

    /// Register database functions into the engine
    fn register_db_functions(&self, engine: &mut Engine, adapter: Arc<IronBaseAdapter>) -> Result<()> {
        // db_find(collection, query) -> array of documents
        let adapter_find = adapter.clone();
        engine.register_fn("db_find", move |collection: &str, query: Map| -> Dynamic {
            let query_json = map_to_json(&query);
            match adapter_find.find(collection, query_json, AdapterFindOptions::default()) {
                Ok(result) => {
                    let docs: Vec<Dynamic> = result.documents.into_iter()
                        .map(|d| json_to_dynamic(&d))
                        .collect();
                    Dynamic::from(docs)
                }
                Err(e) => Dynamic::from(format!("Error: {}", e))
            }
        });

        // db_find_one(collection, query) -> document or ()
        let adapter_find_one = adapter.clone();
        engine.register_fn("db_find_one", move |collection: &str, query: Map| -> Dynamic {
            let query_json = map_to_json(&query);
            match adapter_find_one.find_one(collection, query_json) {
                Ok(Some(doc)) => json_to_dynamic(&doc),
                Ok(None) => Dynamic::UNIT,
                Err(e) => Dynamic::from(format!("Error: {}", e))
            }
        });

        // db_insert_one(collection, document) -> inserted_id
        let adapter_insert = adapter.clone();
        engine.register_fn("db_insert_one", move |collection: &str, doc: Map| -> Dynamic {
            let doc_json = map_to_json(&doc);
            match adapter_insert.insert_one(collection, doc_json) {
                Ok(id) => Dynamic::from(id), // insert_one returns String directly
                Err(e) => Dynamic::from(format!("Error: {}", e))
            }
        });

        // db_update_one(collection, filter, update) -> {matched_count, modified_count}
        let adapter_update_one = adapter.clone();
        engine.register_fn("db_update_one", move |collection: &str, filter: Map, update: Map| -> Dynamic {
            let filter_json = map_to_json(&filter);
            let update_json = map_to_json(&update);
            match adapter_update_one.update_one(collection, filter_json, update_json) {
                Ok(result) => {
                    let mut map = Map::new();
                    map.insert("matched_count".into(), Dynamic::from(result.matched_count as i64));
                    map.insert("modified_count".into(), Dynamic::from(result.modified_count as i64));
                    Dynamic::from(map)
                }
                Err(e) => Dynamic::from(format!("Error: {}", e))
            }
        });

        // db_update_many(collection, filter, update) -> {matched_count, modified_count}
        let adapter_update_many = adapter.clone();
        engine.register_fn("db_update_many", move |collection: &str, filter: Map, update: Map| -> Dynamic {
            let filter_json = map_to_json(&filter);
            let update_json = map_to_json(&update);
            match adapter_update_many.update_many(collection, filter_json, update_json) {
                Ok(result) => {
                    let mut map = Map::new();
                    map.insert("matched_count".into(), Dynamic::from(result.matched_count as i64));
                    map.insert("modified_count".into(), Dynamic::from(result.modified_count as i64));
                    Dynamic::from(map)
                }
                Err(e) => Dynamic::from(format!("Error: {}", e))
            }
        });

        // db_delete_one(collection, filter) -> deleted_count
        let adapter_delete_one = adapter.clone();
        engine.register_fn("db_delete_one", move |collection: &str, filter: Map| -> Dynamic {
            let filter_json = map_to_json(&filter);
            match adapter_delete_one.delete_one(collection, filter_json) {
                Ok(count) => Dynamic::from(count as i64),
                Err(e) => Dynamic::from(format!("Error: {}", e))
            }
        });

        // db_delete_many(collection, filter) -> deleted_count
        let adapter_delete_many = adapter.clone();
        engine.register_fn("db_delete_many", move |collection: &str, filter: Map| -> Dynamic {
            let filter_json = map_to_json(&filter);
            match adapter_delete_many.delete_many(collection, filter_json) {
                Ok(count) => Dynamic::from(count as i64),
                Err(e) => Dynamic::from(format!("Error: {}", e))
            }
        });

        // db_count(collection, query) -> count
        let adapter_count = adapter.clone();
        engine.register_fn("db_count", move |collection: &str, query: Map| -> Dynamic {
            let query_json = map_to_json(&query);
            match adapter_count.count_documents(collection, query_json) {
                Ok(count) => Dynamic::from(count as i64),
                Err(e) => Dynamic::from(format!("Error: {}", e))
            }
        });

        // db_aggregate(collection, pipeline) -> array of documents
        let adapter_agg = adapter.clone();
        engine.register_fn("db_aggregate", move |collection: &str, pipeline: rhai::Array| -> Dynamic {
            // Convert Rhai Array to Vec<Value>
            let pipeline_vec: Vec<Value> = pipeline.iter()
                .map(dynamic_to_json)
                .collect();
            match adapter_agg.aggregate(collection, pipeline_vec) {
                Ok(docs) => {
                    let result: Vec<Dynamic> = docs.into_iter()
                        .map(|d| json_to_dynamic(&d))
                        .collect();
                    Dynamic::from(result)
                }
                Err(e) => Dynamic::from(format!("Error: {}", e))
            }
        });

        Ok(())
    }

    /// Register utility functions (base64, etc.)
    fn register_utility_functions(engine: &mut Engine) {
        // base64_encode(string) -> base64 encoded string
        engine.register_fn("base64_encode", |s: &str| -> String {
            base64::engine::general_purpose::STANDARD.encode(s.as_bytes())
        });

        // base64_decode(base64_string) -> decoded string or error
        engine.register_fn("base64_decode", |s: &str| -> Dynamic {
            match base64::engine::general_purpose::STANDARD.decode(s) {
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(decoded) => Dynamic::from(decoded),
                    Err(e) => Dynamic::from(format!("Error: Invalid UTF-8: {}", e)),
                },
                Err(e) => Dynamic::from(format!("Error: Invalid base64: {}", e)),
            }
        });
    }
}

// ============================================================
// JSON <-> Rhai Dynamic Conversion Helpers
// ============================================================

/// Convert serde_json::Value to Rhai Dynamic
fn json_to_dynamic(value: &Value) -> Dynamic {
    match value {
        Value::Null => Dynamic::UNIT,
        Value::Bool(b) => Dynamic::from(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Dynamic::from(i)
            } else if let Some(f) = n.as_f64() {
                Dynamic::from(f)
            } else {
                Dynamic::UNIT
            }
        }
        Value::String(s) => Dynamic::from(s.clone()),
        Value::Array(arr) => {
            let vec: Vec<Dynamic> = arr.iter().map(json_to_dynamic).collect();
            Dynamic::from(vec)
        }
        Value::Object(obj) => {
            let mut map = Map::new();
            for (k, v) in obj {
                map.insert(k.clone().into(), json_to_dynamic(v));
            }
            Dynamic::from(map)
        }
    }
}

/// Convert Rhai Dynamic to serde_json::Value
fn dynamic_to_json(value: &Dynamic) -> Value {
    if value.is_unit() {
        Value::Null
    } else if value.is_bool() {
        Value::Bool(value.as_bool().unwrap_or(false))
    } else if value.is_int() {
        Value::Number(value.as_int().unwrap_or(0).into())
    } else if value.is_float() {
        if let Ok(f) = value.as_float() {
            serde_json::Number::from_f64(f)
                .map(Value::Number)
                .unwrap_or(Value::Null)
        } else {
            Value::Null
        }
    } else if value.is_string() {
        Value::String(value.clone().into_string().unwrap_or_default())
    } else if value.is_array() {
        let arr: Vec<Dynamic> = value.clone().into_array().unwrap_or_default();
        Value::Array(arr.iter().map(dynamic_to_json).collect())
    } else if value.is_map() {
        let map: Map = value.clone().cast::<Map>();
        let mut obj = serde_json::Map::new();
        for (k, v) in map {
            obj.insert(k.to_string(), dynamic_to_json(&v));
        }
        Value::Object(obj)
    } else {
        // Try to convert to string as fallback
        Value::String(value.to_string())
    }
}

/// Convert Rhai Map to serde_json::Value
fn map_to_json(map: &Map) -> Value {
    let mut obj = serde_json::Map::new();
    for (k, v) in map {
        obj.insert(k.to_string(), dynamic_to_json(v));
    }
    Value::Object(obj)
}

/// Format Rhai error for display
fn format_rhai_error(err: &EvalAltResult) -> String {
    match err {
        EvalAltResult::ErrorTooManyOperations(_) => {
            "Script exceeded maximum operation limit (10,000)".to_string()
        }
        _ => format!("Script error: {}", err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn create_test_adapter() -> (Arc<IronBaseAdapter>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        let adapter = Arc::new(IronBaseAdapter::new(&db_path).unwrap());
        (adapter, temp_dir)
    }

    #[test]
    fn test_script_save_and_get() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        // Save a script
        manager
            .save("test_script", "let x = 1 + 1;", Some("Test script"))
            .unwrap();

        // Get it back
        let script = manager.get("test_script").unwrap().unwrap();
        assert_eq!(script.name, "test_script");
        assert_eq!(script.code, "let x = 1 + 1;");
        assert_eq!(script.description, Some("Test script".to_string()));
    }

    #[test]
    fn test_script_list() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        // Save multiple scripts
        manager.save("script1", "code1", Some("First")).unwrap();
        manager.save("script2", "code2", Some("Second")).unwrap();

        // List them
        let scripts = manager.list().unwrap();
        assert_eq!(scripts.len(), 2);

        let names: Vec<&str> = scripts.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"script1"));
        assert!(names.contains(&"script2"));
    }

    #[test]
    fn test_script_update() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        // Save initial version
        manager.save("updatable", "v1", Some("Version 1")).unwrap();

        // Update it
        manager.save("updatable", "v2", Some("Version 2")).unwrap();

        // Get updated version
        let script = manager.get("updatable").unwrap().unwrap();
        assert_eq!(script.code, "v2");
        assert_eq!(script.description, Some("Version 2".to_string()));

        // Should still be only one script
        let scripts = manager.list().unwrap();
        assert_eq!(scripts.len(), 1);
    }

    #[test]
    fn test_script_delete() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        // Save and delete
        manager.save("deletable", "code", None).unwrap();
        let deleted = manager.delete("deletable").unwrap();
        assert!(deleted);

        // Should be gone
        let script = manager.get("deletable").unwrap();
        assert!(script.is_none());

        // Delete non-existent should return false
        let deleted = manager.delete("nonexistent").unwrap();
        assert!(!deleted);
    }

    #[test]
    fn test_get_nonexistent() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        let script = manager.get("nonexistent").unwrap();
        assert!(script.is_none());
    }

    // ============================================================
    // RhaiEngine Tests
    // ============================================================

    #[test]
    fn test_rhai_simple_arithmetic() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter);

        let result = engine.run("1 + 2", None).unwrap();
        assert_eq!(result.result, json!(3));
    }

    #[test]
    fn test_rhai_string_operations() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter);

        let result = engine.run(r#""hello" + " world""#, None).unwrap();
        assert_eq!(result.result, json!("hello world"));
    }

    #[test]
    fn test_rhai_with_params() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter);

        let result = engine.run("params.x + params.y", Some(json!({"x": 10, "y": 5}))).unwrap();
        assert_eq!(result.result, json!(15));
    }

    #[test]
    fn test_rhai_print_captures_logs() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter);

        let result = engine.run(r#"
            print("Hello");
            print("World");
            42
        "#, None).unwrap();

        assert_eq!(result.result, json!(42));
        assert_eq!(result.logs.len(), 2);
        assert_eq!(result.logs[0], "Hello");
        assert_eq!(result.logs[1], "World");
    }

    #[test]
    fn test_rhai_db_insert_and_find() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter);

        let result = engine.run(r#"
            let doc = #{ name: "Alice", age: 30 };
            let id = db_insert_one("users", doc);
            let found = db_find("users", #{});
            found.len()
        "#, None).unwrap();

        assert_eq!(result.result, json!(1));
    }

    #[test]
    fn test_rhai_db_find_one() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter);

        let result = engine.run(r#"
            db_insert_one("users", #{ name: "Bob", age: 25 });
            let user = db_find_one("users", #{ name: "Bob" });
            user.age
        "#, None).unwrap();

        assert_eq!(result.result, json!(25));
    }

    #[test]
    fn test_rhai_db_update() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter);

        let result = engine.run(r#"
            db_insert_one("users", #{ name: "Charlie", age: 20 });
            let update_result = db_update_one("users", #{ name: "Charlie" }, #{ "$set": #{ age: 21 } });
            update_result.modified_count
        "#, None).unwrap();

        assert_eq!(result.result, json!(1));
    }

    #[test]
    fn test_rhai_db_delete() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter);

        let result = engine.run(r#"
            db_insert_one("users", #{ name: "Dave" });
            db_insert_one("users", #{ name: "Eve" });
            let deleted = db_delete_one("users", #{ name: "Dave" });
            deleted
        "#, None).unwrap();

        assert_eq!(result.result, json!(1));
    }

    #[test]
    fn test_rhai_db_count() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter);

        let result = engine.run(r#"
            db_insert_one("items", #{ type: "A" });
            db_insert_one("items", #{ type: "B" });
            db_insert_one("items", #{ type: "A" });
            db_count("items", #{ type: "A" })
        "#, None).unwrap();

        assert_eq!(result.result, json!(2));
    }

    #[test]
    fn test_rhai_syntax_error() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter);

        let result = engine.run("let x = ", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_rhai_operations_limit() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter);

        // This should exceed the operation limit
        let result = engine.run(r#"
            let x = 0;
            loop {
                x += 1;
            }
            x
        "#, None);

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("operation limit") || err_msg.contains("Script error"));
    }
}
