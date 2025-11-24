use std::collections::HashMap;

use serde_json::Value;

use crate::error::{MongoLiteError, Result};

#[derive(Clone)]
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
