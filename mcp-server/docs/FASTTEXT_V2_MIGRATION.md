# FastText .ironbase.v2 Format — Migration Guide

## Summary

Version 1.0.329 introduces the `.ironbase.v2` model format with **subword (n-gram) support**. This enables meaningful embeddings for out-of-vocabulary (OOV) words — compound nouns, abbreviations, typos — that previously received zero vectors.

| | v1 (.ironbase.bin) | v2 (.ironbase.v2.bin) |
|---|---|---|
| Known words | Word vector | Word vector (FastText formula) |
| OOV words | **Zero vector** | **Subword n-gram average** |
| File size (hu 300d) | ~2.3 GB | ~4.5 GB |
| Subword buckets | No | 2M buckets |
| Backward compatible | N/A | Yes (auto-detected) |

### Impact

| Query type | v1 result | v2 result |
|---|---|---|
| `embed_text("fékpad")` | Valid vector (known word) | Same vector |
| `embed_text("fékerőmérő")` | **Zero vector** (OOV) | Valid vector from subwords |
| `embed_text("PEF")` | **Zero vector** (OOV) | Valid vector from subwords |
| `rag_search("lengéscsillapító")` | **No vector match** | Semantic match works |
| `hybrid_search(...)` | Fulltext only for OOV | Fulltext + vector |

Estimated **30-40% of Hungarian domain queries** were affected by the OOV zero-vector problem.

---

## Prerequisites

- IronBase MCP Server v1.0.329+
- Converted model file: `cc.hu.300.ironbase.v2.bin` (~4.5 GB)
- Sufficient disk space for the model + re-embedding

## Step 1: Convert the Model (one-time)

If you don't already have the v2 model file:

```bash
cd /path/to/IronBase
python3 models/convert_bin_to_ironbase_v2.py \
    models/cc.hu.300.bin \
    models/cc.hu.300.ironbase.v2.bin
```

- Input: original FastText `cc.hu.300.bin` (6.8 GB)
- Output: `cc.hu.300.ironbase.v2.bin` (~4.5 GB)
- Runtime: ~10 minutes
- Memory: ~200 MB (streaming architecture)

## Step 2: Update Server Configuration

Change the `IRONBASE_FASTTEXT_MODEL` environment variable to point to the v2 file:

```bash
# Before
export IRONBASE_FASTTEXT_MODEL=/path/to/models/cc.hu.300.ironbase.bin

# After
export IRONBASE_FASTTEXT_MODEL=/path/to/models/cc.hu.300.ironbase.v2.bin
```

For systemd service:
```ini
# /etc/systemd/system/mcp-ironbase.service
[Service]
Environment=IRONBASE_FASTTEXT_MODEL=/path/to/models/cc.hu.300.ironbase.v2.bin
```

For Claude Desktop (`claude_desktop_config.json`):
```json
{
  "mcpServers": {
    "ironbase": {
      "command": "/path/to/mcp-ironbase-server",
      "args": ["--stdio"],
      "env": {
        "IRONBASE_PATH": "/path/to/database.mlite",
        "IRONBASE_FASTTEXT_MODEL": "/path/to/models/cc.hu.300.ironbase.v2.bin"
      }
    }
  }
}
```

## Step 3: Restart the Server

```bash
# Graceful stop
kill -SIGTERM $(pgrep mcp-ironbase)
sleep 5

# Start with v2 model
IRONBASE_FASTTEXT_MODEL=/path/to/models/cc.hu.300.ironbase.v2.bin \
IRONBASE_PATH=/path/to/database.mlite \
./mcp-ironbase-server
```

The server auto-detects the v2 format. Look for this log line at startup:
```
Loading FastText v2 model: vocab_size=2000000, dim=300, buckets=2000000, minn=5, maxn=5
```

## Step 4: Re-embed Existing RAG Collections

**This is required** for existing RAG collections. Without re-embedding, stored document vectors (v1) won't match query vectors (v2) for OOV words.

### Option A: Backfill via MCP tool (recommended)

For each RAG collection that has auto-embedding enabled:

```json
{
  "name": "auto_embed_backfill",
  "arguments": {
    "collection": "your_collection_name"
  }
}
```

This re-generates embeddings for all documents using the new v2 model. It runs as a background job — check progress with:

```json
{"name": "embed_job_list"}
```

### Option B: Re-import documents

If you prefer a clean re-index:

1. Export documents (or use original source files)
2. Drop and recreate the RAG collection:
   ```json
   {"name": "collection_drop", "arguments": {"collection": "your_collection"}}
   {"name": "rag_collection_create", "arguments": {"collection": "your_collection", "source_field": "content"}}
   ```
3. Re-import documents:
   ```json
   {"name": "rag_document_import", "arguments": {"collection": "your_collection", "content": "...", "metadata": {}}}
   ```

### Which collections need re-embedding?

| Collection type | Needs re-embedding? | How to check |
|---|---|---|
| RAG collections (with HNSW) | **Yes** | `rag_collection_stats` shows vector index |
| Auto-embed enabled | **Yes** | `auto_embed_status` returns config |
| Manual vector fields | **Yes** | Documents have `*_embedding` fields |
| No vector indexes | **No** | Fulltext/B+tree only |

### Verification

After re-embedding, test OOV queries:

```json
{
  "name": "rag_search",
  "arguments": {
    "collection": "your_collection",
    "query": "fékerőmérő"
  }
}
```

If the vector component returns results (check `_vector_rank` in metadata), the migration is successful.

## Step 5: (Optional) Remove v1 Model

Once all collections are re-embedded and verified:

```bash
rm models/cc.hu.300.ironbase.bin  # ~2.3 GB freed
```

Keep `cc.hu.300.bin` (the original FastText binary) as source for future reconversions.

---

## Technical Details

### .ironbase.v2 File Format

```
Header (32 bytes):
  magic:         [u8; 4]  = b"IBv2"
  dim:           u32      = 300
  vocab_size:    u32      = 2,000,000
  bucket_count:  u32      = 2,000,000
  minn:          u32      = 5
  maxn:          u32      = 5
  bucket_offset: u64      = byte offset to bucket section

Word Section (byte 32 .. bucket_offset):
  [word_bytes\0][f32 x dim] x vocab_size

Bucket Section (bucket_offset .. EOF):
  [f32 x dim] x bucket_count
```

### OOV Algorithm

For unknown word "fekeroemero":
1. Pad: `<fekeroemero>`
2. Extract 5-grams: `<feke`, `feker`, `ekero`, `keroe`, ...
3. Hash each: `FNV-1a(ngram) % 2,000,000` -> bucket_id
4. Read bucket vector from mmap (on-demand, no full load)
5. Average all n-gram vectors -> OOV word vector

### Format Auto-Detection

The loader reads the first 4 bytes:
- `b"IBv2"` -> v2 loading path (32-byte header + subword support)
- Anything else -> v1 loading path (8-byte header, no subwords)

Both formats are fully supported. No code changes needed for v1 users.

### Memory Usage

| Component | v1 | v2 |
|---|---|---|
| Word index (RAM) | ~50 MB | ~50 MB |
| Vectors (mmap) | 2.3 GB virtual | 4.5 GB virtual |
| Actual RAM | On-demand pages | On-demand pages |
| OOV computation | N/A | ~5-20 us per word |

The 4.5 GB file is memory-mapped — only accessed pages are loaded into physical RAM.

---

## Rollback

To revert to v1:

```bash
export IRONBASE_FASTTEXT_MODEL=/path/to/models/cc.hu.300.ironbase.bin
# Restart server
```

Note: if you already re-embedded collections with v2, the stored vectors are valid (just computed differently). Re-embedding again with v1 is not necessary unless you want exact v1 behavior.

---

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `Loading FastText v1 model` in logs | Wrong model path | Check `IRONBASE_FASTTEXT_MODEL` |
| OOV words still zero vector | Using v1 model | Verify v2 magic in log: `Loading FastText v2 model` |
| `rag_search` returns no vector results | Stale embeddings | Run `auto_embed_backfill` |
| Converter OOM | Old converter version | Use streaming converter (v1.0.329+) |
| Server startup slow | First mmap page faults | Normal for first access (~30s for 4.5 GB) |
