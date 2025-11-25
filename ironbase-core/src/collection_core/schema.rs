use std::collections::HashMap;

use serde_json::Value;

use crate::error::{MongoLiteError, Result};

#[derive(Clone, Debug)]
pub struct CompiledSchema {
    pub(super) required: Vec<String>,
    pub(super) properties: HashMap<String, SchemaType>,
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
                    properties.insert(field.clone(), parsed_type);
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

        for field in &self.required {
            if !obj.contains_key(field) {
                return Err(MongoLiteError::SchemaError(format!(
                    "Missing required field '{}'",
                    field
                )));
            }
        }

        for (field, schema_type) in &self.properties {
            if let Some(field_value) = obj.get(field) {
                if !schema_type.matches(field_value) {
                    return Err(MongoLiteError::SchemaError(format!(
                        "Field '{}' expected type {}",
                        field,
                        schema_type.as_str()
                    )));
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
        assert!(matches!(SchemaType::from_str("string"), Some(SchemaType::String)));
    }

    #[test]
    fn test_schema_type_from_str_number() {
        assert!(matches!(SchemaType::from_str("number"), Some(SchemaType::Number)));
    }

    #[test]
    fn test_schema_type_from_str_integer() {
        assert!(matches!(SchemaType::from_str("integer"), Some(SchemaType::Number)));
    }

    #[test]
    fn test_schema_type_from_str_boolean() {
        assert!(matches!(SchemaType::from_str("boolean"), Some(SchemaType::Boolean)));
    }

    #[test]
    fn test_schema_type_from_str_object() {
        assert!(matches!(SchemaType::from_str("object"), Some(SchemaType::Object)));
    }

    #[test]
    fn test_schema_type_from_str_array() {
        assert!(matches!(SchemaType::from_str("array"), Some(SchemaType::Array)));
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
        assert!(SchemaType::Number.matches(&json!(3.14)));
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
        assert!(result.unwrap_err().to_string().contains("must be a JSON object"));
    }

    #[test]
    fn test_compiled_schema_from_value_type_not_string() {
        let schema = json!({
            "type": 123
        });
        let result = CompiledSchema::from_value(&schema);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("type must be a string"));
    }

    #[test]
    fn test_compiled_schema_from_value_non_object_type() {
        let schema = json!({
            "type": "array"
        });
        let result = CompiledSchema::from_value(&schema);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Only object schemas"));
    }

    #[test]
    fn test_compiled_schema_from_value_required_not_array() {
        let schema = json!({
            "type": "object",
            "required": "name"
        });
        let result = CompiledSchema::from_value(&schema);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("required must be an array"));
    }

    #[test]
    fn test_compiled_schema_from_value_required_entry_not_string() {
        let schema = json!({
            "type": "object",
            "required": [123]
        });
        let result = CompiledSchema::from_value(&schema);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("required entries must be strings"));
    }

    #[test]
    fn test_compiled_schema_from_value_properties_not_object() {
        let schema = json!({
            "type": "object",
            "properties": "not an object"
        });
        let result = CompiledSchema::from_value(&schema);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("properties must be an object"));
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
        assert!(result.unwrap_err().to_string().contains("type must be a string"));
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
        assert!(result.unwrap_err().to_string().contains("must be a JSON object"));
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
        assert!(result.unwrap_err().to_string().contains("Missing required field 'email'"));
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
        assert!(result.unwrap_err().to_string().contains("expected type number"));
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
}
