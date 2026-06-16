//! Index management tool definitions
//!
//! Tools: index_create (type: btree|fulltext|fuzzy|vector), index_list (optional
//!        type filter), index_drop (any subtype), index_stats (per type),
//!        index_stats_refresh, fulltext_search, fulltext_analyze, fuzzy_search,
//!        explain, find_with_hint

use super::common::{fields, schemas};
use serde_json::{json, Value};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "index_create",
            "title": "Create Index",
            "description": "Create an index of any type. Set 'type' to choose: 'btree' (default; B+ tree on 'field' or compound 'fields'), 'fulltext' (BM25 with language-aware stemming), 'fuzzy' (approximate string matching), or 'vector' (HNSW similarity search). Only the fields relevant to the chosen type are used; others are ignored. Required fields per type: btree → field OR fields; fulltext/fuzzy → field; vector → field + dim. Server validates per type and returns an explicit error if a required field is missing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection(),
                    "type": {
                        "type": "string",
                        "description": "Index type to create (default: 'btree')",
                        "enum": ["btree", "fulltext", "fuzzy", "vector"],
                        "default": "btree"
                    },
                    "field": fields::index_field(),
                    "fields": fields::index_fields(),
                    "unique": fields::unique(),
                    "sparse": fields::sparse(),
                    // fulltext params
                    "language": fields::fulltext_language(),
                    "min_word_length": fields::min_word_length(),
                    "accent_folding": fields::accent_folding(),
                    // fuzzy params
                    "algorithm": fields::fuzzy_algorithm(),
                    "threshold": fields::fuzzy_threshold(),
                    // vector params
                    "dim": {
                        "type": "integer",
                        "description": "[vector] Vector dimension (must match your embeddings)",
                        "minimum": 1,
                        "maximum": 4096
                    },
                    "metric": {
                        "type": "string",
                        "description": "[vector] Distance metric for similarity calculation",
                        "enum": ["cosine", "euclidean", "dot_product"],
                        "default": "cosine"
                    },
                    "max_vectors": {
                        "type": "integer",
                        "description": "[vector] Maximum number of vectors (default: 100,000)",
                        "default": 100000
                    },
                    "m": {
                        "type": "integer",
                        "description": "[vector] HNSW M parameter (connections per node, default: 16)",
                        "default": 16,
                        "minimum": 4,
                        "maximum": 64
                    },
                    "ef_construction": {
                        "type": "integer",
                        "description": "[vector] HNSW ef_construction (build-time quality, default: 200)",
                        "default": 200,
                        "minimum": 50,
                        "maximum": 1000
                    },
                    "ef_search": {
                        "type": "integer",
                        "description": "[vector] HNSW ef_search (search-time quality, default: 50)",
                        "default": 50,
                        "minimum": 10,
                        "maximum": 500
                    }
                },
                "required": ["collection"]
            }
        }),
        json!({
            "name": "index_list",
            "title": "List Indexes",
            "description": "List indexes on a collection: B+ tree, fulltext, fuzzy, and vector — all with details. Pass 'type' (btree|fulltext|fuzzy|vector) to list only one subtype; omit it to list everything.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection(),
                    "type": {
                        "type": "string",
                        "description": "Optional: restrict listing to one index subtype",
                        "enum": ["btree", "fulltext", "fuzzy", "vector"]
                    }
                },
                "required": ["collection"]
            }
        }),
        json!({
            "name": "fulltext_search",
            "title": "Full-Text Search",
            "description": "Search documents using BM25 relevance scoring (TF saturation + document length normalization). Requires a fulltext index first (create via index_create with type='fulltext'). Use 'filter' to apply MongoDB-style filtering on results (fast, applies AFTER scoring). Set highlight=true for <mark>...</mark> snippets. Searched field must be in projection for highlights. Use 'fields' (array) for multi-field search, or 'field' (string) for single-field. If both are provided, 'fields' takes precedence.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Collection with fulltext index"
                    },
                    "field": {
                        "type": "string",
                        "description": "Single indexed field to search (default: 'content'). Use 'fields' for multi-field search.",
                        "default": "content"
                    },
                    "fields": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Multiple fields to search across (overrides 'field'). Scores merged per document using best-field strategy (max score)."
                    },
                    "query": fields::search_query(),
                    "mode": {
                        "type": "string",
                        "description": "Search mode: 'or' (default) = any word matches, relevance ranked by BM25 score (industry-standard disjunctive retrieval); 'and' = ALL words must match (opt-in precision filter)",
                        "enum": ["and", "or"],
                        "default": "or"
                    },
                    "match_scope": {
                        "type": "string",
                        "description": "Match scope for 'and' mode only: 'document' (default) = all words must appear across the document's chunks (ideal for RAG), 'chunk' = all words in a single chunk. No effect in the default 'or' mode.",
                        "enum": ["document", "chunk"],
                        "default": "document"
                    },
                    "limit": fields::limit_results(10),
                    "skip": fields::skip_results(),
                    "min_score": fields::min_score(),
                    "projection": fields::projection_simple(),
                    "filter": fields::fulltext_filter(),
                    "highlight": fields::highlight(),
                    "highlight_context": fields::highlight_context(),
                    "highlight_max_snippets": fields::highlight_max_snippets()
                },
                "required": ["collection", "query"]
            }
        }),
        json!({
            "name": "fulltext_analyze",
            "title": "Analyze Text Tokenization",
            "description": "Debug tool showing how text is tokenized: original words, normalized form, stemmed form, and which tokens are kept. Useful for understanding why searches match or don't match. Pass 'collection' AND 'field' to inherit that index's real language/accent_folding/min_word_length (so the output matches how that index tokenizes); otherwise the explicit 'language' param is used (default 'none').",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": fields::text_to_analyze(),
                    "language": fields::fulltext_language(),
                    "accent_folding": fields::accent_folding(),
                    "min_word_length": fields::min_word_length(),
                    "collection": {
                        "type": "string",
                        "description": "Optional: inherit tokenization config from this collection's fulltext index (requires 'field')."
                    },
                    "field": {
                        "type": "string",
                        "description": "Optional: the fulltext-indexed field whose language/options to inherit (requires 'collection')."
                    }
                },
                "required": ["text"]
            }
        }),
        json!({
            "name": "fuzzy_search",
            "title": "Fuzzy String Search",
            "description": "Find documents with approximate string matching. Requires a fuzzy index first (create via index_create with type='fuzzy'). Supports filter, projection, and highlight.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Collection with fuzzy index"
                    },
                    "field": {
                        "type": "string",
                        "description": "Indexed field to search"
                    },
                    "query": fields::fuzzy_query(),
                    "algorithm": {
                        "type": "string",
                        "description": "Override index algorithm for this search",
                        "enum": ["jaro_winkler", "levenshtein", "damerau_levenshtein"]
                    },
                    "threshold": {
                        "type": "number",
                        "description": "Override similarity threshold (0.0-1.0)",
                        "minimum": 0,
                        "maximum": 1
                    },
                    "limit": fields::limit(None),
                    "skip": fields::skip(),
                    "projection": fields::projection_simple(),
                    "filter": {
                        "type": "object",
                        "description": "MongoDB-style filter applied AFTER fuzzy matching. Example: {\"status\": \"active\"}"
                    },
                    "highlight": {
                        "type": "boolean",
                        "description": "Enable highlighting of matched value with <mark> tags",
                        "default": false
                    }
                },
                "required": ["collection", "field", "query"]
            }
        }),
        json!({
            "name": "index_drop",
            "title": "Drop Index",
            "description": "Remove an index of any type (B+ tree, fulltext, fuzzy, or vector) from a collection. The subtype is detected automatically from the index name; vector indexes also have their on-disk cache file removed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection(),
                    "index_name": fields::index_name()
                },
                "required": ["collection", "index_name"]
            }
        }),
        json!({
            "name": "index_stats_refresh",
            "title": "Refresh Index Statistics",
            "description": "Recompute statistics for all B+ tree indexes: distinct_count, null_count, and histograms. For indexes with 100k+ keys, equi-depth histograms (64 buckets) are built for accurate range query selectivity estimation. The query planner uses these statistics to choose the best index. Run this after bulk inserts for optimal query plans.",
            "inputSchema": schemas::collection_only()
        }),
        json!({
            "name": "index_stats",
            "title": "Get Index Statistics",
            "description": "Get detailed statistics for indexes in a collection. Default ('btree') returns num_keys, distinct_count, has_histogram, has_mcv per B+ tree index. Set 'type' to 'fulltext' (num_documents, num_tokens), 'fuzzy' (num_entries), or 'vector' (vector_count) for those subtypes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection(),
                    "type": {
                        "type": "string",
                        "description": "Index subtype to report stats for (default: 'btree')",
                        "enum": ["btree", "fulltext", "fuzzy", "vector"],
                        "default": "btree"
                    }
                },
                "required": ["collection"]
            }
        }),
        json!({
            "name": "explain",
            "title": "Explain Query Plan",
            "description": "Analyze query execution plan showing which indexes will be used.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": {
                        "type": "string",
                        "description": "Collection to analyze"
                    },
                    "query": {
                        "type": "object",
                        "description": "Query filter to analyze. Shows: scan type, index used, estimated cost."
                    }
                },
                "required": ["collection"]
            }
        }),
        json!({
            "name": "find_with_hint",
            "title": "Query with Index Hint",
            "description": "Execute a query forcing a specific index. Useful for testing index performance.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection": fields::collection_query(),
                    "query": {
                        "type": "object",
                        "description": "MongoDB-style filter"
                    },
                    "hint": fields::hint(),
                    "projection": fields::projection_simple(),
                    "sort": fields::sort(),
                    "limit": {
                        "type": "integer",
                        "description": "Maximum documents to return",
                        "default": 10000
                    },
                    "skip": fields::skip()
                },
                "required": ["collection", "hint"]
            }
        }),
    ]
}
