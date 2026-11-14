//! Tool markers: one zero-sized type per MCP tool
//!
//! A marker ties a tool's registered name to its argument and response types, so
//! a client calls `Find` with `FindParams` instead of a hand-built JSON object.
//! `Resp` is `serde_json::Value` until the tool's response gets its own struct.
//!
//! Only tools whose handler parses a typed argument struct have a marker; tools
//! without arguments and `index_create` (four argument shapes selected by `type`)
//! do not have one yet.

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::params::*;

/// An MCP tool: its registered name, its arguments and its response.
pub trait Tool {
    /// The name the server registers the tool under (`tools/call` → `name`).
    const NAME: &'static str;
    /// The tool's arguments (`tools/call` → `arguments`).
    type Args: Serialize + DeserializeOwned;
    /// The tool's response.
    type Resp: Serialize + DeserializeOwned;
}

macro_rules! tools {
    ($($(#[$meta:meta])* $marker:ident => $name:literal, $args:ty;)+) => {
        $(
            $(#[$meta])*
            #[doc = concat!("`", $name, "` tool")]
            #[derive(Debug, Clone, Copy, PartialEq, Eq)]
            pub struct $marker;

            impl Tool for $marker {
                const NAME: &'static str = $name;
                type Args = $args;
                type Resp = Value;
            }
        )+

        /// `Tool::NAME` of every marker in this module, in declaration order.
        pub const TOOL_NAMES: &[&str] = &[$(<$marker as Tool>::NAME),+];
    };
}

tools! {
    // Document CRUD
    InsertOne => "insert_one", InsertOneParams;
    InsertMany => "insert_many", InsertManyParams;
    Find => "find", FindParams;
    FindOne => "find_one", FindOneParams;
    UpdateOne => "update_one", UpdateOneParams;
    UpdateMany => "update_many", UpdateManyParams;
    DeleteOne => "delete_one", DeleteParams;
    DeleteMany => "delete_many", DeleteParams;
    Count => "count", CountParams;
    Distinct => "distinct", DistinctParams;
    Aggregate => "aggregate", AggregateParams;

    // Index management & search
    IndexList => "index_list", IndexListParams;
    IndexDrop => "index_drop", IndexDropParams;
    IndexStats => "index_stats", IndexListParams;
    IndexStatsRefresh => "index_stats_refresh", IndexListParams;
    FuzzySearch => "fuzzy_search", FuzzySearchParams;
    FulltextSearch => "fulltext_search", FulltextSearchParams;
    FulltextAnalyze => "fulltext_analyze", FulltextAnalyzeParams;
    Explain => "explain", ExplainParams;
    FindWithHint => "find_with_hint", FindWithHintParams;

    // Collection & schema
    CollectionCreate => "collection_create", CollectionCreateParams;
    CollectionDrop => "collection_drop", CollectionDropParams;
    CollectionRename => "collection_rename", CollectionRenameParams;
    SchemaSet => "schema_set", SchemaSetParams;
    SchemaGet => "schema_get", SchemaGetParams;

    // Transactions
    TransactionCommit => "transaction_commit", TransactionIdParams;
    TransactionRollback => "transaction_rollback", TransactionIdParams;
    TransactionInsertOne => "transaction_insert_one", TransactionInsertParams;
    TransactionUpdateOne => "transaction_update_one", TransactionUpdateParams;
    TransactionDeleteOne => "transaction_delete_one", TransactionDeleteParams;

    // Scripts
    ScriptSave => "script_save", ScriptSaveParams;
    ScriptList => "script_list", ScriptListParams;
    ScriptGet => "script_get", ScriptNameParams;
    ScriptDelete => "script_delete", ScriptNameParams;
    ScriptRun => "script_run", ScriptRunParams;
    ScriptExec => "script_exec", ScriptExecParams;
    ScriptHistory => "script_history", ScriptHistoryParams;
    ScriptRollback => "script_rollback", ScriptVersionParams;
    ScriptVersionGet => "script_version_get", ScriptVersionParams;
    ScriptTagsAdd => "script_tags_add", ScriptTagsParams;
    ScriptTagsRemove => "script_tags_remove", ScriptTagsParams;
    ScriptStats => "script_stats", ScriptNameParams;

    // ACL
    AclGet => "acl_get", AclCollectionParams;
    AclSet => "acl_set", AclSetParams;
    AclDelete => "acl_delete", AclCollectionParams;

    // Listeners
    ListenerGet => "listener_get", ListenerIdParams;
    ListenerCreate => "listener_create", ListenerAddParams;
    ListenerDelete => "listener_delete", ListenerIdParams;
    ListenerEnable => "listener_enable", ListenerIdParams;
    ListenerDisable => "listener_disable", ListenerIdParams;

    // Database & admin
    DbOpen => "db_open", DbOpenParams;
    AdminCollectionList => "admin_collection_list", AdminKeyParams;
    AdminCollectionCreateSystem => "admin_collection_create_system", AdminCollectionParams;
    AdminCollectionSetFlags => "admin_collection_set_flags", AdminFlagsParams;
    AdminCollectionDropProtected => "admin_collection_drop_protected", AdminCollectionParams;
    AdminApikeyCreate => "admin_apikey_create", AdminApiKeyCreateParams;
    AdminApikeyList => "admin_apikey_list", AdminKeyParams;
    AdminApikeyRevoke => "admin_apikey_revoke", AdminApiKeyIdParams;
    AdminApikeyDelete => "admin_apikey_delete", AdminApiKeyIdParams;

    // Vector & retrieval
    VectorSearch => "vector_search", VectorSearchParams;
    Search => "search", SearchParams;

    // Embedding
    EmbedText => "embed_text", EmbedTextParams;
    EmbedBatch => "embed_batch", EmbedBatchParams;
    EmbedDocument => "embed_document", EmbedDocumentParams;

    // Auto-embedding
    AutoEmbedEnable => "auto_embed_enable", AutoEmbedEnableParams;
    AutoEmbedDisable => "auto_embed_disable", AutoEmbedDisableParams;
    AutoEmbedStatus => "auto_embed_status", AutoEmbedStatusParams;

    // Embedding jobs
    EmbedJobStatus => "embed_job_status", JobStatusParams;
    EmbedJobList => "embed_job_list", JobListParams;
    EmbedJobCancel => "embed_job_cancel", JobCancelParams;

    // RAG
    RagCollectionCreate => "rag_collection_create", RagCollectionCreateParams;
    RagDocumentImport => "rag_document_import", RagDocumentImportParams;
    RagCollectionStats => "rag_collection_stats", RagCollectionStatsParams;
    RagChunksLoad => "rag_chunks_load", RagLoadAllChunksParams;
}
