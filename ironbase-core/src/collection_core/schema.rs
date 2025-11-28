use std::collections::HashMap;

use regex::Regex;
use serde_json::Value;

use crate::error::{MongoLiteError, Result};

/// Compiled property schema with extended validation constraints
#[derive(Clone, Debug)]
pub struct PropertySchema {
    pub schema_type: SchemaType,
    pub enum_values: Option<Vec<Value>>, // enum validation
    pub pattern: Option<Regex>,          // regex pattern validation
    pub min_items: Option<usize>,        // array minimum length
    pub max_items: Option<usize>,        // array maximum length
}

impl PropertySchema {
    pub fn new(schema_type: SchemaType) -> Self {
        Self {
            schema_type,
            enum_values: None,
            pattern: None,
            min_items: None,
            max_items: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CompiledSchema {
    pub(super) required: Vec<String>,
    pub(super) properties: HashMap<String, PropertySchema>,
}

#[derive(Clone, Copy, Debug)]
pub enum SchemaType {
    String,
    Number,
    Boolean,
    Object,
    Array,
}

impl SchemaType {
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "string" => Some(Self::String),
            "number" | "integer" => Some(Self::Number),
            "boolean" => Some(Self::Boolean),
            "object" => Some(Self::Object),
            "array" => Some(Self::Array),
            _ => None,
        }
    }

    pub fn matches(&self, value: &Value) -> bool {
        match self {
            SchemaType::String => value.is_string(),
            SchemaType::Number => value.is_number(),
            SchemaType::Boolean => value.is_boolean(),
            SchemaType::Object => value.is_object(),
            SchemaType::Array => value.is_array(),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            SchemaType::String => "string",
            SchemaType::Number => "number",
            SchemaType::Boolean => "boolean",
            SchemaType::Object => "object",
            SchemaType::Array => "array",
        }
    }
}

impl CompiledSchema {
    pub fn from_value(schema: &Value) -> Result<Self> {
        let obj = schema.as_object().ok_or_else(|| {
            MongoLiteError::SchemaError("Schema must be a JSON object".to_string())
        })?;

        if let Some(schema_type) = obj.get("type") {
            let type_str = schema_type.as_str().ok_or_else(|| {
                MongoLiteError::SchemaError("Schema type must be a string".to_string())
            })?;
            if type_str != "object" {
                return Err(MongoLiteError::SchemaError(
                    "Only object schemas are supported".to_string(),
                ));
            }
        }

        let mut required = Vec::new();
        if let Some(required_value) = obj.get("required") {
            let arr = required_value.as_array().ok_or_else(|| {
                MongoLiteError::SchemaError("required must be an array of field names".to_string())
            })?;
            for entry in arr {
                let field = entry.as_str().ok_or_else(|| {
                    MongoLiteError::SchemaError("required entries must be strings".to_string())
                })?;
                required.push(field.to_string());
            }
        }

        let mut properties = HashMap::new();
        if let Some(props) = obj.get("properties") {
            let props_obj = props.as_object().ok_or_else(|| {
                MongoLiteError::SchemaError("properties must be an object".to_string())
            })?;
            for (field, spec) in props_obj {
                if let Some(type_value) = spec.get("type") {
                    let type_str = type_value.as_str().ok_or_else(|| {
                        MongoLiteError::SchemaError(format!(
                            "Property '{}' type must be a string",
                            field
                        ))
                    })?;
                    let parsed_type = SchemaType::from_str(type_str).ok_or_else(|| {
                        MongoLiteError::SchemaError(format!(
                            "Unsupported type '{}' for field '{}'",
                            type_str, field
                        ))
                    })?;

                    let mut prop_schema = PropertySchema::new(parsed_type);

                    // Parse enum values
                    if let Some(enum_value) = spec.get("enum") {
                        let enum_arr = enum_value.as_array().ok_or_else(|| {
                            MongoLiteError::SchemaError(format!(
                                "Property '{}' enum must be an array",
                                field
                            ))
                        })?;
                        prop_schema.enum_values = Some(enum_arr.clone());
                    }

                    // Parse pattern (regex)
                    if let Some(pattern_value) = spec.get("pattern") {
                        let pattern_str = pattern_value.as_str().ok_or_else(|| {
                            MongoLiteError::SchemaError(format!(
                                "Property '{}' pattern must be a string",
                                field
                            ))
                        })?;
                        let regex = Regex::new(pattern_str).map_err(|e| {
                            MongoLiteError::SchemaError(format!(
                                "Property '{}' has invalid regex pattern: {}",
                                field, e
                            ))
                        })?;
                        prop_schema.pattern = Some(regex);
                    }

                    // Parse minItems (array constraint)
                    if let Some(min_value) = spec.get("minItems") {
                        let min = min_value.as_u64().ok_or_else(|| {
                            MongoLiteError::SchemaError(format!(
                                "Property '{}' minItems must be a non-negative integer",
                                field
                            ))
                        })?;
                        prop_schema.min_items = Some(min as usize);
                    }

                    // Parse maxItems (array constraint)
                    if let Some(max_value) = spec.get("maxItems") {
                        let max = max_value.as_u64().ok_or_else(|| {
                            MongoLiteError::SchemaError(format!(
                                "Property '{}' maxItems must be a non-negative integer",
                                field
                            ))
                        })?;
                        prop_schema.max_items = Some(max as usize);
                    }

                    properties.insert(field.clone(), prop_schema);
                }
            }
        }

        Ok(Self {
            required,
            properties,
        })
    }

    pub fn validate(&self, value: &Value) -> Result<()> {
        let obj = value.as_object().ok_or_else(|| {
            MongoLiteError::SchemaError("Document must be a JSON object".to_string())
        })?;

        // Check required fields
        for field in &self.required {
            if !obj.contains_key(field) {
                return Err(MongoLiteError::SchemaError(format!(
                    "Missing required field '{}'",
                    field
                )));
            }
        }

        // Validate each property
        for (field, prop_schema) in &self.properties {
            if let Some(field_value) = obj.get(field) {
                // Type validation
                if !prop_schema.schema_type.matches(field_value) {
                    return Err(MongoLiteError::SchemaError(format!(
                        "Field '{}' expected type {}",
                        field,
                        prop_schema.schema_type.as_str()
                    )));
                }

                // Enum validation
                if let Some(enum_values) = &prop_schema.enum_values {
                    if !enum_values.contains(field_value) {
                        return Err(MongoLiteError::SchemaError(format!(
                            "Field '{}' value not in allowed enum values: {:?}",
                            field, enum_values
                        )));
                    }
                }

                // Pattern (regex) validation - only for strings
                if let Some(pattern) = &prop_schema.pattern {
                    if let Some(s) = field_value.as_str() {
                        if !pattern.is_match(s) {
                            return Err(MongoLiteError::SchemaError(format!(
                                "Field '{}' does not match required pattern",
                                field
                            )));
                        }
                    }
                }

                // Array constraints validation
                if let Some(arr) = field_value.as_array() {
                    // minItems validation
                    if let Some(min) = prop_schema.min_items {
                        if arr.len() < min {
                            return Err(MongoLiteError::SchemaError(format!(
                                "Field '{}' has {} items, minimum required is {}",
                                field,
                                arr.len(),
                                min
                            )));
                        }
                    }

                    // maxItems validation
                    if let Some(max) = prop_schema.max_items {
                        if arr.len() > max {
                            return Err(MongoLiteError::SchemaError(format!(
                                "Field '{}' has {} items, maximum allowed is {}",
                                field,
                                arr.len(),
                                max
                            )));
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ========== SchemaType::from_str tests ==========

    #[test]
    fn test_schema_type_from_str_string() {
        assert!(matches!(
            SchemaType::from_str("string"),
            Some(SchemaType::String)
        ));
    }

    #[test]
    fn test_schema_type_from_str_number() {
        assert!(matches!(
            SchemaType::from_str("number"),
            Some(SchemaType::Number)
        ));
    }

    #[test]
    fn test_schema_type_from_str_integer() {
        assert!(matches!(
            SchemaType::from_str("integer"),
            Some(SchemaType::Number)
        ));
    }

    #[test]
    fn test_schema_type_from_str_boolean() {
        assert!(matches!(
            SchemaType::from_str("boolean"),
            Some(SchemaType::Boolean)
        ));
    }

    #[test]
    fn test_schema_type_from_str_object() {
        assert!(matches!(
            SchemaType::from_str("object"),
            Some(SchemaType::Object)
        ));
    }

    #[test]
    fn test_schema_type_from_str_array() {
        assert!(matches!(
            SchemaType::from_str("array"),
            Some(SchemaType::Array)
        ));
    }

    #[test]
    fn test_schema_type_from_str_unknown() {
        assert!(SchemaType::from_str("unknown").is_none());
        assert!(SchemaType::from_str("").is_none());
        assert!(SchemaType::from_str("int").is_none());
    }

    // ========== SchemaType::matches tests ==========

    #[test]
    fn test_schema_type_matches_string() {
        assert!(SchemaType::String.matches(&json!("hello")));
        assert!(!SchemaType::String.matches(&json!(123)));
        assert!(!SchemaType::String.matches(&json!(true)));
    }

    #[test]
    fn test_schema_type_matches_number() {
        assert!(SchemaType::Number.matches(&json!(123)));
        assert!(SchemaType::Number.matches(&json!(1.5)));
        assert!(!SchemaType::Number.matches(&json!("123")));
    }

    #[test]
    fn test_schema_type_matches_boolean() {
        assert!(SchemaType::Boolean.matches(&json!(true)));
        assert!(SchemaType::Boolean.matches(&json!(false)));
        assert!(!SchemaType::Boolean.matches(&json!(1)));
    }

    #[test]
    fn test_schema_type_matches_object() {
        assert!(SchemaType::Object.matches(&json!({"key": "value"})));
        assert!(!SchemaType::Object.matches(&json!([1, 2, 3])));
    }

    #[test]
    fn test_schema_type_matches_array() {
        assert!(SchemaType::Array.matches(&json!([1, 2, 3])));
        assert!(SchemaType::Array.matches(&json!([])));
        assert!(!SchemaType::Array.matches(&json!({"key": "value"})));
    }

    // ========== SchemaType::as_str tests ==========

    #[test]
    fn test_schema_type_as_str() {
        assert_eq!(SchemaType::String.as_str(), "string");
        assert_eq!(SchemaType::Number.as_str(), "number");
        assert_eq!(SchemaType::Boolean.as_str(), "boolean");
        assert_eq!(SchemaType::Object.as_str(), "object");
        assert_eq!(SchemaType::Array.as_str(), "array");
    }

    // ========== CompiledSchema::from_value tests ==========

    #[test]
    fn test_compiled_schema_from_value_basic() {
        let schema = json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "number"}
            }
        });

        let compiled = CompiledSchema::from_value(&schema).unwrap();
        assert_eq!(compiled.required, vec!["name"]);
        assert_eq!(compiled.properties.len(), 2);
    }

    #[test]
    fn test_compiled_schema_from_value_no_type() {
        // Schema without explicit type should work (defaults to object behavior)
        let schema = json!({
            "required": ["id"],
            "properties": {
                "id": {"type": "string"}
            }
        });

        let compiled = CompiledSchema::from_value(&schema).unwrap();
        assert_eq!(compiled.required, vec!["id"]);
    }

    #[test]
    fn test_compiled_schema_from_value_non_object_schema() {
        let schema = json!("not an object");
        let result = CompiledSchema::from_value(&schema);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must be a JSON object"));
    }

    #[test]
    fn test_compiled_schema_from_value_type_not_string() {
        let schema = json!({
            "type": 123
        });
        let result = CompiledSchema::from_value(&schema);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("type must be a string"));
    }

    #[test]
    fn test_compiled_schema_from_value_non_object_type() {
        let schema = json!({
            "type": "array"
        });
        let result = CompiledSchema::from_value(&schema);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Only object schemas"));
    }

    #[test]
    fn test_compiled_schema_from_value_required_not_array() {
        let schema = json!({
            "type": "object",
            "required": "name"
        });
        let result = CompiledSchema::from_value(&schema);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("required must be an array"));
    }

    #[test]
    fn test_compiled_schema_from_value_required_entry_not_string() {
        let schema = json!({
            "type": "object",
            "required": [123]
        });
        let result = CompiledSchema::from_value(&schema);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("required entries must be strings"));
    }

    #[test]
    fn test_compiled_schema_from_value_properties_not_object() {
        let schema = json!({
            "type": "object",
            "properties": "not an object"
        });
        let result = CompiledSchema::from_value(&schema);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("properties must be an object"));
    }

    #[test]
    fn test_compiled_schema_from_value_property_type_not_string() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": 123}
            }
        });
        let result = CompiledSchema::from_value(&schema);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("type must be a string"));
    }

    #[test]
    fn test_compiled_schema_from_value_unsupported_type() {
        let schema = json!({
            "type": "object",
            "properties": {
                "data": {"type": "binary"}
            }
        });
        let result = CompiledSchema::from_value(&schema);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unsupported type"));
    }

    #[test]
    fn test_compiled_schema_from_value_all_types() {
        let schema = json!({
            "type": "object",
            "properties": {
                "str_field": {"type": "string"},
                "num_field": {"type": "number"},
                "int_field": {"type": "integer"},
                "bool_field": {"type": "boolean"},
                "obj_field": {"type": "object"},
                "arr_field": {"type": "array"}
            }
        });

        let compiled = CompiledSchema::from_value(&schema).unwrap();
        assert_eq!(compiled.properties.len(), 6);
    }

    // ========== CompiledSchema::validate tests ==========

    #[test]
    fn test_validate_success() {
        let schema = json!({
            "type": "object",
            "required": ["name", "age"],
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "number"}
            }
        });

        let compiled = CompiledSchema::from_value(&schema).unwrap();
        let doc = json!({"name": "Alice", "age": 30});
        assert!(compiled.validate(&doc).is_ok());
    }

    #[test]
    fn test_validate_not_object() {
        let schema = json!({"type": "object"});
        let compiled = CompiledSchema::from_value(&schema).unwrap();

        let result = compiled.validate(&json!("not an object"));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must be a JSON object"));
    }

    #[test]
    fn test_validate_missing_required() {
        let schema = json!({
            "type": "object",
            "required": ["name", "email"]
        });
        let compiled = CompiledSchema::from_value(&schema).unwrap();

        let doc = json!({"name": "Alice"});
        let result = compiled.validate(&doc);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Missing required field 'email'"));
    }

    #[test]
    fn test_validate_type_mismatch() {
        let schema = json!({
            "type": "object",
            "properties": {
                "age": {"type": "number"}
            }
        });
        let compiled = CompiledSchema::from_value(&schema).unwrap();

        let doc = json!({"age": "thirty"});
        let result = compiled.validate(&doc);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("expected type number"));
    }

    #[test]
    fn test_validate_extra_fields_allowed() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"}
            }
        });
        let compiled = CompiledSchema::from_value(&schema).unwrap();

        // Extra field "extra" should be allowed
        let doc = json!({"name": "Alice", "extra": "allowed"});
        assert!(compiled.validate(&doc).is_ok());
    }

    #[test]
    fn test_validate_optional_field_absent() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "number"}
            }
        });
        let compiled = CompiledSchema::from_value(&schema).unwrap();

        // "age" is not required, so absent is OK
        let doc = json!({"name": "Alice"});
        assert!(compiled.validate(&doc).is_ok());
    }

    #[test]
    fn test_validate_all_types() {
        let schema = json!({
            "type": "object",
            "properties": {
                "str": {"type": "string"},
                "num": {"type": "number"},
                "bool": {"type": "boolean"},
                "obj": {"type": "object"},
                "arr": {"type": "array"}
            }
        });
        let compiled = CompiledSchema::from_value(&schema).unwrap();

        let doc = json!({
            "str": "hello",
            "num": 42,
            "bool": true,
            "obj": {"nested": 1},
            "arr": [1, 2, 3]
        });
        assert!(compiled.validate(&doc).is_ok());
    }

    // ========== Enum validation tests ==========

    #[test]
    fn test_enum_valid_value() {
        let schema = json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["active", "inactive", "pending"]
                }
            }
        });
        let compiled = CompiledSchema::from_value(&schema).unwrap();

        let doc = json!({"status": "active"});
        assert!(compiled.validate(&doc).is_ok());

        let doc2 = json!({"status": "pending"});
        assert!(compiled.validate(&doc2).is_ok());
    }

    #[test]
    fn test_enum_invalid_value() {
        let schema = json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["active", "inactive", "pending"]
                }
            }
        });
        let compiled = CompiledSchema::from_value(&schema).unwrap();

        let doc = json!({"status": "deleted"});
        let result = compiled.validate(&doc);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not in allowed enum values"));
    }

    #[test]
    fn test_enum_with_numbers() {
        let schema = json!({
            "type": "object",
            "properties": {
                "priority": {
                    "type": "number",
                    "enum": [1, 2, 3]
                }
            }
        });
        let compiled = CompiledSchema::from_value(&schema).unwrap();

        // Valid
        let doc = json!({"priority": 2});
        assert!(compiled.validate(&doc).is_ok());

        // Invalid
        let doc2 = json!({"priority": 5});
        let result = compiled.validate(&doc2);
        assert!(result.is_err());
    }

    #[test]
    fn test_enum_not_array_error() {
        let schema = json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": "not_an_array"
                }
            }
        });
        let result = CompiledSchema::from_value(&schema);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("enum must be an array"));
    }

    // ========== Pattern validation tests ==========

    #[test]
    fn test_pattern_match() {
        let schema = json!({
            "type": "object",
            "properties": {
                "version": {
                    "type": "string",
                    "pattern": "^\\d+\\.\\d+\\.\\d+$"
                }
            }
        });
        let compiled = CompiledSchema::from_value(&schema).unwrap();

        let doc = json!({"version": "1.2.3"});
        assert!(compiled.validate(&doc).is_ok());

        let doc2 = json!({"version": "10.20.30"});
        assert!(compiled.validate(&doc2).is_ok());
    }

    #[test]
    fn test_pattern_no_match() {
        let schema = json!({
            "type": "object",
            "properties": {
                "version": {
                    "type": "string",
                    "pattern": "^\\d+\\.\\d+\\.\\d+$"
                }
            }
        });
        let compiled = CompiledSchema::from_value(&schema).unwrap();

        let doc = json!({"version": "invalid-version"});
        let result = compiled.validate(&doc);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("does not match required pattern"));
    }

    #[test]
    fn test_pattern_email_format() {
        let schema = json!({
            "type": "object",
            "properties": {
                "email": {
                    "type": "string",
                    "pattern": "^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}$"
                }
            }
        });
        let compiled = CompiledSchema::from_value(&schema).unwrap();

        // Valid email
        let doc = json!({"email": "test@example.com"});
        assert!(compiled.validate(&doc).is_ok());

        // Invalid email
        let doc2 = json!({"email": "not-an-email"});
        let result = compiled.validate(&doc2);
        assert!(result.is_err());
    }

    #[test]
    fn test_pattern_invalid_regex() {
        let schema = json!({
            "type": "object",
            "properties": {
                "field": {
                    "type": "string",
                    "pattern": "[invalid(regex"
                }
            }
        });
        let result = CompiledSchema::from_value(&schema);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("invalid regex pattern"));
    }

    #[test]
    fn test_pattern_not_string_error() {
        let schema = json!({
            "type": "object",
            "properties": {
                "field": {
                    "type": "string",
                    "pattern": 123
                }
            }
        });
        let result = CompiledSchema::from_value(&schema);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("pattern must be a string"));
    }

    // ========== Array constraints (minItems/maxItems) tests ==========

    #[test]
    fn test_array_min_items_valid() {
        let schema = json!({
            "type": "object",
            "properties": {
                "tags": {
                    "type": "array",
                    "minItems": 1
                }
            }
        });
        let compiled = CompiledSchema::from_value(&schema).unwrap();

        let doc = json!({"tags": ["one"]});
        assert!(compiled.validate(&doc).is_ok());

        let doc2 = json!({"tags": ["one", "two", "three"]});
        assert!(compiled.validate(&doc2).is_ok());
    }

    #[test]
    fn test_array_min_items_invalid() {
        let schema = json!({
            "type": "object",
            "properties": {
                "tags": {
                    "type": "array",
                    "minItems": 2
                }
            }
        });
        let compiled = CompiledSchema::from_value(&schema).unwrap();

        let doc = json!({"tags": ["only_one"]});
        let result = compiled.validate(&doc);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("minimum required is 2"));
    }

    #[test]
    fn test_array_min_items_empty_array() {
        let schema = json!({
            "type": "object",
            "properties": {
                "tags": {
                    "type": "array",
                    "minItems": 1
                }
            }
        });
        let compiled = CompiledSchema::from_value(&schema).unwrap();

        let doc = json!({"tags": []});
        let result = compiled.validate(&doc);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("has 0 items"));
    }

    #[test]
    fn test_array_max_items_valid() {
        let schema = json!({
            "type": "object",
            "properties": {
                "tags": {
                    "type": "array",
                    "maxItems": 3
                }
            }
        });
        let compiled = CompiledSchema::from_value(&schema).unwrap();

        let doc = json!({"tags": ["one", "two"]});
        assert!(compiled.validate(&doc).is_ok());

        let doc2 = json!({"tags": ["one", "two", "three"]});
        assert!(compiled.validate(&doc2).is_ok());
    }

    #[test]
    fn test_array_max_items_invalid() {
        let schema = json!({
            "type": "object",
            "properties": {
                "tags": {
                    "type": "array",
                    "maxItems": 2
                }
            }
        });
        let compiled = CompiledSchema::from_value(&schema).unwrap();

        let doc = json!({"tags": ["one", "two", "three"]});
        let result = compiled.validate(&doc);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("maximum allowed is 2"));
    }

    #[test]
    fn test_array_min_max_items_combined() {
        let schema = json!({
            "type": "object",
            "properties": {
                "tags": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 5
                }
            }
        });
        let compiled = CompiledSchema::from_value(&schema).unwrap();

        // Valid - within range
        let doc = json!({"tags": ["one", "two", "three"]});
        assert!(compiled.validate(&doc).is_ok());

        // Invalid - too few
        let doc2 = json!({"tags": []});
        assert!(compiled.validate(&doc2).is_err());

        // Invalid - too many
        let doc3 = json!({"tags": ["1", "2", "3", "4", "5", "6"]});
        assert!(compiled.validate(&doc3).is_err());
    }

    #[test]
    fn test_min_items_not_integer_error() {
        let schema = json!({
            "type": "object",
            "properties": {
                "tags": {
                    "type": "array",
                    "minItems": "two"
                }
            }
        });
        let result = CompiledSchema::from_value(&schema);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("minItems must be a non-negative integer"));
    }

    #[test]
    fn test_max_items_not_integer_error() {
        let schema = json!({
            "type": "object",
            "properties": {
                "tags": {
                    "type": "array",
                    "maxItems": "five"
                }
            }
        });
        let result = CompiledSchema::from_value(&schema);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("maxItems must be a non-negative integer"));
    }

    // ========== Combined constraints tests ==========

    #[test]
    fn test_combined_enum_and_pattern() {
        // Both enum and pattern on same field
        let schema = json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "enum": ["A1", "A2", "B1", "B2"],
                    "pattern": "^[A-B][1-2]$"
                }
            }
        });
        let compiled = CompiledSchema::from_value(&schema).unwrap();

        // Valid
        let doc = json!({"code": "A1"});
        assert!(compiled.validate(&doc).is_ok());

        // Invalid - not in enum
        let doc2 = json!({"code": "C1"});
        assert!(compiled.validate(&doc2).is_err());
    }

    #[test]
    fn test_schema_with_all_new_constraints() {
        let schema = json!({
            "type": "object",
            "required": ["status", "tags"],
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["active", "inactive"]
                },
                "version": {
                    "type": "string",
                    "pattern": "^v\\d+$"
                },
                "tags": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 10
                }
            }
        });
        let compiled = CompiledSchema::from_value(&schema).unwrap();

        // All valid
        let doc = json!({
            "status": "active",
            "version": "v2",
            "tags": ["important", "urgent"]
        });
        assert!(compiled.validate(&doc).is_ok());
    }
}
