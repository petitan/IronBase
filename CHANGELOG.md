# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Cross-Process File Locking**: Prevents database corruption from multiple processes
  - Uses `fs2` crate for OS-level exclusive file locks
  - Non-blocking `try_lock_exclusive()` - returns error immediately if locked (deadlock-free)
  - Automatic lock release on process exit or crash
  - New `DatabaseLocked` error type for clear error messages
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

## [0.3.0] - 2025-01-XX

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

## [0.2.0] - 2024-12-XX

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

## [0.1.0] - 2024-11-XX

### Added
- Initial release
- Core CRUD operations: `insert_one`, `insert_many`, `find`, `find_one`, `update_one`, `update_many`, `delete_one`, `delete_many`
- Append-only storage engine
- Document catalog with offset tracking
- Basic query matching
- WAL (Write-Ahead Log) for crash recovery
- MIT License

[Unreleased]: https://github.com/petitan/IronBase/compare/v1.0.5...HEAD
[1.0.5]: https://github.com/petitan/IronBase/compare/v0.3.0...v1.0.5
[0.3.0]: https://github.com/petitan/IronBase/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/petitan/IronBase/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/petitan/IronBase/releases/tag/v0.1.0
