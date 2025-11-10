// src/query.rs
use serde_json::Value;
use std::collections::HashMap;
use crate::document::Document;
use crate::error::{Result, MongoLiteError};

/// Query típusok
#[derive(Debug, Clone)]
pub enum QueryOperator {
    // Összehasonlítás
    Eq(Value),           // $eq
    Ne(Value),           // $ne
    Gt(Value),           // $gt
    Gte(Value),          // $gte
    Lt(Value),           // $lt
    Lte(Value),          // $lte
    In(Vec<Value>),      // $in
    Nin(Vec<Value>),     // $nin
    
    // Logikai
    And(Vec<Query>),     // $and
    Or(Vec<Query>),      // $or
    Not(Box<Query>),     // $not
    Nor(Vec<Query>),     // $nor
    
    // Egyéb
    Exists(bool),        // $exists
    Type(String),        // $type
    Regex(String),       // $regex
}

/// Query - MongoDB-szerű lekérdezés
#[derive(Debug, Clone)]
pub struct Query {
    pub conditions: HashMap<String, QueryOperator>,
}

impl Query {
    /// Új query létrehozása
    pub fn new() -> Self {
        Query {
            conditions: HashMap::new(),
        }
    }
    
    /// Query parsing JSON-ből
    pub fn from_json(json: &Value) -> Result<Self> {
        let mut query = Query::new();

        if let Value::Object(map) = json {
            for (field, condition) in map {
                // Check for top-level logical operators
                if field.starts_with('$') {
                    let operator = Self::parse_logical_operator(field, condition)?;
                    query.conditions.insert(field.clone(), operator);
                } else {
                    let operator = Self::parse_operator(condition)?;
                    query.conditions.insert(field.clone(), operator);
                }
            }
        }

        Ok(query)
    }

    /// Parse logical operators ($and, $or, $not, etc.)
    fn parse_logical_operator(op: &str, value: &Value) -> Result<QueryOperator> {
        match op {
            "$and" => {
                if let Value::Array(arr) = value {
                    let mut queries = Vec::new();
                    for item in arr {
                        queries.push(Self::from_json(item)?);
                    }
                    Ok(QueryOperator::And(queries))
                } else {
                    Err(MongoLiteError::InvalidQuery("$and requires array".into()))
                }
            }
            "$or" => {
                if let Value::Array(arr) = value {
                    let mut queries = Vec::new();
                    for item in arr {
                        queries.push(Self::from_json(item)?);
                    }
                    Ok(QueryOperator::Or(queries))
                } else {
                    Err(MongoLiteError::InvalidQuery("$or requires array".into()))
                }
            }
            "$nor" => {
                if let Value::Array(arr) = value {
                    let mut queries = Vec::new();
                    for item in arr {
                        queries.push(Self::from_json(item)?);
                    }
                    Ok(QueryOperator::Nor(queries))
                } else {
                    Err(MongoLiteError::InvalidQuery("$nor requires array".into()))
                }
            }
            _ => Err(MongoLiteError::InvalidQuery(format!("Unknown logical operator: {}", op)))
        }
    }

    /// Operátor parsing
    fn parse_operator(value: &Value) -> Result<QueryOperator> {
        match value {
            // Egyszerű egyenlőség
            Value::String(_) | Value::Number(_) | Value::Bool(_) => {
                Ok(QueryOperator::Eq(value.clone()))
            }

            // Operátorok
            Value::Object(map) => {
                if let Some((op, val)) = map.iter().next() {
                    match op.as_str() {
                        "$eq" => Ok(QueryOperator::Eq(val.clone())),
                        "$ne" => Ok(QueryOperator::Ne(val.clone())),
                        "$gt" => Ok(QueryOperator::Gt(val.clone())),
                        "$gte" => Ok(QueryOperator::Gte(val.clone())),
                        "$lt" => Ok(QueryOperator::Lt(val.clone())),
                        "$lte" => Ok(QueryOperator::Lte(val.clone())),
                        "$in" => {
                            if let Value::Array(arr) = val {
                                Ok(QueryOperator::In(arr.clone()))
                            } else {
                                Err(MongoLiteError::InvalidQuery("$in requires array".into()))
                            }
                        }
                        "$nin" => {
                            if let Value::Array(arr) = val {
                                Ok(QueryOperator::Nin(arr.clone()))
                            } else {
                                Err(MongoLiteError::InvalidQuery("$nin requires array".into()))
                            }
                        }
                        "$not" => {
                            // $not wraps another operator - parse it recursively
                            let inner_operator = Self::parse_operator(val)?;
                            // Wrap in a special Not operator that contains the inner operator
                            // We'll handle this specially in matches_operator
                            let mut dummy_query = Query::new();
                            dummy_query.conditions.insert("_field_".to_string(), inner_operator);
                            Ok(QueryOperator::Not(Box::new(dummy_query)))
                        }
                        "$exists" => {
                            if let Value::Bool(b) = val {
                                Ok(QueryOperator::Exists(*b))
                            } else {
                                Err(MongoLiteError::InvalidQuery("$exists requires bool".into()))
                            }
                        }
                        "$regex" => {
                            if let Value::String(s) = val {
                                Ok(QueryOperator::Regex(s.clone()))
                            } else {
                                Err(MongoLiteError::InvalidQuery("$regex requires string".into()))
                            }
                        }
                        _ => Err(MongoLiteError::InvalidQuery(format!("Unknown operator: {}", op)))
                    }
                } else {
                    Ok(QueryOperator::Eq(value.clone()))
                }
            }

            _ => Ok(QueryOperator::Eq(value.clone()))
        }
    }
    
    /// Dokumentum illeszkedik-e a query-re
    pub fn matches(&self, document: &Document) -> bool {
        for (field, operator) in &self.conditions {
            // Check if this is a logical operator (starts with $)
            if field.starts_with('$') {
                if !Self::matches_logical_operator(operator, document) {
                    return false;
                }
            } else {
                let field_value = document.get(field);
                if !Self::matches_operator(field_value, operator, document) {
                    return false;
                }
            }
        }

        true
    }

    /// Logical operator matching
    fn matches_logical_operator(operator: &QueryOperator, document: &Document) -> bool {
        match operator {
            QueryOperator::And(queries) => {
                // All queries must match
                queries.iter().all(|q| q.matches(document))
            }
            QueryOperator::Or(queries) => {
                // At least one query must match
                queries.iter().any(|q| q.matches(document))
            }
            QueryOperator::Nor(queries) => {
                // None of the queries must match
                !queries.iter().any(|q| q.matches(document))
            }
            QueryOperator::Not(query) => {
                // Query must not match
                !query.matches(document)
            }
            _ => false,
        }
    }

    /// Operátor illeszkedés ellenőrzése
    fn matches_operator(value: Option<&Value>, operator: &QueryOperator, document: &Document) -> bool {
        match operator {
            QueryOperator::Eq(target) => {
                value.map_or(false, |v| v == target)
            }

            QueryOperator::Ne(target) => {
                value.map_or(true, |v| v != target)
            }

            QueryOperator::Gt(target) => {
                value.map_or(false, |v| Self::compare_values(v, target) == Some(std::cmp::Ordering::Greater))
            }

            QueryOperator::Gte(target) => {
                value.map_or(false, |v| {
                    matches!(Self::compare_values(v, target), Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal))
                })
            }

            QueryOperator::Lt(target) => {
                value.map_or(false, |v| Self::compare_values(v, target) == Some(std::cmp::Ordering::Less))
            }

            QueryOperator::Lte(target) => {
                value.map_or(false, |v| {
                    matches!(Self::compare_values(v, target), Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal))
                })
            }

            QueryOperator::In(targets) => {
                value.map_or(false, |v| targets.contains(v))
            }

            QueryOperator::Nin(targets) => {
                value.map_or(true, |v| !targets.contains(v))
            }

            QueryOperator::Exists(should_exist) => {
                value.is_some() == *should_exist
            }

            QueryOperator::Not(query) => {
                // For field-level $not - check if the inner operator matches
                // The query contains a single dummy "_field_" condition with the real operator
                if let Some(inner_operator) = query.conditions.get("_field_") {
                    !Self::matches_operator(value, inner_operator, document)
                } else {
                    // Fallback: treat as document-level not
                    !query.matches(document)
                }
            }

            _ => false,
        }
    }
    
    /// Értékek összehasonlítása
    fn compare_values(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
        match (a, b) {
            (Value::Number(n1), Value::Number(n2)) => {
                let f1 = n1.as_f64()?;
                let f2 = n2.as_f64()?;
                f1.partial_cmp(&f2)
            }
            (Value::String(s1), Value::String(s2)) => Some(s1.cmp(s2)),
            (Value::Bool(b1), Value::Bool(b2)) => Some(b1.cmp(b2)),
            _ => None,
        }
    }
}

impl Default for Query {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, DocumentId};
    use serde_json::json;

    fn create_test_document(id: i64, fields: serde_json::Map<String, Value>) -> Document {
        let mut field_map = HashMap::new();
        for (k, v) in fields {
            field_map.insert(k, v);
        }
        Document::new(DocumentId::Int(id), field_map)
    }

    #[test]
    fn test_query_eq_operator() {
        let query = Query::from_json(&json!({"name": "Alice"})).unwrap();

        let doc1 = create_test_document(1, serde_json::Map::from_iter(vec![
            ("name".to_string(), json!("Alice"))
        ]));

        let doc2 = create_test_document(2, serde_json::Map::from_iter(vec![
            ("name".to_string(), json!("Bob"))
        ]));

        assert!(query.matches(&doc1));
        assert!(!query.matches(&doc2));
    }

    #[test]
    fn test_query_ne_operator() {
        let query = Query::from_json(&json!({"age": {"$ne": 30}})).unwrap();

        let doc1 = create_test_document(1, serde_json::Map::from_iter(vec![
            ("age".to_string(), json!(25))
        ]));

        let doc2 = create_test_document(2, serde_json::Map::from_iter(vec![
            ("age".to_string(), json!(30))
        ]));

        assert!(query.matches(&doc1));
        assert!(!query.matches(&doc2));
    }

    #[test]
    fn test_query_gt_operator() {
        let query = Query::from_json(&json!({"score": {"$gt": 50}})).unwrap();

        let doc1 = create_test_document(1, serde_json::Map::from_iter(vec![
            ("score".to_string(), json!(75))
        ]));

        let doc2 = create_test_document(2, serde_json::Map::from_iter(vec![
            ("score".to_string(), json!(30))
        ]));

        assert!(query.matches(&doc1));
        assert!(!query.matches(&doc2));
    }

    #[test]
    fn test_query_gte_operator() {
        let query = Query::from_json(&json!({"age": {"$gte": 18}})).unwrap();

        let doc1 = create_test_document(1, serde_json::Map::from_iter(vec![
            ("age".to_string(), json!(18))
        ]));

        let doc2 = create_test_document(2, serde_json::Map::from_iter(vec![
            ("age".to_string(), json!(25))
        ]));

        let doc3 = create_test_document(3, serde_json::Map::from_iter(vec![
            ("age".to_string(), json!(15))
        ]));

        assert!(query.matches(&doc1));
        assert!(query.matches(&doc2));
        assert!(!query.matches(&doc3));
    }

    #[test]
    fn test_query_lt_operator() {
        let query = Query::from_json(&json!({"price": {"$lt": 100}})).unwrap();

        let doc1 = create_test_document(1, serde_json::Map::from_iter(vec![
            ("price".to_string(), json!(50))
        ]));

        let doc2 = create_test_document(2, serde_json::Map::from_iter(vec![
            ("price".to_string(), json!(150))
        ]));

        assert!(query.matches(&doc1));
        assert!(!query.matches(&doc2));
    }

    #[test]
    fn test_query_lte_operator() {
        let query = Query::from_json(&json!({"rating": {"$lte": 5}})).unwrap();

        let doc1 = create_test_document(1, serde_json::Map::from_iter(vec![
            ("rating".to_string(), json!(5))
        ]));

        let doc2 = create_test_document(2, serde_json::Map::from_iter(vec![
            ("rating".to_string(), json!(3))
        ]));

        let doc3 = create_test_document(3, serde_json::Map::from_iter(vec![
            ("rating".to_string(), json!(7))
        ]));

        assert!(query.matches(&doc1));
        assert!(query.matches(&doc2));
        assert!(!query.matches(&doc3));
    }

    #[test]
    fn test_query_in_operator() {
        let query = Query::from_json(&json!({"city": {"$in": ["NYC", "LA", "SF"]}})).unwrap();

        let doc1 = create_test_document(1, serde_json::Map::from_iter(vec![
            ("city".to_string(), json!("NYC"))
        ]));

        let doc2 = create_test_document(2, serde_json::Map::from_iter(vec![
            ("city".to_string(), json!("Chicago"))
        ]));

        assert!(query.matches(&doc1));
        assert!(!query.matches(&doc2));
    }

    #[test]
    fn test_query_nin_operator() {
        let query = Query::from_json(&json!({"status": {"$nin": ["deleted", "archived"]}})).unwrap();

        let doc1 = create_test_document(1, serde_json::Map::from_iter(vec![
            ("status".to_string(), json!("active"))
        ]));

        let doc2 = create_test_document(2, serde_json::Map::from_iter(vec![
            ("status".to_string(), json!("deleted"))
        ]));

        assert!(query.matches(&doc1));
        assert!(!query.matches(&doc2));
    }

    #[test]
    fn test_query_and_operator() {
        let query = Query::from_json(&json!({
            "$and": [
                {"age": {"$gte": 18}},
                {"city": "NYC"}
            ]
        })).unwrap();

        let doc1 = create_test_document(1, serde_json::Map::from_iter(vec![
            ("age".to_string(), json!(25)),
            ("city".to_string(), json!("NYC"))
        ]));

        let doc2 = create_test_document(2, serde_json::Map::from_iter(vec![
            ("age".to_string(), json!(15)),
            ("city".to_string(), json!("NYC"))
        ]));

        let doc3 = create_test_document(3, serde_json::Map::from_iter(vec![
            ("age".to_string(), json!(25)),
            ("city".to_string(), json!("LA"))
        ]));

        assert!(query.matches(&doc1));
        assert!(!query.matches(&doc2));
        assert!(!query.matches(&doc3));
    }

    #[test]
    fn test_query_or_operator() {
        let query = Query::from_json(&json!({
            "$or": [
                {"age": {"$lt": 18}},
                {"age": {"$gt": 65}}
            ]
        })).unwrap();

        let doc1 = create_test_document(1, serde_json::Map::from_iter(vec![
            ("age".to_string(), json!(15))
        ]));

        let doc2 = create_test_document(2, serde_json::Map::from_iter(vec![
            ("age".to_string(), json!(70))
        ]));

        let doc3 = create_test_document(3, serde_json::Map::from_iter(vec![
            ("age".to_string(), json!(30))
        ]));

        assert!(query.matches(&doc1));
        assert!(query.matches(&doc2));
        assert!(!query.matches(&doc3));
    }

    #[test]
    fn test_query_nor_operator() {
        let query = Query::from_json(&json!({
            "$nor": [
                {"status": "deleted"},
                {"status": "archived"}
            ]
        })).unwrap();

        let doc1 = create_test_document(1, serde_json::Map::from_iter(vec![
            ("status".to_string(), json!("active"))
        ]));

        let doc2 = create_test_document(2, serde_json::Map::from_iter(vec![
            ("status".to_string(), json!("deleted"))
        ]));

        assert!(query.matches(&doc1));
        assert!(!query.matches(&doc2));
    }

    #[test]
    fn test_query_exists_operator() {
        let query_exists = Query::from_json(&json!({"email": {"$exists": true}})).unwrap();
        let query_not_exists = Query::from_json(&json!({"email": {"$exists": false}})).unwrap();

        let doc1 = create_test_document(1, serde_json::Map::from_iter(vec![
            ("email".to_string(), json!("test@example.com"))
        ]));

        let doc2 = create_test_document(2, serde_json::Map::from_iter(vec![
            ("name".to_string(), json!("Alice"))
        ]));

        assert!(query_exists.matches(&doc1));
        assert!(!query_exists.matches(&doc2));
        assert!(!query_not_exists.matches(&doc1));
        assert!(query_not_exists.matches(&doc2));
    }

    #[test]
    fn test_query_complex_nested() {
        let query = Query::from_json(&json!({
            "$and": [
                {
                    "$or": [
                        {"city": "NYC"},
                        {"city": "LA"}
                    ]
                },
                {"age": {"$gte": 25}},
                {"active": true}
            ]
        })).unwrap();

        let doc1 = create_test_document(1, serde_json::Map::from_iter(vec![
            ("city".to_string(), json!("NYC")),
            ("age".to_string(), json!(30)),
            ("active".to_string(), json!(true))
        ]));

        let doc2 = create_test_document(2, serde_json::Map::from_iter(vec![
            ("city".to_string(), json!("LA")),
            ("age".to_string(), json!(20)),
            ("active".to_string(), json!(true))
        ]));

        let doc3 = create_test_document(3, serde_json::Map::from_iter(vec![
            ("city".to_string(), json!("Chicago")),
            ("age".to_string(), json!(30)),
            ("active".to_string(), json!(true))
        ]));

        assert!(query.matches(&doc1));
        assert!(!query.matches(&doc2)); // age < 25
        assert!(!query.matches(&doc3)); // city not in [NYC, LA]
    }

    #[test]
    fn test_query_missing_field() {
        let query = Query::from_json(&json!({"email": "test@example.com"})).unwrap();

        let doc_without_email = create_test_document(1, serde_json::Map::from_iter(vec![
            ("name".to_string(), json!("Alice"))
        ]));

        assert!(!query.matches(&doc_without_email));
    }

    #[test]
    fn test_query_string_comparison() {
        let query = Query::from_json(&json!({"name": {"$gt": "M"}})).unwrap();

        let doc1 = create_test_document(1, serde_json::Map::from_iter(vec![
            ("name".to_string(), json!("Zoe"))
        ]));

        let doc2 = create_test_document(2, serde_json::Map::from_iter(vec![
            ("name".to_string(), json!("Alice"))
        ]));

        assert!(query.matches(&doc1));
        assert!(!query.matches(&doc2));
    }
}