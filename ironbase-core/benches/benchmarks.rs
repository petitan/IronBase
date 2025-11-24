// Criterion benchmarks for MongoLite Core
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use ironbase_core::{DatabaseCore, Document, DocumentId};
use serde_json::json;
use std::collections::HashMap;
use tempfile::TempDir;

// ========== DOCUMENT BENCHMARKS ==========

fn bench_document_creation(c: &mut Criterion) {
    c.bench_function("document_create", |b| {
        b.iter(|| {
            let mut fields = HashMap::new();
            fields.insert("name".to_string(), json!("Alice"));
            fields.insert("age".to_string(), json!(30));
            fields.insert("city".to_string(), json!("NYC"));

            Document::new(DocumentId::Int(1), fields)
        });
    });
}

fn bench_document_serialization(c: &mut Criterion) {
    let mut fields = HashMap::new();
    fields.insert("name".to_string(), json!("Alice"));
    fields.insert("age".to_string(), json!(30));
    fields.insert("email".to_string(), json!("alice@example.com"));
    fields.insert("active".to_string(), json!(true));
    let doc = Document::new(DocumentId::Int(1), fields);

    c.bench_function("document_to_json", |b| {
        b.iter(|| doc.to_json().unwrap());
    });
}

fn bench_document_deserialization(c: &mut Criterion) {
    let json_str = r#"{"_id":1,"name":"Alice","age":30,"email":"alice@example.com","active":true}"#;

    c.bench_function("document_from_json", |b| {
        b.iter(|| Document::from_json(black_box(json_str)).unwrap());
    });
}

// ========== STORAGE BENCHMARKS ==========

fn bench_storage_write(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("bench.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();

    c.bench_function("storage_write_1kb", |b| {
        let data = vec![0u8; 1024];
        let coll = db.collection("bench").unwrap();

        b.iter(|| {
            let mut fields = HashMap::new();
            fields.insert("data".to_string(), json!(data.clone()));
            black_box(coll.insert_one(fields).unwrap());
        });
    });
}

fn bench_storage_write_varying_sizes(c: &mut Criterion) {
    let mut group = c.benchmark_group("storage_write_sizes");

    for size in [100, 1_000, 10_000, 100_000].iter() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("bench.mlite");
        let db = DatabaseCore::open(&db_path).unwrap();
        let coll = db.collection("bench").unwrap();

        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let data = vec![0u8; size];
            b.iter(|| {
                let mut fields = HashMap::new();
                fields.insert("data".to_string(), json!(data.clone()));
                black_box(coll.insert_one(fields.clone()).unwrap());
            });
        });
    }
    group.finish();
}

// ========== CRUD BENCHMARKS ==========

fn bench_insert_one(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("bench.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();
    let coll = db.collection("users").unwrap();

    c.bench_function("insert_one", |b| {
        let mut counter = 0;
        b.iter(|| {
            let mut fields = HashMap::new();
            fields.insert("name".to_string(), json!(format!("User{}", counter)));
            fields.insert("age".to_string(), json!(counter % 100));
            counter += 1;
            black_box(coll.insert_one(fields).unwrap());
        });
    });
}

fn bench_find_all(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("bench.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();
    let coll = db.collection("users").unwrap();

    // Pre-populate with 1000 documents
    for i in 0..1000 {
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), json!(format!("User{}", i)));
        fields.insert("age".to_string(), json!(i % 100));
        coll.insert_one(fields).unwrap();
    }

    c.bench_function("find_all_1000_docs", |b| {
        b.iter(|| {
            let query = json!({});
            black_box(coll.find(&query).unwrap());
        });
    });
}

fn bench_find_with_filter(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("bench.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();
    let coll = db.collection("users").unwrap();

    // Pre-populate
    for i in 0..1000 {
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), json!(format!("User{}", i)));
        fields.insert("age".to_string(), json!(i % 100));
        fields.insert("active".to_string(), json!(i % 2 == 0));
        coll.insert_one(fields).unwrap();
    }

    c.bench_function("find_filtered_1000_docs", |b| {
        b.iter(|| {
            let query = json!({"age": {"$gte": 25}});
            black_box(coll.find(&query).unwrap());
        });
    });
}

fn bench_count_documents(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("bench.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();
    let coll = db.collection("users").unwrap();

    // Pre-populate
    for i in 0..1000 {
        let mut fields = HashMap::new();
        fields.insert("age".to_string(), json!(i % 100));
        coll.insert_one(fields).unwrap();
    }

    c.bench_function("count_documents_1000_docs", |b| {
        b.iter(|| {
            let query = json!({"age": {"$gt": 50}});
            black_box(coll.count_documents(&query).unwrap());
        });
    });
}

fn bench_update_one(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("bench.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();
    let coll = db.collection("users").unwrap();

    // Pre-populate
    for i in 0..100 {
        let mut fields = HashMap::new();
        fields.insert("name".to_string(), json!(format!("User{}", i)));
        fields.insert("score".to_string(), json!(0));
        coll.insert_one(fields).unwrap();
    }

    c.bench_function("update_one_100_docs", |b| {
        let mut counter = 0;
        b.iter(|| {
            let query = json!({"name": format!("User{}", counter % 100)});
            let update = json!({"$inc": {"score": 1}});
            counter += 1;
            black_box(coll.update_one(&query, &update).unwrap());
        });
    });
}

fn bench_delete_one(c: &mut Criterion) {
    c.bench_function("delete_one_tombstone", |b| {
        b.iter_batched(
            || {
                // Setup: create fresh DB with 100 docs
                let temp_dir = TempDir::new().unwrap();
                let db_path = temp_dir.path().join("bench.mlite");
                let db = DatabaseCore::open(&db_path).unwrap();
                let coll = db.collection("users").unwrap();

                for i in 0..100 {
                    let mut fields = HashMap::new();
                    fields.insert("id".to_string(), json!(i));
                    coll.insert_one(fields).unwrap();
                }
                (temp_dir, db)
            },
            |(temp_dir, db)| {
                let coll = db.collection("users").unwrap();
                let query = json!({"id": 50});
                black_box(coll.delete_one(&query).unwrap());
                drop(temp_dir);
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

// ========== QUERY BENCHMARKS ==========

fn bench_complex_query(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("bench.mlite");
    let db = DatabaseCore::open(&db_path).unwrap();
    let coll = db.collection("users").unwrap();

    // Pre-populate
    for i in 0..1000 {
        let mut fields = HashMap::new();
        fields.insert("age".to_string(), json!(i % 100));
        fields.insert("city".to_string(), json!(["NYC", "LA", "SF"][i % 3]));
        fields.insert("active".to_string(), json!(i % 2 == 0));
        coll.insert_one(fields).unwrap();
    }

    c.bench_function("complex_query_and_or", |b| {
        b.iter(|| {
            let query = json!({
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
            });
            black_box(coll.find(&query).unwrap());
        });
    });
}

// Group all benchmarks
criterion_group!(
    benches,
    bench_document_creation,
    bench_document_serialization,
    bench_document_deserialization,
    bench_storage_write,
    bench_storage_write_varying_sizes,
    bench_insert_one,
    bench_find_all,
    bench_find_with_filter,
    bench_count_documents,
    bench_update_one,
    bench_delete_one,
    bench_complex_query,
);

criterion_main!(benches);
