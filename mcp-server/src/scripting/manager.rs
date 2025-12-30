//! Script management - CRUD operations for scripts.
//!
//! Provides operations for managing scripts stored in `_system.scripts`:
//! - Save, get, list, delete scripts
//! - Version history and rollback
//! - Tag management
//! - Dependency resolution
//! - Execution statistics

use crate::adapter::{
    FindOptions as AdapterFindOptions, IronBaseAdapter, SCRIPTS_COLLECTION,
    SCRIPT_VERSIONS_COLLECTION,
};
use crate::error::{McpError, Result};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;

use super::engine::RhaiEngine;
use super::limits::ScriptLimits;
use super::types::{Script, ScriptInfo, ScriptListFilter, ScriptResult, ScriptStats, ScriptVersion};

/// Script Manager - CRUD operations for scripts.
///
/// Manages scripts stored in the `_system.scripts` collection,
/// with support for versioning, tags, and dependencies.
///
/// # Example
///
/// ```rust,ignore
/// let manager = ScriptManager::new(adapter);
///
/// // Save a script
/// let version = manager.save("my_script", "1 + 1", Some("Adds numbers"), None, None)?;
///
/// // Run the script
/// let engine = RhaiEngine::new(adapter);
/// let result = manager.run_script("my_script", None, &engine)?;
/// ```
pub struct ScriptManager {
    adapter: Arc<IronBaseAdapter>,
}

impl ScriptManager {
    /// Create a new ScriptManager.
    ///
    /// # Arguments
    ///
    /// * `adapter` - Database adapter for persistence
    pub fn new(adapter: Arc<IronBaseAdapter>) -> Self {
        Self { adapter }
    }

    // ============================================================
    // CRUD Operations
    // ============================================================

    /// Save a script (insert or update) with versioning.
    ///
    /// If the script exists, the current version is archived to
    /// `_system.script_versions` and the version number is incremented.
    ///
    /// # Arguments
    ///
    /// * `name` - Unique script name
    /// * `code` - Rhai script code
    /// * `description` - Optional description
    /// * `tags` - Optional tags for categorization
    /// * `dependencies` - Optional list of dependency script names
    ///
    /// # Returns
    ///
    /// The new version number
    ///
    /// # Errors
    ///
    /// - `McpError::ScriptError` - Dependency validation failed
    /// - `McpError::ScriptError` - Concurrent modification detected
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

        // Validate dependencies exist and no cycles
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

            // Update with optimistic locking
            let new_ver = existing_script.version + 1;
            let update_result = self.adapter.update_one(
                SCRIPTS_COLLECTION,
                json!({"_id": name, "version": existing_script.version}),
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

            // Check for concurrent modification
            if update_result.matched_count == 0 {
                return Err(McpError::ScriptError(format!(
                    "Script '{}' was modified by another request. Please retry.",
                    name
                )));
            }
            new_ver
        } else {
            // Insert new script
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

    /// Get a script by name (with code).
    ///
    /// # Arguments
    ///
    /// * `name` - Script name
    ///
    /// # Returns
    ///
    /// The script if found, or None
    pub fn get(&self, name: &str) -> Result<Option<Script>> {
        let doc = self
            .adapter
            .find_one(SCRIPTS_COLLECTION, json!({"_id": name}))?;

        match doc {
            Some(doc) => Ok(Some(parse_script(&doc, name))),
            None => Ok(None),
        }
    }

    /// List all scripts (without code) with optional filtering.
    ///
    /// # Arguments
    ///
    /// * `filter` - Optional filter by tags
    ///
    /// # Returns
    ///
    /// Vector of script info (without code for efficiency)
    pub fn list(&self, filter: Option<ScriptListFilter>) -> Result<Vec<ScriptInfo>> {
        // Build query based on filter
        let query = if let Some(f) = filter {
            if let Some(tags) = f.tags {
                if !tags.is_empty() {
                    if f.match_all_tags {
                        json!({"tags": {"$all": tags}})
                    } else {
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
            AdapterFindOptions {
                projection: Some(json!({"code": 0})), // Exclude code
                ..Default::default()
            },
        )?;

        let scripts = docs
            .documents
            .into_iter()
            .filter_map(|doc| parse_script_info(&doc))
            .collect();

        Ok(scripts)
    }

    /// Delete a script by name (also deletes version history).
    ///
    /// # Arguments
    ///
    /// * `name` - Script name
    ///
    /// # Returns
    ///
    /// `true` if the script was deleted, `false` if not found
    ///
    /// # Errors
    ///
    /// - `McpError::ScriptError` - Other scripts depend on this one
    pub fn delete(&self, name: &str) -> Result<bool> {
        // Check if any scripts depend on this one
        let dependents = self.adapter.find(
            SCRIPTS_COLLECTION,
            json!({"dependencies": name}),
            AdapterFindOptions::default(),
        )?;

        if !dependents.documents.is_empty() {
            let dependent_names: Vec<String> = dependents
                .documents
                .iter()
                .filter_map(|d| d.get("_id").and_then(|v| v.as_str()).map(|s| s.to_string()))
                .collect();
            return Err(McpError::ScriptError(format!(
                "Cannot delete '{}': scripts depend on it: {}",
                name,
                dependent_names.join(", ")
            )));
        }

        // Delete version history first
        self.adapter
            .delete_many(SCRIPT_VERSIONS_COLLECTION, json!({"script_name": name}))?;

        // Delete the script
        let count = self
            .adapter
            .delete_one(SCRIPTS_COLLECTION, json!({"_id": name}))?;
        Ok(count > 0)
    }

    // ============================================================
    // Version Management
    // ============================================================

    /// Get version history for a script.
    ///
    /// # Arguments
    ///
    /// * `name` - Script name
    /// * `limit` - Optional limit on number of versions
    ///
    /// # Returns
    ///
    /// Vector of historical versions (newest first)
    pub fn get_history(&self, name: &str, limit: Option<usize>) -> Result<Vec<ScriptVersion>> {
        let mut options = AdapterFindOptions {
            sort: Some(vec![("version".to_string(), -1)]), // Newest first
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
            .filter_map(|doc| parse_script_version(&doc))
            .collect();

        Ok(versions)
    }

    /// Get a specific version of a script.
    ///
    /// Checks both current version and history.
    ///
    /// # Arguments
    ///
    /// * `name` - Script name
    /// * `version` - Version number
    ///
    /// # Returns
    ///
    /// The version if found
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
                    created_at: current
                        .updated_at
                        .unwrap_or_else(|| current.created_at.unwrap_or_default()),
                }));
            }
        }

        // Look in version history
        let doc = self.adapter.find_one(
            SCRIPT_VERSIONS_COLLECTION,
            json!({"script_name": name, "version": version}),
        )?;

        match doc {
            Some(doc) => Ok(parse_script_version(&doc)),
            None => Ok(None),
        }
    }

    /// Rollback to a specific version.
    ///
    /// Creates a new version with the old code (doesn't delete history).
    ///
    /// # Arguments
    ///
    /// * `name` - Script name
    /// * `version` - Version to rollback to
    ///
    /// # Returns
    ///
    /// The new version number
    pub fn rollback(&self, name: &str, version: u32) -> Result<u32> {
        let old_version = self.get_version(name, version)?;

        match old_version {
            Some(v) => self.save(
                name,
                &v.code,
                v.description.as_deref(),
                Some(v.tags),
                Some(v.dependencies),
            ),
            None => Err(McpError::ScriptError(format!(
                "Version {} of script '{}' not found",
                version, name
            ))),
        }
    }

    // ============================================================
    // Tag Management
    // ============================================================

    /// Add tags to a script (no version bump).
    ///
    /// Uses `$addToSet` to ensure uniqueness.
    ///
    /// # Arguments
    ///
    /// * `name` - Script name
    /// * `tags` - Tags to add
    pub fn add_tags(&self, name: &str, tags: Vec<String>) -> Result<()> {
        if self.get(name)?.is_none() {
            return Err(McpError::ScriptError(format!(
                "Script '{}' not found",
                name
            )));
        }

        self.adapter.update_one(
            SCRIPTS_COLLECTION,
            json!({"_id": name}),
            json!({"$addToSet": {"tags": {"$each": tags}}}),
        )?;
        Ok(())
    }

    /// Remove tags from a script (no version bump).
    ///
    /// # Arguments
    ///
    /// * `name` - Script name
    /// * `tags` - Tags to remove
    pub fn remove_tags(&self, name: &str, tags: Vec<String>) -> Result<()> {
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
    // Dependency Management
    // ============================================================

    /// Validate that dependencies exist and there are no cycles.
    ///
    /// # Arguments
    ///
    /// * `script_name` - Name of the script being saved
    /// * `deps` - List of dependency names
    ///
    /// # Errors
    ///
    /// - Self-dependency
    /// - Missing dependency
    /// - Circular dependency
    pub fn validate_dependencies(&self, script_name: &str, deps: &[String]) -> Result<()> {
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

    /// Detect circular dependencies using DFS.
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

    /// Resolve dependencies in topological order.
    ///
    /// Returns a list of script names where dependencies come
    /// before the scripts that depend on them.
    ///
    /// # Arguments
    ///
    /// * `name` - Script name to resolve
    ///
    /// # Returns
    ///
    /// Ordered list of script names (dependencies first, target last)
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

    // ============================================================
    // Execution Statistics
    // ============================================================

    /// Get execution statistics for a script.
    ///
    /// # Arguments
    ///
    /// * `name` - Script name
    ///
    /// # Returns
    ///
    /// Statistics if the script exists
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

    /// Update execution statistics after running a script.
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

    // ============================================================
    // Script Execution
    // ============================================================

    /// Run a stored script by name.
    ///
    /// Resolves dependencies and executes them in order before
    /// the main script.
    ///
    /// # Arguments
    ///
    /// * `name` - Script name
    /// * `params` - Optional parameters accessible as `params` in script
    /// * `engine` - Rhai engine for execution
    ///
    /// # Returns
    ///
    /// Execution result with logs and timing
    pub fn run_script(
        &self,
        name: &str,
        params: Option<Value>,
        engine: &RhaiEngine,
    ) -> Result<ScriptResult> {
        self.run_script_with_limits(name, params, engine, ScriptLimits::default())
    }

    /// Run a stored script with options (legacy API).
    ///
    /// # Arguments
    ///
    /// * `name` - Script name
    /// * `params` - Optional parameters
    /// * `engine` - Rhai engine
    /// * `options` - Optional ScriptOptions (converted to ScriptLimits)
    pub fn run_script_with_options(
        &self,
        name: &str,
        params: Option<Value>,
        engine: &RhaiEngine,
        options: Option<super::types::ScriptOptions>,
    ) -> Result<ScriptResult> {
        let limits = options.map(Into::into).unwrap_or_default();
        self.run_script_with_limits(name, params, engine, limits)
    }

    /// Run a stored script with custom limits.
    ///
    /// # Arguments
    ///
    /// * `name` - Script name
    /// * `params` - Optional parameters
    /// * `engine` - Rhai engine
    /// * `limits` - Resource limits for execution
    pub fn run_script_with_limits(
        &self,
        name: &str,
        params: Option<Value>,
        engine: &RhaiEngine,
        limits: ScriptLimits,
    ) -> Result<ScriptResult> {
        // Get the script
        let script = self
            .get(name)?
            .ok_or_else(|| McpError::ScriptError(format!("Script '{}' not found", name)))?;

        // Resolve dependencies
        let dep_order = self.resolve_dependencies(name)?;

        // Collect dependency code (excluding the main script)
        let dep_codes: Vec<String> = dep_order
            .iter()
            .filter(|dep_name| *dep_name != name)
            .filter_map(|dep_name| self.get(dep_name).ok().flatten().map(|s| s.code))
            .collect();

        // Run the script with dependencies
        let result = engine.run_with_dependencies(&script.code, dep_codes, params, &limits);

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
// Helper Functions
// ============================================================

fn parse_script(doc: &Value, name: &str) -> Script {
    Script {
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
        tags: parse_string_array(doc.get("tags")),
        dependencies: parse_string_array(doc.get("dependencies")),
    }
}

fn parse_script_info(doc: &Value) -> Option<ScriptInfo> {
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
        tags: parse_string_array(doc.get("tags")),
        execution_count: doc
            .get("execution_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        last_run_at: doc
            .get("last_run_at")
            .and_then(|v| v.as_str())
            .map(String::from),
    })
}

fn parse_script_version(doc: &Value) -> Option<ScriptVersion> {
    Some(ScriptVersion {
        script_name: doc.get("script_name")?.as_str()?.to_string(),
        version: doc.get("version").and_then(|v| v.as_u64()).unwrap_or(1) as u32,
        code: doc.get("code")?.as_str()?.to_string(),
        description: doc
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from),
        tags: parse_string_array(doc.get("tags")),
        dependencies: parse_string_array(doc.get("dependencies")),
        created_at: doc
            .get("created_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

fn parse_string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}
