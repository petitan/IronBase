# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### RAG Pipeline (mcp-server v1.0.494 – v1.0.501)

Comprehensive engineering overview: [`docs/RAG_PIPELINE.md`](docs/RAG_PIPELINE.md).

#### Breaking (v1.0.501)
- **`fulltext_search` response shape is now flat (#68)**, consistent with `hybrid_search`. Document fields move to the top level; engine metadata uses `_`-prefixed keys to avoid colliding with user fields.
  - Before: `{"document": {"_id":1,"title":"T",...}, "score": 6.2, "matched_tokens": [...], "highlights": ...}`
  - After: `{"_id":1,"title":"T",..., "_score": 6.2, "_matched_tokens": [...], "_highlights": ...}`
  - Migration: `hit.document.X → hit.X`, `hit.score → hit._score`, `hit.matched_tokens → hit._matched_tokens`, `hit.highlights → hit._highlights`. The `hybrid_search` parser works for both tools now.
- **`fuzzy_search` response shape is now flat (review follow-up)**, same shape as `fulltext_search`. Hit keys: `<doc fields>`, `_score`, `_matched_value`, `_highlight?`. Brings the entire fulltext-style tool suite to one parser.
- **Rhai `db_fulltext_search`, `db_fuzzy_search`, `db_vector_search` and `db_vector_search_filter` mirror the flat shape** (review follow-up). Same MCP↔Rhai consistency pattern as #66.
- **`vector_search` MCP response confirmed flat across all readers**: MCP `vector_search` already emitted the flat shape (`<doc fields>, _score`); the TUI vector state/renderer was silently reading the pre-flat keys (`document`/`distance` → always null/0.0) — fixed (`VectorSearchResult.distance` renamed to `score`, reads `_score` from the flat hit).
- **`tuti-tui` updated** to consume the flat `fulltext_search` shape (`ironbase-tui/src/modals/fulltext.rs`): reads `_score` and the flat result; the doc-preview helper now also skips `_`-prefixed engine metadata.

#### Hardening (review follow-up, v1.0.501)
- **`GROUP_RESERVED_KEYS` widened** to cover chunk-level engine and structural fields (`chunk_index`, `chunk_total`, `start_char`, `end_char`, `table_header`, `chunk_merged`, `chunks_in_merge`, `_text_score`, `_vector_score`, `_rrf_score`, `_final_score`, `_rerank_boost`, `_vector_rank`, `_text_rank`) **plus the default chunk payload (`content`, `embedding`)** — duplicate ingest can leave every chunk with identical content/embedding; promoting either to the group root would bloat the response (embedding) or mislabel chunk text as doc-level (content). Previously only the four group-root keys were reserved; a coincidentally identical chunk-level field (e.g. all chunks tied on `_text_score`, or every chunk carrying `chunk_merged=true` after a merge run) could lift to the group root, falsely promoting chunk semantics to document semantics. Now those keys stay in `chunks[i]` regardless of value identity. Regression test added.
- **Rhai `db_hybrid_search` now honors `group_by_document` and `match_scope`** — feature parity with the MCP `hybrid_search` tool: STEP 1.5 qualification via the shared `fusion::apply_document_qualification`, Phase 1+2 grouped builder with `lift_common_fields`, and per-chunk `_text_score` annotation. Phase 2 also emits a `tracing::warn!` when a chunk lacks `doc_id` (matching MCP) instead of silently dropping. The Rhai impl returns a `Vec<Dynamic>` of group objects (`{doc_id, best_score, chunk_count, <lifted doc fields>, chunks: [...]}`) when `group_by_document: true`. CLAUDE.md's existing documentation of these options now matches behavior.
- **Stale HTML/Markdown docs updated** (`docs/en/ironbase-guide.html`, `docs/hu/ironbase-guide.html`): the fuzzy/fulltext_search response-shape examples (which were always inaccurate via a `doc` envelope key) now show the flat v1.0.501+ shape. `docs/hu/README.md`'s Python-binding tuple shape is unchanged — that's the PyO3 surface, not MCP.
- **`hybrid_search group_by_document=true` lifts doc-level fields to the group root (#69)**. Keys whose values are identical across ALL chunks of a group (e.g. `title`, `customer`, `year`, `date`, `doc_type`) are moved to the group top level and removed from each `chunks[i]`. Chunk-specific keys (`content`, `_id`, `chunk_index`, `embedding`, `_text_score`, ...) naturally vary across chunks and stay in `chunks[i]`. Generic, no hardcoded field list.
  - **Single-chunk groups are NOT lifted** (review hardening): with one chunk, "all match" is vacuously true on every key; lifting would empty `chunks[0]`. Gated → single-chunk groups keep `chunks[0]` self-contained.
  - **Engine-reserved group-root keys** (`doc_id`, `best_score`, `chunk_count`, `chunks`) are never lifted (review hardening), so a user field that happens to match one of those names cannot overwrite engine metadata at the group root.
  - Migration: `group.chunks[0].title → group.title` for multi-chunk groups (single-chunk groups still expose doc-level fields via `chunks[0].title`).
  - Flat (non-grouped) mode is unchanged.

#### Stale documentation updates (v1.0.501)
- `mcp-server/src/prompts/search.rs` — the `fulltext-search` and fuzzy-search prompt "Response Format" examples updated to the flat shape so LLMs/clients are taught the current contract.

#### Added
- **Contextual chunk embedding** (v1.0.494): `build_embed_text(body, section_path)` prepends a section breadcrumb and flattens markdown tables on the embedded text only; the stored chunk text stays an untouched slice of the source. `strip_markdown_tables` is wired into the embedding path. `db_rag_import` field set now matches `rag_document_import` (`section_path`, `heading_level`).
- **Fenced-code-aware heading detection** (v1.0.494): `# comment` inside ` ``` ` blocks no longer pollutes `section_path`.
- **Idempotent chunk import (#67, v1.0.495)**: new `if_exists` parameter on `rag_document_import` / `embed_document` / Rhai `db_rag_import` — `replace` (default) | `skip` | `error` | `append`. Shared `insert_chunks_idempotent` helper (`helpers.rs`) with safe ordering — captures existing chunk ids via `_id` projection, inserts new, deletes old → never loses good data on partial failure. `should_skip_before_embedding` pre-check skips the expensive embedding call when `skip`/`error` would short-circuit (v1.0.498).
- **Deterministic merge-field resolution (#64, v1.0.496)**: `pick_text_field` resolves the hybrid-search merge field deterministically (content-preferred, lexicographic fallback) — fixes a HashMap-order bug that occasionally caused `merge_adjacent_chunks` to concatenate `title` instead of `content`.
- **Configurable fulltext language (#65, v1.0.497)**: `language` parameter on `rag_document_import` + Rhai `db_rag_import` (default `"none"`, Snowball stemmers for hungarian/english/german). `fulltext_analyze` gains optional `collection`+`field` to inherit the real index `FtsOptions` via the new `adapter.get_fulltext_index_options` — the debugger now reflects exactly how that index tokenizes (`inherited_from_index` response flag).
- **Markdown table header propagation (#63, v1.0.499)**: table-continuation chunks get the `<header>\n<separator>` block prepended (stored in a new `table_header` chunk field), so every retrieved table chunk is self-interpretable. Detection runs on the raw (overlap-free) slice (production default `overlap=100` works). `merge_adjacent_chunks` strips the duplicated header via the stored field — exact, CRLF-safe. `current_table_header` resets on heading change.
- **Multi-field fulltext indexing (#66, v1.0.500)**: new `text_fields` parameter on `rag_collection_create` + `rag_document_import` + Rhai `db_rag_create`/`db_rag_import` — creates an FTS index on every listed field (primary `text_field` always included, deduplicated). `RagConfig` gains `text_fields: Vec<String>` (`#[serde(default)]` → backward-compatible). `hybrid_search` (MCP and Rhai `db_hybrid_search`) defaults to the configured multi-field set when the caller omits `text_fields`. `rag_document_import` auto-creates a `RagConfig` when none exists, so the default applies to import-only workflows too. `rag_collection_stats` reports `text_fields`.

#### Hardened (code review follow-ups)
- `embed_document` now filters `RESERVED_METADATA_KEYS` like `rag_document_import` — user-supplied `metadata.doc_id` can no longer override the chunk's `doc_id` and silently defeat #67 idempotency (v1.0.498). `RESERVED_METADATA_KEYS` consolidated into a single shared definition (`helpers.rs`); `table_header` added.
- `rag_document_import` `language_ignored` warning only fires when the requested language *differs* from the stored config (no false positive on repeated imports with identical arguments) (v1.0.500).
- Multi-field default search intersects with the actually-indexed fields (`resolve_search_text_fields`) — a configured-but-unindexed field (failed creation / later `index_drop`) no longer hard-errors `fulltext_search_multi`; it is silently excluded from the default set. Explicit `text_fields` from the caller is honored as-is (v1.0.500).
- Rhai layer brought in line with MCP: `get_rag_config` carries `text_fields`, `db_hybrid_search` and `db_rag_stats` are consistent with the MCP equivalents (v1.0.500).
- `auto_embed_enable` warns when applied to a chunk-imported collection (verbatim-embed boundary made visible, not silently degraded) (v1.0.494).

### Fixed
- **MCP System Collections Hidden Flag**: `ensure_system_collections()` now fixes flags on existing collections
  - Legacy databases where `_system.scripts` was created before flag system now get `hidden: true`
  - Auto-migrates on MCP server startup - no manual intervention needed

### Added
- **Cross-Process File Locking**: Prevents database corruption from multiple processes
  - Uses `fs2` crate for OS-level exclusive file locks
  - Non-blocking `try_lock_exclusive()` - returns error immediately if locked (deadlock-free)
  - Automatic lock release on process exit or crash
  - New `DatabaseLocked` error type for clear error messages
- **Explicit `close()` Method**: Release file lock without waiting for GC/Drop
  - `DatabaseCore::close()` - flush and release lock for immediate reopening
  - Critical for Python/C# where GC timing is unpredictable
  - Fixes Windows CI test failures with file persistence tests
- **Safe Hot Backup**: Snapshot isolation for consistent backups from running databases
  - Shared lock only during metadata read (~1ms) - DB continues operating
  - Append-only storage guarantees data immutability during backup
  - `concurrent_writes` flag indicates if DB was modified during backup
  - New data automatically included in next incremental backup
- Full GitHub publication readiness (CONTRIBUTING, CODE_OF_CONDUCT, SECURITY)
- CI/CD workflows for all platforms
- **Compound Index Prefix Queries**: Queries on first field of compound index now use index
  - `{"country": "HU"}` uses `(country, city)` compound index via range scan
  - New `IndexKey::MaxKey` sentinel for upper bounds
  - `build_prefix_range()` method for compound prefix bounds
  - `IndexPrefixInfo` struct for QueryPlanner compound awareness
- **Explicit Collection Existence Checks**: `get_collection()` now returns error if collection doesn't exist
  - Prevents typos from creating empty collections (e.g., `find("uusers")`)
  - Insert operations still create collections implicitly (MongoDB-compatible)
  - New `CollectionNotFound` error type propagates through MCP layer

### Changed
- **Performance Optimization**: `get_collection()` now uses READ locks only
  - Hot path reduced from 4 locks (incl. WRITE) to 2 READ locks
  - New `with_shared_indexes_readonly()` method for existing collections
  - Significantly reduces lock contention under high concurrency
  - Storage WRITE lock no longer needed for read operations

### Fixed
- **MCP `collection_create` tool**: Now actually creates the collection
  - Previously returned success without performing any operation
  - Root cause: tool handler was a no-op, relying on implicit creation that never happened

### Removed
- **FileStorage**: Removed deprecated wrapper around StorageEngine
  - Was redundant layer with no added functionality
  - Use `StorageEngine` directly instead

## [1.0.5] - 2025-12-06

### Added
- **Fuzzy Text Search**: New `$fuzzy` query operator with multiple algorithms
  - Jaro-Winkler (default, fastest)
  - Levenshtein (most accurate)
  - Damerau-Levenshtein (best for typos/OCR)
  - Configurable similarity threshold (0.0-1.0)
- **Fuzzy Indexes**: `create_fuzzy_index()` for optimized fuzzy searches
  - Accelerates `$fuzzy` queries with pre-computed similarity
  - Supports all three algorithms
  - Configurable threshold per index
- **Pagination Enhancement**: `include_total` parameter for FindOptions
  - Returns total document count alongside results
  - Enables "Showing 1-10 of 100 results" UI patterns
- **MCP Tools**: New fuzzy search tools
  - `fuzzy_search` - Execute fuzzy text queries
  - `index_create_fuzzy` - Create fuzzy text indexes
- **MCP Server Improvements**
  - Configurable max body size with human-readable format
  - `-Update` switch for easy service updates (Windows)
  - Improved Windows install experience
  - CRLF to LF normalization in config parsing

### Fixed
- Windows configuration file line ending issues

## [0.3.0] - 2025-11-26

### Added
- **C# Bindings**: Complete .NET bindings with MongoDB-like API
  - NuGet package support
  - Cursor/Streaming API for large datasets
  - Collection-level transaction support
  - Schema validation
  - Logging API
- **Nested Documents**: Full dot notation support
  - Query nested fields: `{"address.city": "NYC"}`
  - Update nested fields with all operators
  - Index nested fields for fast lookups
- **Array Operations**: MongoDB-style array element matching
  - `$size` operator for array length queries
  - Array element queries with dot notation
- **MCP Server**: Model Context Protocol integration
  - DOCJL document editing support
  - 15+ prompts for AI-assisted operations
  - Resources and tools endpoints
- **Testing Infrastructure**
  - 5-hour fuzzing corpus (~367M iterations)
  - Chaos testing framework
  - 140+ C# tests
  - 31 nested document tests
- **In-Memory Mode**: `DatabaseCore::open_memory()` for testing

### Fixed
- Integer overflow in `$sum` aggregation
- `$inc` now creates non-existent fields
- Dot notation for nested field indexes on persistence reload
- UTF-8 character preservation in MCP bridge

### Changed
- Modularized `collection_core` module structure
- Improved documentation with dot notation examples

## [0.2.0] - 2025-11-10

### Added
- **Query Operators**: Complete MongoDB-compatible query system
  - Comparison: `$eq`, `$ne`, `$gt`, `$gte`, `$lt`, `$lte`, `$in`, `$nin`
  - Logical: `$and`, `$or`, `$not`, `$nor`
  - Element: `$exists`, `$type`
  - Array: `$all`, `$elemMatch`
  - Regex: `$regex`
- **Update Operators**
  - `$set`, `$unset`, `$inc`
  - `$push`, `$pull`, `$addToSet`, `$pop`
- **Aggregation Pipeline**
  - Stages: `$match`, `$group`, `$project`, `$sort`, `$limit`, `$skip`
  - Accumulators: `$sum`, `$avg`, `$min`, `$max`, `$first`, `$last`
- **B+ Tree Indexing**
  - Unique and non-unique indexes
  - Compound indexes
  - Query optimizer with explain()
- **FindOptions**: Projection, sort, limit, skip
- **Transactions**: ACD (Atomicity, Consistency, Durability) with WAL
- **Durability Modes**: Safe, Batch, Unsafe
- **JSON Schema Validation**
- **Query Cache**: LRU cache for repeated queries
- **Python Bindings**: Complete PyO3-based Python API

### Changed
- Strategy pattern for query operators (83% complexity reduction)
- Improved storage engine with compaction support

## [0.1.0] - 2025-11-01

### Added
- Initial release
- Core CRUD operations: `insert_one`, `insert_many`, `find`, `find_one`, `update_one`, `update_many`, `delete_one`, `delete_many`
- Append-only storage engine
- Document catalog with offset tracking
- Basic query matching
- WAL (Write-Ahead Log) for crash recovery
- MIT License

[Unreleased]: https://github.com/petitan/IronBase/compare/v1.0.55...HEAD
[1.0.5]: https://github.com/petitan/IronBase/compare/v0.3.0...v1.0.5
[0.3.0]: https://github.com/petitan/IronBase/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/petitan/IronBase/releases/tag/v0.2.0
[0.1.0]: https://github.com/petitan/IronBase/releases/tag/v0.1.0
