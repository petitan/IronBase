# TODO: OOM and Memory Return Consistency Review

1. Verify jemalloc OS memory return behavior vs. comments in `mcp-server/src/main.rs`.
2. Audit “jemalloc-aware” try_reserve comments in `ironbase-core/src/collection_core/mod.rs` against actual allocator usage across binaries.
3. Review aggregate throttling logic in `mcp-server/src/adapter.rs` for `$match` without index and sync `aggregate` path.
4. Evaluate preflight `count_documents` in `mcp-server/src/tools/mod.rs` for unintended full-scan/OOM risk.
5. Align `find` default limits across tool API and scripting (`mcp-server/src/tools/helpers.rs` vs `mcp-server/src/scripting/limits.rs`).
