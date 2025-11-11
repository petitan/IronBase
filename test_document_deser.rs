use serde::{Serialize, Deserialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    #[serde(rename = "_id")]
    pub id: i32,

    #[serde(flatten)]
    pub fields: HashMap<String, Value>,
}

fn main() {
    let json_str = r#"{"_id":1,"age":30,"_collection":"users","name":"Alice"}"#;

    let doc: Document = serde_json::from_str(json_str).unwrap();

    println!("Document.id = {}", doc.id);
    println!("Document.fields = {:?}", doc.fields);
    println!("fields.contains_key(\"_id\") = {}", doc.fields.contains_key("_id"));
}
