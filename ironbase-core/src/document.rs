// src/document.rs
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

/// MongoDB-szerű dokumentum
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    #[serde(rename = "_id")]
    pub id: DocumentId,

    #[serde(flatten)]
    pub fields: HashMap<String, Value>,
}

/// Dokumentum ID típusok
/// FONTOS: Untagged, hogy a dokumentumokban egyszerű értékként jelenjen meg: {"_id": 2}
/// A metadat catalog-ban külön kezeljük a típus megőrzést custom serialization-nel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(untagged)]
pub enum DocumentId {
    Int(i64),
    String(String),
    ObjectId(String), // BSON ObjectId string reprezentáció
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
        let mut doc: Self = serde_json::from_str(json)?;

        // WORKAROUND: serde's #[serde(rename = "_id")] + #[serde(flatten)]
        // consumes _id and doesn't put it in fields.
        // For query matching to work, we need _id in fields too.
        doc.fields
            .insert("_id".to_string(), serde_json::to_value(&doc.id)?);

        Ok(doc)
    }

    /// 🚀 OPTIMIZED: Create Document directly from serde_json::Value
    /// Avoids Value → String → Document round-trip serialization
    pub fn from_value(value: &Value) -> serde_json::Result<Self> {
        let mut doc: Self = serde_json::from_value(value.clone())?;

        // WORKAROUND: same as from_json - _id needs to be in fields for query matching
        doc.fields
            .insert("_id".to_string(), serde_json::to_value(&doc.id)?);

        Ok(doc)
    }

    /// Dokumentum JSON-be
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }

    /// Mező lekérése (includes _id)
    /// WORKAROUND: Since _id is in doc.id field after deserialization,
    /// we can't return a reference to it. The query engine must special-case _id matching.
    pub fn get(&self, field: &str) -> Option<&Value> {
        if field.is_empty() {
            return None;
        }
        if field.contains('.') {
            let mut value = self.fields.get(field.split('.').next().unwrap())?;
            for part in field.split('.').skip(1) {
                match value {
                    Value::Object(map) => {
                        value = map.get(part)?;
                    }
                    Value::Array(arr) => {
                        if let Ok(index) = part.parse::<usize>() {
                            value = arr.get(index)?;
                        } else {
                            return None;
                        }
                    }
                    _ => return None,
                }
            }
            Some(value)
        } else {
            self.fields.get(field)
        }
    }

    /// Get the _id value as a JSON Value (for query matching)
    pub fn get_id_value(&self) -> Value {
        serde_json::to_value(&self.id).unwrap()
    }

    /// Mező beállítása (top-level only, use set_nested for dot notation)
    pub fn set(&mut self, field: String, value: Value) {
        self.fields.insert(field, value);
    }

    /// Mező beállítása dot notation támogatással (MongoDB-style)
    /// Pl: "address.city" -> address object-ben a city mezőt állítja be
    pub fn set_nested(&mut self, field: &str, value: Value) {
        if !field.contains('.') {
            self.fields.insert(field.to_string(), value);
            return;
        }

        let parts: Vec<&str> = field.split('.').collect();
        let first = parts[0];

        // Ha nincs még ilyen top-level mező, létrehozzuk a teljes útvonalat
        if !self.fields.contains_key(first) {
            let nested = Self::create_nested_value(&parts[1..], value);
            self.fields.insert(first.to_string(), nested);
            return;
        }

        // Meglévő struktúra módosítása
        let root = self.fields.get_mut(first).unwrap();
        Self::set_value_at_path(root, &parts[1..], value);
    }

    /// Helper: Beágyazott struktúra létrehozása
    fn create_nested_value(parts: &[&str], value: Value) -> Value {
        if parts.is_empty() {
            return value;
        }

        let mut obj = serde_json::Map::new();
        obj.insert(
            parts[0].to_string(),
            Self::create_nested_value(&parts[1..], value),
        );
        Value::Object(obj)
    }

    /// Helper: Érték beállítása az útvonal mentén
    fn set_value_at_path(current: &mut Value, parts: &[&str], value: Value) {
        if parts.is_empty() {
            return;
        }

        if parts.len() == 1 {
            // Utolsó rész - itt kell beállítani az értéket
            match current {
                Value::Object(map) => {
                    map.insert(parts[0].to_string(), value);
                }
                Value::Array(arr) => {
                    if let Ok(index) = parts[0].parse::<usize>() {
                        if index < arr.len() {
                            arr[index] = value;
                        }
                    }
                }
                _ => {
                    // Ha nem object/array, cseréljük le object-re
                    let mut obj = serde_json::Map::new();
                    obj.insert(parts[0].to_string(), value);
                    *current = Value::Object(obj);
                }
            }
            return;
        }

        // Köztes rész - navigálunk vagy létrehozunk
        match current {
            Value::Object(map) => {
                if !map.contains_key(parts[0]) {
                    // Nem létezik - létrehozzuk a maradék útvonalat
                    map.insert(
                        parts[0].to_string(),
                        Self::create_nested_value(&parts[1..], value),
                    );
                } else {
                    // Létezik - rekurzívan folytatjuk
                    let next = map.get_mut(parts[0]).unwrap();
                    Self::set_value_at_path(next, &parts[1..], value);
                }
            }
            Value::Array(arr) => {
                if let Ok(index) = parts[0].parse::<usize>() {
                    if index < arr.len() {
                        Self::set_value_at_path(&mut arr[index], &parts[1..], value);
                    }
                }
            }
            _ => {
                // Nem navigálható - cseréljük le object-re
                let nested = Self::create_nested_value(parts, value);
                *current = nested;
            }
        }
    }

    /// Mező törlése (top-level only)
    pub fn remove(&mut self, field: &str) -> Option<Value> {
        self.fields.remove(field)
    }

    /// Mező törlése dot notation támogatással (MongoDB-style)
    pub fn remove_nested(&mut self, field: &str) -> Option<Value> {
        if !field.contains('.') {
            return self.fields.remove(field);
        }

        let parts: Vec<&str> = field.split('.').collect();
        let first = parts[0];

        if !self.fields.contains_key(first) {
            return None;
        }

        let root = self.fields.get_mut(first)?;
        Self::remove_value_at_path(root, &parts[1..])
    }

    /// Helper: Érték törlése az útvonal mentén
    fn remove_value_at_path(current: &mut Value, parts: &[&str]) -> Option<Value> {
        if parts.is_empty() {
            return None;
        }

        if parts.len() == 1 {
            match current {
                Value::Object(map) => map.remove(parts[0]),
                Value::Array(arr) => {
                    if let Ok(index) = parts[0].parse::<usize>() {
                        if index < arr.len() {
                            return Some(arr.remove(index));
                        }
                    }
                    None
                }
                _ => None,
            }
        } else {
            match current {
                Value::Object(map) => {
                    let next = map.get_mut(parts[0])?;
                    Self::remove_value_at_path(next, &parts[1..])
                }
                Value::Array(arr) => {
                    if let Ok(index) = parts[0].parse::<usize>() {
                        if index < arr.len() {
                            return Self::remove_value_at_path(&mut arr[index], &parts[1..]);
                        }
                    }
                    None
                }
                _ => None,
            }
        }
    }

    /// Módosítható referencia egy beágyazott értékhez (dot notation)
    pub fn get_mut_nested(&mut self, field: &str) -> Option<&mut Value> {
        if field.is_empty() {
            return None;
        }
        if !field.contains('.') {
            return self.fields.get_mut(field);
        }

        let parts: Vec<&str> = field.split('.').collect();
        let first = parts[0];

        let mut current = self.fields.get_mut(first)?;
        for part in &parts[1..] {
            current = match current {
                Value::Object(map) => map.get_mut(*part)?,
                Value::Array(arr) => {
                    if let Ok(index) = part.parse::<usize>() {
                        arr.get_mut(index)?
                    } else {
                        return None;
                    }
                }
                _ => return None,
            };
        }
        Some(current)
    }

    /// Tartalmazza-e a mezőt
    pub fn contains(&self, field: &str) -> bool {
        self.fields.contains_key(field)
    }
}

impl From<Document> for Value {
    fn from(doc: Document) -> Self {
        let mut map = serde_json::Map::new();

        // Többi mező először (including _id if present in fields)
        for (k, v) in doc.fields {
            map.insert(k, v);
        }

        // _id hozzáadása ONLY if not already present
        if !map.contains_key("_id") {
            map.insert("_id".to_string(), serde_json::to_value(&doc.id).unwrap());
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
    fn test_document_deser_id_in_fields() {
        // Test if _id ends up in fields after deserialization with flatten
        let json_str = r#"{"_id":1,"age":30,"name":"Alice"}"#;
        let doc: Document = serde_json::from_str(json_str).unwrap();

        assert_eq!(doc.id, DocumentId::Int(1));
        // With #[serde(flatten)], _id should NOT be duplicated in fields
        // because #[serde(rename = "_id")] on id field consumes it
        assert!(
            !doc.fields.contains_key("_id"),
            "_id should NOT be in fields with flatten + rename!"
        );
        assert_eq!(doc.fields.len(), 2); // Only age and name
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
        fields.insert(
            "metadata".to_string(),
            json!({"version": 1, "stable": true}),
        );

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
        fields.insert(
            "user".to_string(),
            json!({
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
            }),
        );

        let doc = Document::new(DocumentId::Int(1), fields);

        let user_data = doc.get("user").unwrap();
        assert!(user_data.is_object());

        let profile = &user_data["profile"];
        assert_eq!(profile["name"], "Helen");
        assert_eq!(profile["contacts"]["email"], "helen@example.com");
    }

    #[test]
    fn test_document_get_nested_dot_path() {
        let json_str = r#"{
            "_id": 1,
            "address": {"city": "Budapest", "zip": 1111},
            "stats": {"login_count": 42}
        }"#;
        let doc: Document = serde_json::from_str(json_str).unwrap();
        assert_eq!(doc.get("address.city").unwrap(), &json!("Budapest"));
        assert_eq!(doc.get("stats.login_count").unwrap(), &json!(42));
    }
}
