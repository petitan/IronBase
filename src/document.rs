// src/document.rs
use serde::{Serialize, Deserialize};
use serde_json::Value;
use uuid::Uuid;
use std::collections::HashMap;

/// MongoDB-szerű dokumentum
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    #[serde(rename = "_id")]
    pub id: DocumentId,
    
    #[serde(flatten)]
    pub fields: HashMap<String, Value>,
}

/// Dokumentum ID típusok
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(untagged)]
pub enum DocumentId {
    Int(i64),
    String(String),
    ObjectId(String),  // BSON ObjectId string reprezentáció
}

impl DocumentId {
    /// Új auto-increment ID generálás
    pub fn new_auto(last_id: u64) -> Self {
        DocumentId::Int((last_id + 1) as i64)
    }
    
    /// Új ObjectId generálás (UUID v4)
    pub fn new_object_id() -> Self {
        DocumentId::ObjectId(Uuid::new_v4().to_string())
    }
}

impl Document {
    /// Új dokumentum létrehozása
    pub fn new(id: DocumentId, fields: HashMap<String, Value>) -> Self {
        Document { id, fields }
    }
    
    /// Dokumentum JSON-ből
    pub fn from_json(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }
    
    /// Dokumentum JSON-be
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }
    
    /// Mező lekérése
    pub fn get(&self, field: &str) -> Option<&Value> {
        if field == "_id" {
            None  // _id külön kezeljük
        } else {
            self.fields.get(field)
        }
    }
    
    /// Mező beállítása
    pub fn set(&mut self, field: String, value: Value) {
        self.fields.insert(field, value);
    }
    
    /// Mező törlése
    pub fn remove(&mut self, field: &str) -> Option<Value> {
        self.fields.remove(field)
    }
    
    /// Tartalmazza-e a mezőt
    pub fn contains(&self, field: &str) -> bool {
        self.fields.contains_key(field)
    }
}

impl From<Document> for Value {
    fn from(doc: Document) -> Self {
        let mut map = serde_json::Map::new();

        // _id hozzáadása
        map.insert("_id".to_string(), serde_json::to_value(&doc.id).unwrap());

        // Többi mező
        for (k, v) in doc.fields {
            map.insert(k, v);
        }

        Value::Object(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_document_id_int() {
        let id = DocumentId::Int(42);

        match id {
            DocumentId::Int(n) => assert_eq!(n, 42),
            _ => panic!("Expected Int variant"),
        }
    }

    #[test]
    fn test_document_id_string() {
        let id = DocumentId::String("test_id".to_string());

        match id {
            DocumentId::String(s) => assert_eq!(s, "test_id"),
            _ => panic!("Expected String variant"),
        }
    }

    #[test]
    fn test_document_id_object_id() {
        let id = DocumentId::new_object_id();

        match id {
            DocumentId::ObjectId(s) => {
                // UUID v4 format: 8-4-4-4-12 characters
                assert_eq!(s.len(), 36); // UUID with dashes
                assert!(s.contains('-'));
            }
            _ => panic!("Expected ObjectId variant"),
        }
    }

    #[test]
    fn test_document_id_new_auto() {
        let id1 = DocumentId::new_auto(0);
        let id2 = DocumentId::new_auto(10);
        let id3 = DocumentId::new_auto(99);

        assert_eq!(id1, DocumentId::Int(1));
        assert_eq!(id2, DocumentId::Int(11));
        assert_eq!(id3, DocumentId::Int(100));
    }

    #[test]
    fn test_document_creation() {
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), json!("Alice"));
        fields.insert("age".to_string(), json!(30));

        let doc = Document::new(DocumentId::Int(1), fields);

        assert_eq!(doc.id, DocumentId::Int(1));
        assert_eq!(doc.fields.len(), 2);
        assert_eq!(doc.fields.get("name").unwrap(), &json!("Alice"));
        assert_eq!(doc.fields.get("age").unwrap(), &json!(30));
    }

    #[test]
    fn test_document_get_field() {
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), json!("Bob"));
        fields.insert("email".to_string(), json!("bob@example.com"));

        let doc = Document::new(DocumentId::Int(1), fields);

        assert_eq!(doc.get("name").unwrap(), &json!("Bob"));
        assert_eq!(doc.get("email").unwrap(), &json!("bob@example.com"));
        assert!(doc.get("nonexistent").is_none());
    }

    #[test]
    fn test_document_get_id_returns_none() {
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), json!("Carol"));

        let doc = Document::new(DocumentId::Int(1), fields);

        // _id is handled separately, not in fields
        assert!(doc.get("_id").is_none());
    }

    #[test]
    fn test_document_set_field() {
        let fields = HashMap::new();
        let mut doc = Document::new(DocumentId::Int(1), fields);

        doc.set("name".to_string(), json!("Dave"));
        doc.set("age".to_string(), json!(25));

        assert_eq!(doc.fields.len(), 2);
        assert_eq!(doc.get("name").unwrap(), &json!("Dave"));
        assert_eq!(doc.get("age").unwrap(), &json!(25));
    }

    #[test]
    fn test_document_set_overwrites() {
        let mut fields = HashMap::new();
        fields.insert("count".to_string(), json!(1));

        let mut doc = Document::new(DocumentId::Int(1), fields);

        doc.set("count".to_string(), json!(2));
        doc.set("count".to_string(), json!(3));

        assert_eq!(doc.fields.len(), 1);
        assert_eq!(doc.get("count").unwrap(), &json!(3));
    }

    #[test]
    fn test_document_remove_field() {
        let mut fields = HashMap::new();
        fields.insert("temp".to_string(), json!("remove_me"));
        fields.insert("keep".to_string(), json!("stay"));

        let mut doc = Document::new(DocumentId::Int(1), fields);

        let removed = doc.remove("temp");
        assert_eq!(removed, Some(json!("remove_me")));
        assert_eq!(doc.fields.len(), 1);
        assert!(doc.get("temp").is_none());
        assert_eq!(doc.get("keep").unwrap(), &json!("stay"));
    }

    #[test]
    fn test_document_remove_nonexistent() {
        let fields = HashMap::new();
        let mut doc = Document::new(DocumentId::Int(1), fields);

        let removed = doc.remove("nonexistent");
        assert!(removed.is_none());
    }

    #[test]
    fn test_document_contains() {
        let mut fields = HashMap::new();
        fields.insert("active".to_string(), json!(true));

        let doc = Document::new(DocumentId::Int(1), fields);

        assert!(doc.contains("active"));
        assert!(!doc.contains("inactive"));
    }

    #[test]
    fn test_document_to_json() {
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), json!("Eve"));
        fields.insert("score".to_string(), json!(95));

        let doc = Document::new(DocumentId::Int(1), fields);

        let json_str = doc.to_json().unwrap();

        // Parse back to verify structure
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["_id"], 1);
        assert_eq!(parsed["name"], "Eve");
        assert_eq!(parsed["score"], 95);
    }

    #[test]
    fn test_document_from_json() {
        let json_str = r#"{"_id": 42, "name": "Frank", "active": true}"#;

        let doc = Document::from_json(json_str).unwrap();

        assert_eq!(doc.id, DocumentId::Int(42));
        assert_eq!(doc.get("name").unwrap(), &json!("Frank"));
        assert_eq!(doc.get("active").unwrap(), &json!(true));
    }

    #[test]
    fn test_document_from_json_with_string_id() {
        let json_str = r#"{"_id": "abc123", "type": "test"}"#;

        let doc = Document::from_json(json_str).unwrap();

        assert_eq!(doc.id, DocumentId::String("abc123".to_string()));
        assert_eq!(doc.get("type").unwrap(), &json!("test"));
    }

    #[test]
    fn test_document_roundtrip_serialization() {
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), json!("Grace"));
        fields.insert("tags".to_string(), json!(["rust", "database"]));
        fields.insert("metadata".to_string(), json!({"version": 1, "stable": true}));

        let original = Document::new(DocumentId::Int(99), fields);

        // Serialize to JSON
        let json_str = original.to_json().unwrap();

        // Deserialize back
        let restored = Document::from_json(&json_str).unwrap();

        assert_eq!(restored.id, original.id);
        assert_eq!(restored.get("name"), original.get("name"));
        assert_eq!(restored.get("tags"), original.get("tags"));
        assert_eq!(restored.get("metadata"), original.get("metadata"));
    }

    #[test]
    fn test_document_to_value_conversion() {
        let mut fields = HashMap::new();
        fields.insert("key".to_string(), json!("value"));

        let doc = Document::new(DocumentId::Int(7), fields);

        let value: Value = doc.into();

        assert!(value.is_object());
        let obj = value.as_object().unwrap();
        assert_eq!(obj.get("_id").unwrap(), &json!(7));
        assert_eq!(obj.get("key").unwrap(), &json!("value"));
    }

    #[test]
    fn test_document_id_equality() {
        let id1 = DocumentId::Int(42);
        let id2 = DocumentId::Int(42);
        let id3 = DocumentId::Int(99);

        assert_eq!(id1, id2);
        assert_ne!(id1, id3);

        let id4 = DocumentId::String("test".to_string());
        let id5 = DocumentId::String("test".to_string());
        let id6 = DocumentId::String("other".to_string());

        assert_eq!(id4, id5);
        assert_ne!(id4, id6);
        assert_ne!(id1, id4); // Different variants
    }

    #[test]
    fn test_document_empty_fields() {
        let fields = HashMap::new();
        let doc = Document::new(DocumentId::Int(1), fields);

        assert_eq!(doc.fields.len(), 0);
        assert!(doc.get("any").is_none());
    }

    #[test]
    fn test_document_complex_nested_data() {
        let mut fields = HashMap::new();
        fields.insert("user".to_string(), json!({
            "profile": {
                "name": "Helen",
                "contacts": {
                    "email": "helen@example.com",
                    "phones": ["+1234567890", "+0987654321"]
                }
            },
            "settings": {
                "theme": "dark",
                "notifications": true
            }
        }));

        let doc = Document::new(DocumentId::Int(1), fields);

        let user_data = doc.get("user").unwrap();
        assert!(user_data.is_object());

        let profile = &user_data["profile"];
        assert_eq!(profile["name"], "Helen");
        assert_eq!(profile["contacts"]["email"], "helen@example.com");
    }
}