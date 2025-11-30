//! Example: Nested document memory benchmark without Python overhead
//! Run with: `cargo run --example nested_memory_profile --release`

use ironbase_core::{DatabaseCore, StorageEngine};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let db_path = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "nested_memory_profile.mlite".to_string());
    let target_docs: usize = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(25_000);
    let batch_size = 1_000_usize;

    println!("==============================================");
    println!("IronBase Nested Memory Benchmark (Rust only)");
    println!("Database path: {db_path}");
    println!("Inserting {target_docs} documents (batch {batch_size})");
    println!("==============================================");

    report_memory("Start")?;

    cleanup(&db_path)?;
    let db = DatabaseCore::<StorageEngine>::open(&db_path)?;
    let collection = db.collection("users")?;

    let mut inserted = 0usize;
    let insert_begin = Instant::now();
    while inserted < target_docs {
        let mut docs = Vec::with_capacity(batch_size);
        for offset in 0..batch_size {
            let idx = inserted + offset;
            if idx >= target_docs {
                break;
            }
            docs.push(build_nested_doc(idx as u64));
        }

        db.insert_many("users", docs)?;
        inserted += batch_size;

        if inserted.is_multiple_of(5_000) || inserted >= target_docs {
            println!(
                "Inserted {:>6} / {:>6} documents in {:.2?}",
                inserted.min(target_docs),
                target_docs,
                insert_begin.elapsed()
            );
            report_memory("  ↳ current RSS")?;
        }
    }

    report_memory("After inserts")?;

    // Create indexes on nested fields
    println!("Creating nested indexes…");
    collection.create_index("profile.location.city".to_string(), false)?;
    collection.create_index("metrics.login.count".to_string(), false)?;
    report_memory("After nested indexes")?;

    // Run a few direct queries to ensure data access works
    let sample = collection.find_one(&json!({"profile.location.city": "Budapest"}))?;
    println!(
        "Sample Budapest doc: {}",
        sample
            .as_ref()
            .and_then(|doc| doc.get("profile"))
            .and_then(|prof| prof.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("<missing>")
    );

    let heavy = collection.count_documents(&json!({"metrics.login.count": {"$gte": 1000}}))?;
    println!("Heavy login users: {heavy}");
    report_memory("After sample queries")?;

    drop(collection);
    db.flush()?;
    cleanup(&db_path)?;
    report_memory("After cleanup")?;

    Ok(())
}

fn build_nested_doc(id: u64) -> HashMap<String, Value> {
    let cities = [
        ("Budapest", "Hungary"),
        ("Debrecen", "Hungary"),
        ("Prague", "Czech Republic"),
        ("Vienna", "Austria"),
        ("Warsaw", "Poland"),
        ("Berlin", "Germany"),
    ];
    let (city, country) = cities[(id as usize) % cities.len()];

    let theme_colors = ["blue", "green", "purple"];
    let device_types = ["ios", "android", "web"];
    let theme_color = theme_colors[((id / 3) as usize) % theme_colors.len()];
    let device = device_types[(id as usize) % device_types.len()];
    let mut doc = HashMap::new();
    doc.insert("user_id".to_string(), json!(id));
    doc.insert(
        "profile".to_string(),
        json!({
            "name": format!("User_{:05}", id),
            "age": 18 + (id % 50),
            "location": {
                "city": city,
                "country": country,
                "address": {
                    "street": format!("{} {} St", 1 + (id % 200), (b'A' + (id % 26) as u8) as char),
                    "zip": 1000 + (id % 9000)
                },
                "geo": {
                    "lat": (id % 90) as f64 + 0.123,
                    "lng": (id % 180) as f64 + 0.456
                }
            },
            "contacts": {
                "email": format!("user{:_>5}@example.com", id),
                "phones": [
                    format!("+361{:09}", id % 1_000_000_000),
                    format!("+362{:09}", (id * 3) % 1_000_000_000)
                ]
            }
        }),
    );
    doc.insert(
        "metrics".to_string(),
        json!({
            "login": {
                "count": (id % 5_000),
                "last_login": format!("2025-{:02}-{:02}", 1 + (id % 12), 1 + (id % 28))
            },
            "orders": {
                "total_value": (id as f64 * std::f64::consts::PI) % 20_000.0,
                "last_order_value": (id as f64 * 1.1) % 1000.0
            }
        }),
    );
    doc.insert(
        "preferences".to_string(),
        json!({
            "notifications": {
                "email": id.is_multiple_of(2),
                "sms": id.is_multiple_of(3),
                "in_app": true
            },
            "theme": {
                "mode": if id.is_multiple_of(2) { "dark" } else { "light" },
                "color": theme_color
            }
        }),
    );
    doc.insert(
        "sessions".to_string(),
        json!([
            {
                "device": device,
                "ip": format!("10.0.{}.{}", (id % 255), (id * 7) % 255)
            }
        ]),
    );
    doc
}

fn report_memory(label: &str) -> anyhow::Result<()> {
    match current_rss_kb() {
        Ok(rss) => {
            println!("{label}: {:.2} MB RSS", rss as f64 / 1024.0);
        }
        Err(err) => {
            eprintln!("{label}: unable to read RSS ({err})");
        }
    }
    Ok(())
}

fn current_rss_kb() -> io::Result<u64> {
    let mut status = String::new();
    File::open("/proc/self/status")?.read_to_string(&mut status)?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let value = rest.split_whitespace().next().unwrap_or("0");
            return value.parse::<u64>().map_err(io::Error::other);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "VmRSS line not found",
    ))
}

fn cleanup(path: &str) -> anyhow::Result<()> {
    let wal = format!("{path}.wal");
    for target in [path, &wal] {
        if Path::new(target).exists() {
            fs::remove_file(target)?;
        }
    }
    Ok(())
}
