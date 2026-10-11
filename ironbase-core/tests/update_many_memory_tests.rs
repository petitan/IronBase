// update_many_memory_tests.rs
// Peak heap growth of update_many relative to the data it modifies (audit
// 2026-10-07 M2: ~8 full copies of every modified document were held until
// the persist phase). Own test binary: it installs a counting allocator.

use ironbase_core::{DatabaseCore, StorageEngine};
use serde_json::{json, Value};
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            let now = CURRENT.fetch_add(layout.size(), Ordering::SeqCst) + layout.size();
            PEAK.fetch_max(now, Ordering::SeqCst);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        CURRENT.fetch_sub(layout.size(), Ordering::SeqCst);
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn update_many_peak_memory_is_bounded() {
    const DOCS: usize = 3000;
    let tmp = tempfile::tempdir().unwrap();
    let db = DatabaseCore::<StorageEngine>::open(tmp.path().join("m2.mlite")).unwrap();
    let body = "x".repeat(2000);
    for i in 0..DOCS {
        let d: HashMap<String, Value> = json!({"i": i, "body": body, "tags": ["a", "b", "c"]})
            .as_object()
            .unwrap()
            .clone()
            .into_iter()
            .collect();
        db.insert_one("c", d).unwrap();
    }
    let data_bytes = DOCS * (body.len() + 64);

    let base = CURRENT.load(Ordering::SeqCst);
    PEAK.store(base, Ordering::SeqCst);
    let (matched, modified) = db
        .update_many("c", &json!({}), &json!({"$set": {"flag": true}}))
        .unwrap();
    let peak_growth = PEAK.load(Ordering::SeqCst) - base;
    assert_eq!((matched, modified), (DOCS as u64, DOCS as u64));

    let ratio = peak_growth as f64 / data_bytes as f64;
    eprintln!(
        "update_many: data {:.1} MB, peak growth {:.1} MB ({ratio:.2}x)",
        data_bytes as f64 / 1e6,
        peak_growth as f64 / 1e6
    );
    // Old + new (needed for the WAL commit, ~2.5x as serde_json values)
    // plus transient per-chunk work: ~3.4x. Before the fix: ~12.8x
    assert!(
        ratio < 4.5,
        "update_many held {ratio:.2}x the modified data"
    );
}
