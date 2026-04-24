//! Központi limit konstansok az IronBase-hez
//!
//! Ez a modul tartalmazza az összes hardcoded limitet egy helyen,
//! hogy könnyen áttekinthető és módosítható legyen.
//!
//! # Kategóriák
//!
//! - **Storage**: Dokumentum és metadata méret limitek
//! - **WAL**: Write-Ahead Log entry limitek
//! - **Index**: B+ tree és index építési limitek
//! - **Query**: Cache és regex limitek
//! - **Cursor**: Batch méret alapértelmezések
//! - **Recursion**: DoS védelem mélységi limitek
//! - **Transaction**: Timeout értékek
//!
//! # Használat
//!
//! ```rust,ignore
//! use ironbase_core::limits::{MAX_DOCUMENT_SIZE_BYTES, INDEX_BUILD_BATCH_SIZE};
//! ```

// ============================================================================
// Storage Limits
// ============================================================================

/// Maximum dokumentum méret bájtokban (64 MB)
///
/// Biztonsági limit a túl nagy dokumentumok ellen.
/// Ha egy dokumentum meghaladja ezt a méretet, az insert elutasításra kerül.
pub const MAX_DOCUMENT_SIZE_BYTES: usize = 64 * 1024 * 1024;

/// Maximum metadata méret bájtokban (64 MB)
///
/// Collection metadata (index definíciók, stb.) maximális mérete.
/// DoS védelem a túlzottan nagy metadata ellen.
pub const MAX_METADATA_SIZE: usize = 64 * 1024 * 1024;

// ============================================================================
// WAL (Write-Ahead Log) Limits
// ============================================================================

/// WAL entry header méret bájtokban
///
/// Struktúra: 8 (tx_id) + 1 (type) + 4 (len) = 13 byte
pub const WAL_HEADER_SIZE: usize = 13;

/// Maximum WAL entry méret bájtokban (64 MB)
///
/// Biztonsági limit a túl nagy WAL bejegyzések ellen.
pub const MAX_WAL_ENTRY_SIZE: usize = 64 * 1024 * 1024;

// ============================================================================
// Index Limits
// ============================================================================

/// Maximum kulcsok száma B+ tree node-onként
///
/// Ha egy node meghaladja ezt a számot, split történik.
/// 16KB page mérettel 128 kulcs optimális.
pub const MAX_KEYS_PER_NODE: usize = 128;

/// B+ tree node page méret bájtokban (16 KB)
///
/// Nagyobb page = kevesebb I/O, de több memória.
pub const NODE_PAGE_SIZE: usize = 16384;

/// Index építési batch méret (dokumentumok száma)
///
/// Ennyi dokumentumot dolgoz fel egyszerre az index építéskor.
/// Nagyobb érték = gyorsabb, de több memória.
pub const INDEX_BUILD_BATCH_SIZE: usize = 1000;

/// Fulltext index építési batch méret
///
/// Kisebb, mint az általános batch, mert a fulltext indexelés
/// memória-intenzívebb (tokenizálás, stemming).
pub const FULLTEXT_INDEX_BUILD_BATCH_SIZE: usize = 100;

/// Index újraépítési batch méret
///
/// Kisebb batch a biztonságosabb újraépítéshez.
pub const INDEX_REBUILD_BATCH_SIZE: usize = 100;

/// Minimum bejegyzések száma histogram építéshez
///
/// Az index statisztikák (selectivity estimation) csak ennyi
/// bejegyzés felett épülnek, mert kisebb indexeknél nem éri meg.
pub const MIN_ENTRIES_FOR_HISTOGRAM: usize = 100_000;

// ============================================================================
// Query & Cache Limits
// ============================================================================

/// Query cache kapacitás (LRU cache méret)
///
/// Maximum ennyi query eredményt tárol a cache.
/// LRU eviction történik, ha megtelik.
pub const QUERY_CACHE_CAPACITY: usize = 1000;

/// Regex pattern cache kapacitás
///
/// Maximum ennyi lefordított regex mintát tárol.
/// Megakadályozza a memória-bloat-ot sok regex query esetén.
pub const REGEX_CACHE_CAPACITY: usize = 100;

/// Default fuzzy keresés küszöbérték (Jaro-Winkler similarity)
///
/// 0.0 = minden egyezik, 1.0 = pontos egyezés szükséges.
/// 0.8 = ~80% hasonlóság szükséges.
pub const DEFAULT_FUZZY_THRESHOLD: f64 = 0.8;

/// Heap-based pagination küszöb
///
/// Ha `skip + limit < HEAP_PAGINATION_THRESHOLD`, akkor heap-alapú
/// O(n log k) partial sort fut a teljes O(n log n) sort helyett.
/// Ez 10x gyorsabb nagy collection-ökön kis limit esetén.
pub const HEAP_PAGINATION_THRESHOLD: usize = 10_000;

/// Nagy query figyelmeztetési küszöb
///
/// Ha egy query ennél több dokumentumot tölt be, warning log íródik.
/// Segít azonosítani a teljesítményproblémákat.
pub const LARGE_QUERY_WARNING_THRESHOLD: usize = 10_000;

// ============================================================================
// Cursor & Batch Limits
// ============================================================================

/// Default cursor batch méret
///
/// Ennyi dokumentumot tölt be egyszerre a cursor iteráláskor.
pub const DEFAULT_CURSOR_BATCH_SIZE: usize = 100;

/// Maximum batch memória méret bájtokban (10 MB)
///
/// Batch műveleteknél (insert_many) korai flush történik,
/// ha a buffer meghaladja ezt a méretet.
pub const MAX_BATCH_MEMORY_BYTES: usize = 10 * 1024 * 1024;

/// Párhuzamos feldolgozás chunk mérete
///
/// Rayon parallel iterátoroknál ennyi dokumentumot dolgoz fel
/// egyszerre egy szálon. ~500MB memória chunk-onként.
pub const PARALLEL_CHUNK_SIZE: usize = 1000;

// ============================================================================
// Recursion & DoS Protection Limits
// ============================================================================

/// Maximum rekurzió mélység ($** operátorhoz)
///
/// Megakadályozza a stack overflow-t mélyen beágyazott dokumentumoknál.
pub const MAX_RECURSION_DEPTH: usize = 100;

// ============================================================================
// Transaction Limits
// ============================================================================

/// Default write lock timeout másodpercekben
///
/// Ennyi ideig vár a tranzakció az exkluzív írási zárra.
pub const DEFAULT_WRITE_LOCK_TIMEOUT_SECS: u64 = 5;

/// Lock polling intervallum milliszekundumokban
///
/// Ennyi időnként próbálja meg újra a lock megszerzését.
pub const LOCK_POLL_INTERVAL_MS: u64 = 10;

// ============================================================================
// Recovery Limits (task #26 WAL replay)
// ============================================================================

/// Maximum recovered WAL operations per collection that may be WAL-replayed
/// into the loaded non-btree indexes on dirty shutdown. Above this count,
/// `try_wal_replay_for_collection` returns `Ok(false)` and recovery falls
/// back to rebuild-from-catalog.
///
/// Rationale: each recovered `Operation::Insert` / `Update` carries its full
/// document JSON; the collected `Vec<(tx_id, Op)>` therefore scales with
/// `ops × avg_doc_size`. At 10 KB average doc size, a 100 000 op cap yields
/// ~1 GB of working memory per collection during replay. Very large
/// accumulated WALs are almost certainly faster to recover via the
/// sequential catalog scan anyway, so the threshold caps blast radius
/// without losing a meaningful speedup.
///
/// Tune downward on memory-constrained systems; tune upward only after
/// verifying that `avg_doc_size × threshold` still fits comfortably in RAM
/// alongside the catalog, the loaded indexes, and other collections'
/// replay buffers.
pub const MAX_REPLAY_OPS_PER_COLLECTION: usize = 100_000;

// ============================================================================
// Cancellation Check Frequency
// ============================================================================

/// Cancellation ellenőrzés gyakorisága (dokumentumok száma)
///
/// Minden N dokumentum feldolgozása után ellenőrzi, hogy
/// a művelet törölve lett-e (timeout vagy user cancellation).
pub const CANCELLATION_CHECK_INTERVAL: usize = 100;

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_limits_are_64mb() {
        assert_eq!(MAX_DOCUMENT_SIZE_BYTES, 64 * 1024 * 1024);
        assert_eq!(MAX_METADATA_SIZE, 64 * 1024 * 1024);
        assert_eq!(MAX_WAL_ENTRY_SIZE, 64 * 1024 * 1024);
    }

    #[test]
    fn test_index_limits() {
        assert_eq!(MAX_KEYS_PER_NODE, 128);
        assert_eq!(NODE_PAGE_SIZE, 16384);
        assert_eq!(INDEX_BUILD_BATCH_SIZE, 1000);
    }

    #[test]
    fn test_cache_limits() {
        assert_eq!(QUERY_CACHE_CAPACITY, 1000);
        assert_eq!(REGEX_CACHE_CAPACITY, 100);
    }

    #[test]
    fn test_batch_limits() {
        assert_eq!(DEFAULT_CURSOR_BATCH_SIZE, 100);
        assert_eq!(MAX_BATCH_MEMORY_BYTES, 10 * 1024 * 1024);
    }

    #[test]
    fn test_recursion_limit() {
        assert_eq!(MAX_RECURSION_DEPTH, 100);
    }

    #[test]
    fn test_transaction_limits() {
        assert_eq!(DEFAULT_WRITE_LOCK_TIMEOUT_SECS, 5);
        assert_eq!(LOCK_POLL_INTERVAL_MS, 10);
    }
}
