//! Tool markers: one zero-sized type per MCP tool
//!
//! A marker ties a tool's registered name to its argument and response types, so
//! a client calls `Find` with `FindParams` instead of a hand-built JSON object.
//! `Resp` is `serde_json::Value` until the tool's response gets its own struct
//! (`crate::responses`).
//!
//! Tools whose handler parses a typed argument struct have a marker, and so do
//! the argument-less read tools a client needs (`collection_list`, `db_stats`,
//! with [`NoParams`]), and `index_create` (with [`IndexCreateArgs`], one argument
//! struct per `type`); the other argument-less tools do not have one yet.

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::params::*;
use crate::responses::*;

/// An MCP tool: its registered name, its arguments and its response.
pub trait Tool {
    /// The name the server registers the tool under (`tools/call` → `name`).
    const NAME: &'static str;
    /// The tool's arguments (`tools/call` → `arguments`).
    type Args: Serialize + DeserializeOwned;
    /// The tool's response.
    type Resp: Serialize + DeserializeOwned;
}

/// The response type of a marker: the given one, or `Value` when none is given
macro_rules! resp_type {
    () => {
        Value
    };
    ($resp:ty) => {
        $resp
    };
}

macro_rules! tools {
    ($($(#[$meta:meta])* $marker:ident => $name:literal, $args:ty $(=> $resp:ty)?;)+) => {
        $(
            $(#[$meta])*
            #[doc = concat!("`", $name, "` tool")]
            #[derive(Debug, Clone, Copy, PartialEq, Eq)]
            pub struct $marker;

            impl Tool for $marker {
                const NAME: &'static str = $name;
                type Args = $args;
                type Resp = resp_type!($($resp)?);
            }
        )+

        /// `Tool::NAME` of every marker in this module, in declaration order.
        pub const TOOL_NAMES: &[&str] = &[$(<$marker as Tool>::NAME),+];
    };
}

tools! {
    // Document CRUD
    InsertOne => "insert_one", InsertOneParams => InsertOneResponse;
    InsertMany => "insert_many", InsertManyParams => InsertManyResponse;
    Find => "find", FindParams => FindResponse;
    FindOne => "find_one", FindOneParams;
    UpdateOne => "update_one", UpdateOneParams => UpdateOneResponse;
    UpdateMany => "update_many", UpdateManyParams => UpdateManyResponse;
    DeleteOne => "delete_one", DeleteParams => DeleteResponse;
    DeleteMany => "delete_many", DeleteParams => DeleteResponse;
    Count => "count", CountParams => CountResponse;
    Distinct => "distinct", DistinctParams;
    Aggregate => "aggregate", AggregateParams;

    // Index management & search
    IndexCreate => "index_create", IndexCreateArgs => IndexCreateResponse;
    IndexList => "index_list", IndexListParams => IndexListResponse;
    IndexDrop => "index_drop", IndexDropParams => IndexDropResponse;
    IndexStats => "index_stats", IndexListParams => IndexStatsResponse;
    IndexStatsRefresh => "index_stats_refresh", IndexListParams => IndexStatsRefreshResponse;
    FuzzySearch => "fuzzy_search", FuzzySearchParams;
    FulltextSearch => "fulltext_search", FulltextSearchParams;
    FulltextAnalyze => "fulltext_analyze", FulltextAnalyzeParams;
    Explain => "explain", ExplainParams => ExplainResponse;
    FindWithHint => "find_with_hint", FindWithHintParams;

    // Collection & schema
    CollectionList => "collection_list", NoParams => CollectionListResponse;
    CollectionCreate => "collection_create", CollectionCreateParams => CollectionCreateResponse;
    CollectionDrop => "collection_drop", CollectionDropParams => CollectionDropResponse;
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
    DbStats => "db_stats", NoParams => DbStatsResponse;
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
