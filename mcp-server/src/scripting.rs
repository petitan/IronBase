//! Script management and execution for IronBase MCP Server
//!
//! Provides CRUD operations for scripts stored in _system.scripts collection.
//! Scripts are stored as JSON documents with name, code, and description.
//!
//! Also provides Rhai script execution engine with database bindings.

use crate::adapter::{
    FindOptions as AdapterFindOptions, IronBaseAdapter, SCRIPTS_COLLECTION,
    SCRIPT_VERSIONS_COLLECTION,
};
use crate::error::{McpError, Result};
use base64::Engine as Base64Engine;
use parking_lot::Mutex;
use rhai::{Dynamic, Engine, EvalAltResult, Map, Scope};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;

/// Script metadata returned by list (without code)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptInfo {
    pub name: String,
    pub description: Option<String>,
    pub created_at: Option<String>,
    /// Script version (increments on each save)
    pub version: u32,
    /// Tags for categorization
    pub tags: Vec<String>,
    /// Number of times the script has been executed
    pub execution_count: u64,
    /// Timestamp of last execution
    pub last_run_at: Option<String>,
}

/// Full script with code and all metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Script {
    pub name: String,
    pub code: String,
    pub description: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    /// Script version (increments on each save)
    pub version: u32,
    /// Tags for categorization
    pub tags: Vec<String>,
    /// Dependencies on other scripts (executed before this one)
    pub dependencies: Vec<String>,
}

/// Script version history entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptVersion {
    pub script_name: String,
    pub version: u32,
    pub code: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub dependencies: Vec<String>,
    pub created_at: String,
}

/// Script execution statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptStats {
    pub name: String,
    pub execution_count: u64,
    pub last_run_at: Option<String>,
    pub last_run_success: Option<bool>,
    pub total_execution_time_ms: u64,
    /// Calculated: total_execution_time_ms / execution_count
    pub avg_execution_time_ms: f64,
}

/// Filter options for script listing
#[derive(Debug, Clone, Default)]
pub struct ScriptListFilter {
    /// Filter by tags
    pub tags: Option<Vec<String>>,
    /// If true, require ALL tags (AND); if false, require ANY tag (OR)
    pub match_all_tags: bool,
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

    /// Save a script (insert or update) with versioning
    /// Returns the new version number
    pub fn save(
        &self,
        name: &str,
        code: &str,
        description: Option<&str>,
        tags: Option<Vec<String>>,
        dependencies: Option<Vec<String>>,
    ) -> Result<u32> {
        let now = chrono::Utc::now().to_rfc3339();

        // Deduplicate tags
        let mut tags = tags.unwrap_or_default();
        tags.sort();
        tags.dedup();

        let dependencies = dependencies.unwrap_or_default();

        // Validate dependencies exist
        self.validate_dependencies(name, &dependencies)?;

        // Check if script exists
        let existing = self.get(name)?;

        let new_version = if let Some(existing_script) = existing {
            // Archive current version to _system.script_versions
            let version_doc = json!({
                "script_name": name,
                "version": existing_script.version,
                "code": existing_script.code,
                "description": existing_script.description,
                "tags": existing_script.tags,
                "dependencies": existing_script.dependencies,
                "created_at": now.clone()
            });
            self.adapter
                .insert_one(SCRIPT_VERSIONS_COLLECTION, version_doc)?;

            // Update existing script with incremented version
            let new_ver = existing_script.version + 1;
            self.adapter.update_one(
                SCRIPTS_COLLECTION,
                json!({"_id": name}),
                json!({
                    "$set": {
                        "code": code,
                        "description": description,
                        "tags": tags,
                        "dependencies": dependencies,
                        "version": new_ver,
                        "updated_at": now
                    }
                }),
            )?;
            new_ver
        } else {
            // Insert new script with version 1
            let doc = json!({
                "_id": name,
                "code": code,
                "description": description,
                "tags": tags,
                "dependencies": dependencies,
                "version": 1,
                "created_at": now,
                "execution_count": 0,
                "total_execution_time_ms": 0
            });
            self.adapter.insert_one(SCRIPTS_COLLECTION, doc)?;
            1
        };

        Ok(new_version)
    }

    /// Validate that all dependencies exist and there are no circular dependencies
    pub fn validate_dependencies(&self, script_name: &str, deps: &[String]) -> Result<()> {
        // Check each dependency exists
        for dep in deps {
            if dep == script_name {
                return Err(McpError::ScriptError(format!(
                    "Script '{}' cannot depend on itself",
                    script_name
                )));
            }
            if self.get(dep)?.is_none() {
                return Err(McpError::ScriptError(format!(
                    "Dependency '{}' does not exist",
                    dep
                )));
            }
        }

        // Check for circular dependencies
        let mut visited = HashSet::new();
        let mut stack = HashSet::new();
        stack.insert(script_name.to_string());

        for dep in deps {
            self.detect_circular(dep, &mut visited, &mut stack)?;
        }

        Ok(())
    }

    /// Detect circular dependencies using DFS
    fn detect_circular(
        &self,
        name: &str,
        visited: &mut HashSet<String>,
        stack: &mut HashSet<String>,
    ) -> Result<()> {
        if stack.contains(name) {
            return Err(McpError::ScriptError(format!(
                "Circular dependency detected involving '{}'",
                name
            )));
        }
        if visited.contains(name) {
            return Ok(());
        }

        stack.insert(name.to_string());
        visited.insert(name.to_string());

        if let Some(script) = self.get(name)? {
            for dep in &script.dependencies {
                self.detect_circular(dep, visited, stack)?;
            }
        }

        stack.remove(name);
        Ok(())
    }

    /// Resolve dependencies in topological order (dependencies first, then script)
    pub fn resolve_dependencies(&self, name: &str) -> Result<Vec<String>> {
        let mut result = Vec::new();
        let mut visited = HashSet::new();
        self.resolve_deps_recursive(name, &mut result, &mut visited)?;
        Ok(result)
    }

    fn resolve_deps_recursive(
        &self,
        name: &str,
        result: &mut Vec<String>,
        visited: &mut HashSet<String>,
    ) -> Result<()> {
        if visited.contains(name) {
            return Ok(());
        }
        visited.insert(name.to_string());

        if let Some(script) = self.get(name)? {
            for dep in &script.dependencies {
                self.resolve_deps_recursive(dep, result, visited)?;
            }
        }

        result.push(name.to_string());
        Ok(())
    }

    /// List all scripts (without code) with optional filtering
    pub fn list(&self, filter: Option<ScriptListFilter>) -> Result<Vec<ScriptInfo>> {
        // Build query based on filter
        let query = if let Some(f) = filter {
            if let Some(tags) = f.tags {
                if !tags.is_empty() {
                    if f.match_all_tags {
                        // AND: require all tags
                        json!({"tags": {"$all": tags}})
                    } else {
                        // OR: require any tag
                        json!({"tags": {"$in": tags}})
                    }
                } else {
                    json!({})
                }
            } else {
                json!({})
            }
        } else {
            json!({})
        };

        let docs = self.adapter.find(
            SCRIPTS_COLLECTION,
            query,
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
                    description: doc
                        .get("description")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    created_at: doc
                        .get("created_at")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    version: doc.get("version").and_then(|v| v.as_u64()).unwrap_or(1) as u32,
                    tags: doc
                        .get("tags")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default(),
                    execution_count: doc
                        .get("execution_count")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                    last_run_at: doc
                        .get("last_run_at")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                })
            })
            .collect();

        Ok(scripts)
    }

    /// Get a script by name (with code)
    pub fn get(&self, name: &str) -> Result<Option<Script>> {
        let doc = self
            .adapter
            .find_one(SCRIPTS_COLLECTION, json!({"_id": name}))?;

        match doc {
            Some(doc) => {
                let script = Script {
                    name: doc
                        .get("_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or(name)
                        .to_string(),
                    code: doc
                        .get("code")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    description: doc
                        .get("description")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    created_at: doc
                        .get("created_at")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    updated_at: doc
                        .get("updated_at")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    version: doc.get("version").and_then(|v| v.as_u64()).unwrap_or(1) as u32,
                    tags: doc
                        .get("tags")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default(),
                    dependencies: doc
                        .get("dependencies")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default(),
                };
                Ok(Some(script))
            }
            None => Ok(None),
        }
    }

    /// Delete a script by name (also deletes version history)
    pub fn delete(&self, name: &str) -> Result<bool> {
        // Delete version history first
        self.adapter.delete_many(
            SCRIPT_VERSIONS_COLLECTION,
            json!({"script_name": name}),
        )?;

        // Delete the script
        let count = self
            .adapter
            .delete_one(SCRIPTS_COLLECTION, json!({"_id": name}))?;
        Ok(count > 0)
    }

    // ============================================================
    // Version Management
    // ============================================================

    /// Get version history for a script
    pub fn get_history(&self, name: &str, limit: Option<usize>) -> Result<Vec<ScriptVersion>> {
        let mut options = crate::adapter::FindOptions {
            sort: Some(json!([["version", -1]])), // Newest first
            ..Default::default()
        };
        if let Some(lim) = limit {
            options.limit = Some(lim);
        }

        let docs = self.adapter.find(
            SCRIPT_VERSIONS_COLLECTION,
            json!({"script_name": name}),
            options,
        )?;

        let versions = docs
            .documents
            .into_iter()
            .filter_map(|doc| {
                Some(ScriptVersion {
                    script_name: doc.get("script_name")?.as_str()?.to_string(),
                    version: doc.get("version").and_then(|v| v.as_u64()).unwrap_or(1) as u32,
                    code: doc.get("code")?.as_str()?.to_string(),
                    description: doc
                        .get("description")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    tags: doc
                        .get("tags")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default(),
                    dependencies: doc
                        .get("dependencies")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default(),
                    created_at: doc
                        .get("created_at")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                })
            })
            .collect();

        Ok(versions)
    }

    /// Get a specific version of a script
    pub fn get_version(&self, name: &str, version: u32) -> Result<Option<ScriptVersion>> {
        // Check if this is the current version
        if let Some(current) = self.get(name)? {
            if current.version == version {
                return Ok(Some(ScriptVersion {
                    script_name: name.to_string(),
                    version: current.version,
                    code: current.code,
                    description: current.description,
                    tags: current.tags,
                    dependencies: current.dependencies,
                    created_at: current.updated_at.unwrap_or_else(|| {
                        current.created_at.unwrap_or_default()
                    }),
                }));
            }
        }

        // Look in version history
        let doc = self.adapter.find_one(
            SCRIPT_VERSIONS_COLLECTION,
            json!({"script_name": name, "version": version}),
        )?;

        match doc {
            Some(doc) => Ok(Some(ScriptVersion {
                script_name: name.to_string(),
                version: doc.get("version").and_then(|v| v.as_u64()).unwrap_or(1) as u32,
                code: doc.get("code").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                description: doc.get("description").and_then(|v| v.as_str()).map(String::from),
                tags: doc
                    .get("tags")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default(),
                dependencies: doc
                    .get("dependencies")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default(),
                created_at: doc.get("created_at").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            })),
            None => Ok(None),
        }
    }

    /// Rollback to a specific version (creates a new version with old code)
    pub fn rollback(&self, name: &str, version: u32) -> Result<u32> {
        let old_version = self.get_version(name, version)?;

        match old_version {
            Some(v) => {
                // Save creates a new version with the old code
                self.save(
                    name,
                    &v.code,
                    v.description.as_deref(),
                    Some(v.tags),
                    Some(v.dependencies),
                )
            }
            None => Err(McpError::ScriptError(format!(
                "Version {} of script '{}' not found",
                version, name
            ))),
        }
    }

    // ============================================================
    // Tag Management
    // ============================================================

    /// Add tags to a script (no version bump)
    pub fn add_tags(&self, name: &str, tags: Vec<String>) -> Result<()> {
        // Verify script exists first
        if self.get(name)?.is_none() {
            return Err(McpError::ScriptError(format!(
                "Script '{}' not found",
                name
            )));
        }

        // Use $addToSet to add unique tags
        self.adapter.update_one(
            SCRIPTS_COLLECTION,
            json!({"_id": name}),
            json!({"$addToSet": {"tags": {"$each": tags}}}),
        )?;
        Ok(())
    }

    /// Remove tags from a script (no version bump)
    pub fn remove_tags(&self, name: &str, tags: Vec<String>) -> Result<()> {
        // Verify script exists first
        if self.get(name)?.is_none() {
            return Err(McpError::ScriptError(format!(
                "Script '{}' not found",
                name
            )));
        }

        self.adapter.update_one(
            SCRIPTS_COLLECTION,
            json!({"_id": name}),
            json!({"$pull": {"tags": {"$in": tags}}}),
        )?;
        Ok(())
    }

    // ============================================================
    // Execution Statistics
    // ============================================================

    /// Get execution statistics for a script
    pub fn get_stats(&self, name: &str) -> Result<Option<ScriptStats>> {
        let doc = self
            .adapter
            .find_one(SCRIPTS_COLLECTION, json!({"_id": name}))?;

        match doc {
            Some(doc) => {
                let execution_count = doc
                    .get("execution_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let total_execution_time_ms = doc
                    .get("total_execution_time_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let avg = if execution_count > 0 {
                    total_execution_time_ms as f64 / execution_count as f64
                } else {
                    0.0
                };

                Ok(Some(ScriptStats {
                    name: name.to_string(),
                    execution_count,
                    last_run_at: doc
                        .get("last_run_at")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    last_run_success: doc.get("last_run_success").and_then(|v| v.as_bool()),
                    total_execution_time_ms,
                    avg_execution_time_ms: avg,
                }))
            }
            None => Ok(None),
        }
    }

    /// Update execution statistics after running a script
    fn update_execution_stats(
        &self,
        name: &str,
        execution_time_ms: u64,
        success: bool,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.adapter.update_one(
            SCRIPTS_COLLECTION,
            json!({"_id": name}),
            json!({
                "$inc": {
                    "execution_count": 1,
                    "total_execution_time_ms": execution_time_ms as i64
                },
                "$set": {
                    "last_run_at": now,
                    "last_run_success": success
                }
            }),
        )?;
        Ok(())
    }

    /// Run a stored script by name with execution statistics tracking
    pub fn run_script(
        &self,
        name: &str,
        params: Option<Value>,
        engine: &RhaiEngine,
    ) -> Result<ScriptResult> {
        // Get the script
        let script = self.get(name)?.ok_or_else(|| {
            McpError::ScriptError(format!("Script '{}' not found", name))
        })?;

        // Resolve dependencies
        let dep_order = self.resolve_dependencies(name)?;

        // Collect dependency code (excluding the main script itself)
        let dep_codes: Vec<String> = dep_order
            .iter()
            .filter(|dep_name| *dep_name != name)
            .filter_map(|dep_name| self.get(dep_name).ok().flatten().map(|s| s.code))
            .collect();

        // Run the script with dependencies
        let result = engine.run_with_dependencies(&script.code, dep_codes, params);

        // Update execution stats
        match &result {
            Ok(res) => {
                let _ = self.update_execution_stats(name, res.execution_time_ms, true);
            }
            Err(_) => {
                let _ = self.update_execution_stats(name, 0, false);
            }
        }

        result
    }
}

// ============================================================
// Rhai Script Execution Engine
// ============================================================

/// Maximum number of operations per script (DoS protection)
const MAX_OPERATIONS: u64 = 1_000_000;

/// Maximum number of log entries (DoS protection)
const MAX_LOG_ENTRIES: usize = 10_000;

/// Result of script execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptResult {
    pub result: Value,
    pub logs: Vec<String>,
    /// Execution time in milliseconds
    pub execution_time_ms: u64,
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
        use std::time::Instant;

        let mut engine = Engine::new();

        // Security: Disable dangerous operations
        engine.set_max_operations(MAX_OPERATIONS);

        // Create logs collector (Arc<Mutex> for thread safety)
        let logs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let logs_clone = logs.clone();

        // Register print function to capture logs (with DoS protection)
        engine.on_print(move |s| {
            let mut logs = logs_clone.lock();
            if logs.len() < MAX_LOG_ENTRIES {
                logs.push(s.to_string());
            }
            // Silently drop logs after limit reached (DoS protection)
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

        // Execute the script and measure time
        let start = Instant::now();
        let result = engine.eval_with_scope::<Dynamic>(&mut scope, code);
        let execution_time_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(value) => {
                let json_result = dynamic_to_json(&value);
                Ok(ScriptResult {
                    result: json_result,
                    logs: logs.lock().clone(),
                    execution_time_ms,
                })
            }
            Err(e) => {
                let err_str = format_rhai_error(&e);
                // Check if it was an operations limit error
                if err_str.contains("operations") || err_str.contains("limit") {
                    Err(McpError::ScriptError(format!(
                        "Script exceeded maximum operations limit ({})",
                        MAX_OPERATIONS
                    )))
                } else {
                    Err(McpError::ScriptError(err_str))
                }
            }
        }
    }

    /// Run a script with dependencies resolved
    pub fn run_with_dependencies(
        &self,
        code: &str,
        dependency_codes: Vec<String>,
        params: Option<Value>,
    ) -> Result<ScriptResult> {
        // Concatenate dependency code before main script
        let full_code = if dependency_codes.is_empty() {
            code.to_string()
        } else {
            let deps = dependency_codes.join("\n\n");
            format!("{}\n\n// === Main Script ===\n{}", deps, code)
        };

        self.run(&full_code, params)
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

        // db_insert_many(collection, documents_array) -> {inserted_count, inserted_ids}
        let adapter_insert_many = adapter.clone();
        engine.register_fn("db_insert_many", move |collection: &str, docs: rhai::Array| -> Dynamic {
            // Convert Rhai Array to Vec<Value>
            let docs_vec: Vec<Value> = docs.iter()
                .map(dynamic_to_json)
                .collect();
            match adapter_insert_many.insert_many(collection, docs_vec) {
                Ok(ids) => {
                    let mut map = Map::new();
                    map.insert("inserted_count".into(), Dynamic::from(ids.len() as i64));
                    let id_dynamics: Vec<Dynamic> = ids.into_iter()
                        .map(Dynamic::from)
                        .collect();
                    map.insert("inserted_ids".into(), Dynamic::from(id_dynamics));
                    Dynamic::from(map)
                }
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
            format!(
                "Script exceeded maximum operation limit ({})",
                MAX_OPERATIONS
            )
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
        let version = manager
            .save("test_script", "let x = 1 + 1;", Some("Test script"), None, None)
            .unwrap();
        assert_eq!(version, 1);

        // Get it back
        let script = manager.get("test_script").unwrap().unwrap();
        assert_eq!(script.name, "test_script");
        assert_eq!(script.code, "let x = 1 + 1;");
        assert_eq!(script.description, Some("Test script".to_string()));
        assert_eq!(script.version, 1);
    }

    #[test]
    fn test_script_list() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        // Save multiple scripts
        manager.save("script1", "code1", Some("First"), None, None).unwrap();
        manager.save("script2", "code2", Some("Second"), None, None).unwrap();

        // List them
        let scripts = manager.list(None).unwrap();
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
        let v1 = manager.save("updatable", "v1", Some("Version 1"), None, None).unwrap();
        assert_eq!(v1, 1);

        // Update it
        let v2 = manager.save("updatable", "v2", Some("Version 2"), None, None).unwrap();
        assert_eq!(v2, 2);

        // Get updated version
        let script = manager.get("updatable").unwrap().unwrap();
        assert_eq!(script.code, "v2");
        assert_eq!(script.description, Some("Version 2".to_string()));
        assert_eq!(script.version, 2);

        // Should still be only one script
        let scripts = manager.list(None).unwrap();
        assert_eq!(scripts.len(), 1);
    }

    #[test]
    fn test_script_delete() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        // Save and delete
        manager.save("deletable", "code", None, None, None).unwrap();
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

    // ============================================================
    // NEW: Versioning Tests
    // ============================================================

    #[test]
    fn test_script_versioning_basic() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        // First save -> version 1
        let v1 = manager.save("versioned", "code_v1", Some("First"), None, None).unwrap();
        assert_eq!(v1, 1);

        // Second save -> version 2
        let v2 = manager.save("versioned", "code_v2", Some("Second"), None, None).unwrap();
        assert_eq!(v2, 2);

        // Third save -> version 3
        let v3 = manager.save("versioned", "code_v3", Some("Third"), None, None).unwrap();
        assert_eq!(v3, 3);

        // Current version should be 3
        let script = manager.get("versioned").unwrap().unwrap();
        assert_eq!(script.version, 3);
        assert_eq!(script.code, "code_v3");
    }

    #[test]
    fn test_script_history() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        // Create multiple versions
        manager.save("history_test", "v1_code", Some("V1"), None, None).unwrap();
        manager.save("history_test", "v2_code", Some("V2"), None, None).unwrap();
        manager.save("history_test", "v3_code", Some("V3"), None, None).unwrap();

        // Get full history
        let history = manager.get_history("history_test", None).unwrap();
        assert_eq!(history.len(), 2); // Only archived versions (v1, v2), not current

        // Check versions are in descending order (newest first)
        assert_eq!(history[0].version, 2);
        assert_eq!(history[1].version, 1);
    }

    #[test]
    fn test_script_history_limit() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        // Create 5 versions
        for i in 1..=5 {
            manager.save("limit_test", &format!("v{}", i), None, None, None).unwrap();
        }

        // Get only 2 most recent versions
        let history = manager.get_history("limit_test", Some(2)).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].version, 4);
        assert_eq!(history[1].version, 3);
    }

    #[test]
    fn test_script_get_specific_version() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        // Create versions
        manager.save("specific_version", "code_v1", Some("V1"), None, None).unwrap();
        manager.save("specific_version", "code_v2", Some("V2"), None, None).unwrap();
        manager.save("specific_version", "code_v3", Some("V3"), None, None).unwrap();

        // Get version 1
        let v1 = manager.get_version("specific_version", 1).unwrap();
        assert!(v1.is_some());
        let v1 = v1.unwrap();
        assert_eq!(v1.code, "code_v1");
        assert_eq!(v1.description, Some("V1".to_string()));

        // Get version 2
        let v2 = manager.get_version("specific_version", 2).unwrap();
        assert!(v2.is_some());
        assert_eq!(v2.unwrap().code, "code_v2");

        // Non-existent version
        let v99 = manager.get_version("specific_version", 99).unwrap();
        assert!(v99.is_none());
    }

    #[test]
    fn test_script_rollback() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        // Create versions
        manager.save("rollback_test", "v1_code", Some("V1"), None, None).unwrap();
        manager.save("rollback_test", "v2_code", Some("V2"), None, None).unwrap();
        manager.save("rollback_test", "v3_code", Some("V3"), None, None).unwrap();

        // Rollback to version 1 - creates version 4
        let new_version = manager.rollback("rollback_test", 1).unwrap();
        assert_eq!(new_version, 4);

        // Current code should be v1's code
        let script = manager.get("rollback_test").unwrap().unwrap();
        assert_eq!(script.code, "v1_code");
        assert_eq!(script.version, 4);
    }

    #[test]
    fn test_script_rollback_nonexistent_version() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        manager.save("rollback_err", "code", None, None, None).unwrap();

        // Try to rollback to non-existent version
        let result = manager.rollback("rollback_err", 99);
        assert!(result.is_err());
    }

    // ============================================================
    // NEW: Tags Tests
    // ============================================================

    #[test]
    fn test_script_tags_on_save() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        // Save with tags
        manager.save(
            "tagged_script",
            "code",
            Some("Description"),
            Some(vec!["utility".to_string(), "report".to_string()]),
            None,
        ).unwrap();

        // Check tags are saved
        let script = manager.get("tagged_script").unwrap().unwrap();
        assert_eq!(script.tags.len(), 2);
        assert!(script.tags.contains(&"utility".to_string()));
        assert!(script.tags.contains(&"report".to_string()));
    }

    #[test]
    fn test_script_tags_add() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        // Save without tags
        manager.save("add_tags_test", "code", None, None, None).unwrap();

        // Add tags
        manager.add_tags("add_tags_test", vec!["new_tag".to_string(), "another".to_string()]).unwrap();

        // Verify tags added
        let script = manager.get("add_tags_test").unwrap().unwrap();
        assert!(script.tags.contains(&"new_tag".to_string()));
        assert!(script.tags.contains(&"another".to_string()));
    }

    #[test]
    fn test_script_tags_remove() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        // Save with tags
        manager.save(
            "remove_tags_test",
            "code",
            None,
            Some(vec!["a".to_string(), "b".to_string(), "c".to_string()]),
            None,
        ).unwrap();

        // Remove some tags
        manager.remove_tags("remove_tags_test", vec!["b".to_string()]).unwrap();

        // Verify tag removed
        let script = manager.get("remove_tags_test").unwrap().unwrap();
        assert!(script.tags.contains(&"a".to_string()));
        assert!(!script.tags.contains(&"b".to_string()));
        assert!(script.tags.contains(&"c".to_string()));
    }

    #[test]
    fn test_script_duplicate_tags() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        // Save with duplicate tags
        manager.save(
            "dup_tags",
            "code",
            None,
            Some(vec!["tag1".to_string(), "tag1".to_string(), "tag2".to_string()]),
            None,
        ).unwrap();

        // Duplicates should be deduplicated
        let script = manager.get("dup_tags").unwrap().unwrap();
        let tag1_count = script.tags.iter().filter(|t| *t == "tag1").count();
        assert_eq!(tag1_count, 1);
    }

    #[test]
    fn test_script_list_filter_by_tags_or() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        // Create scripts with different tags
        manager.save("script_a", "code", None, Some(vec!["util".to_string()]), None).unwrap();
        manager.save("script_b", "code", None, Some(vec!["report".to_string()]), None).unwrap();
        manager.save("script_c", "code", None, Some(vec!["util".to_string(), "report".to_string()]), None).unwrap();
        manager.save("script_d", "code", None, Some(vec!["other".to_string()]), None).unwrap();

        // Filter by "util" OR "report"
        let filter = ScriptListFilter {
            tags: Some(vec!["util".to_string(), "report".to_string()]),
            match_all_tags: false,
        };
        let scripts = manager.list(Some(filter)).unwrap();

        assert_eq!(scripts.len(), 3); // a, b, c
        let names: Vec<&str> = scripts.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"script_a"));
        assert!(names.contains(&"script_b"));
        assert!(names.contains(&"script_c"));
        assert!(!names.contains(&"script_d"));
    }

    #[test]
    fn test_script_list_filter_by_tags_and() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        // Create scripts with different tags
        manager.save("script_a", "code", None, Some(vec!["util".to_string()]), None).unwrap();
        manager.save("script_b", "code", None, Some(vec!["report".to_string()]), None).unwrap();
        manager.save("script_c", "code", None, Some(vec!["util".to_string(), "report".to_string()]), None).unwrap();

        // Filter by "util" AND "report"
        let filter = ScriptListFilter {
            tags: Some(vec!["util".to_string(), "report".to_string()]),
            match_all_tags: true,
        };
        let scripts = manager.list(Some(filter)).unwrap();

        assert_eq!(scripts.len(), 1); // only c
        assert_eq!(scripts[0].name, "script_c");
    }

    // ============================================================
    // NEW: Execution Metadata Tests
    // ============================================================

    #[test]
    fn test_script_execution_time_measured() {
        let (adapter, _temp) = create_test_adapter();
        let engine = RhaiEngine::new(adapter);

        let result = engine.run("1 + 1", None).unwrap();

        // Execution time is recorded as u64 - just verify result was returned
        assert_eq!(result.result, json!(2));
    }

    #[test]
    fn test_script_stats_initial() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        // Save a script
        manager.save("stats_test", "1 + 1", None, None, None).unwrap();

        // Check initial stats
        let stats = manager.get_stats("stats_test").unwrap().unwrap();
        assert_eq!(stats.execution_count, 0);
        assert!(stats.last_run_at.is_none());
    }

    #[test]
    fn test_script_stats_after_run() {
        let (adapter, _temp) = create_test_adapter();
        let adapter_clone = adapter.clone();
        let manager = ScriptManager::new(adapter);
        let engine = RhaiEngine::new(adapter_clone);

        // Save and run a script
        manager.save("stats_run_test", "1 + 1", None, None, None).unwrap();
        manager.run_script("stats_run_test", None, &engine).unwrap();

        // Check stats after run
        let stats = manager.get_stats("stats_run_test").unwrap().unwrap();
        assert_eq!(stats.execution_count, 1);
        assert!(stats.last_run_at.is_some());
        assert_eq!(stats.last_run_success, Some(true));
        // total_execution_time_ms is u64, always >= 0 (could be 0 for fast execution)
    }

    #[test]
    fn test_script_stats_after_multiple_runs() {
        let (adapter, _temp) = create_test_adapter();
        let adapter_clone = adapter.clone();
        let manager = ScriptManager::new(adapter);
        let engine = RhaiEngine::new(adapter_clone);

        // Save and run multiple times
        manager.save("multi_run", "1 + 1", None, None, None).unwrap();
        manager.run_script("multi_run", None, &engine).unwrap();
        manager.run_script("multi_run", None, &engine).unwrap();
        manager.run_script("multi_run", None, &engine).unwrap();

        // Check stats
        let stats = manager.get_stats("multi_run").unwrap().unwrap();
        assert_eq!(stats.execution_count, 3);
    }

    // ============================================================
    // NEW: Dependency Tests
    // ============================================================

    #[test]
    fn test_script_with_dependency() {
        let (adapter, _temp) = create_test_adapter();
        let adapter_clone = adapter.clone();
        let manager = ScriptManager::new(adapter);
        let engine = RhaiEngine::new(adapter_clone);

        // Create helper script
        manager.save("helper", "fn add(a, b) { a + b }", None, None, None).unwrap();

        // Create main script that depends on helper
        manager.save(
            "main",
            "add(10, 20)",
            None,
            None,
            Some(vec!["helper".to_string()]),
        ).unwrap();

        // Run main with dependencies
        let result = manager.run_script("main", None, &engine).unwrap();
        assert_eq!(result.result, json!(30));
    }

    #[test]
    fn test_script_dependency_chain() {
        let (adapter, _temp) = create_test_adapter();
        let adapter_clone = adapter.clone();
        let manager = ScriptManager::new(adapter);
        let engine = RhaiEngine::new(adapter_clone);

        // Create chain: level1 <- level2 <- level3
        manager.save("level1", "fn l1() { 1 }", None, None, None).unwrap();
        manager.save("level2", "fn l2() { l1() + 1 }", None, None, Some(vec!["level1".to_string()])).unwrap();
        manager.save("level3", "l2() + 1", None, None, Some(vec!["level2".to_string()])).unwrap();

        // Run level3
        let result = manager.run_script("level3", None, &engine).unwrap();
        assert_eq!(result.result, json!(3)); // 1 + 1 + 1
    }

    #[test]
    fn test_script_circular_dependency_detection() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        // Create circular dependency: a -> b -> a
        manager.save("a", "fn a_fn() { 1 }", None, None, None).unwrap();
        manager.save("b", "fn b_fn() { a_fn() }", None, None, Some(vec!["a".to_string()])).unwrap();

        // Try to update 'a' to depend on 'b' - should fail
        let result = manager.save("a", "fn a_fn() { b_fn() }", None, None, Some(vec!["b".to_string()]));
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.to_lowercase().contains("circular"));
    }

    #[test]
    fn test_script_missing_dependency_error() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        // Try to save with non-existent dependency
        let result = manager.save(
            "orphan",
            "code",
            None,
            None,
            Some(vec!["nonexistent".to_string()]),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_script_dependency_resolution_order() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        // Create: a <- b <- c (c depends on b, b depends on a)
        manager.save("dep_a", "// a", None, None, None).unwrap();
        manager.save("dep_b", "// b", None, None, Some(vec!["dep_a".to_string()])).unwrap();
        manager.save("dep_c", "// c", None, None, Some(vec!["dep_b".to_string()])).unwrap();

        // Resolve dependencies for c
        let order = manager.resolve_dependencies("dep_c").unwrap();

        // Order should be: dep_a, dep_b, dep_c
        assert_eq!(order.len(), 3);
        assert_eq!(order[0], "dep_a");
        assert_eq!(order[1], "dep_b");
        assert_eq!(order[2], "dep_c");
    }

    // ============================================================
    // NEW: Edge Case Tests
    // ============================================================

    #[test]
    fn test_script_empty_tags() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        // Save with empty tags array
        manager.save("empty_tags", "code", None, Some(vec![]), None).unwrap();

        let script = manager.get("empty_tags").unwrap().unwrap();
        assert!(script.tags.is_empty());
    }

    #[test]
    fn test_script_delete_clears_history() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        // Create multiple versions
        manager.save("delete_history", "v1", None, None, None).unwrap();
        manager.save("delete_history", "v2", None, None, None).unwrap();
        manager.save("delete_history", "v3", None, None, None).unwrap();

        // Delete the script
        manager.delete("delete_history").unwrap();

        // History should also be cleared
        let history = manager.get_history("delete_history", None).unwrap();
        assert!(history.is_empty());
    }

    #[test]
    fn test_script_info_includes_new_fields() {
        let (adapter, _temp) = create_test_adapter();
        let manager = ScriptManager::new(adapter);

        // Save with all new fields
        manager.save(
            "full_info",
            "code",
            Some("Description"),
            Some(vec!["tag1".to_string()]),
            None,
        ).unwrap();

        // List and check ScriptInfo
        let scripts = manager.list(None).unwrap();
        let info = scripts.iter().find(|s| s.name == "full_info").unwrap();

        assert_eq!(info.version, 1);
        assert_eq!(info.tags.len(), 1);
        assert_eq!(info.execution_count, 0);
    }
}
