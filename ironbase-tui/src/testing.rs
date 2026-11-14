//! Test support: a scriptable in-memory MCP server, an `App` connected to it,
//! key-press helpers that drive the real handler entry, and a renderer that
//! returns the screen as text

use crate::app::App;
use crate::config::Config;
use crate::db::DbWrapper;
use crate::mcp::{JsonRpcRequest, JsonRpcResponse, McpResult, Transport};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ironbase_mcp_types::params::{
    CollectionCreateParams, CollectionDropParams, DeleteParams, IndexCreateArgs, IndexDropParams,
    IndexListParams, InsertOneParams, UpdateOneParams,
};
use ironbase_mcp_types::responses::{
    BTreeIndexCreated, CollectionCreateResponse, CollectionDropResponse, CollectionListResponse,
    CompoundIndexCreated, CountResponse, DeleteResponse, FindResponse, FulltextIndexCreated,
    FuzzyIndexCreated, HnswParams, IndexCreateResponse, IndexDropResponse, IndexListAll,
    IndexListResponse, IndexStatsRefreshResponse, InsertOneResponse, TypedIndexList,
    UpdateOneResponse, VectorIndexCreated,
};
use ironbase_mcp_types::tools::{
    CollectionCreate, CollectionDrop, CollectionList, Count, DeleteOne, Find, IndexCreate,
    IndexDrop, IndexList, IndexStatsRefresh, InsertOne, UpdateOne,
};
use ironbase_mcp_types::Tool;
use ratatui::{backend::TestBackend, Terminal};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// In-memory stand-in for the MCP server.
///
/// Built in, over `collections` and answering with the shared response structs
/// (so their shape is the server's): just enough of collection_list / find /
/// count / index_list (no indexes), insert_one / update_one (`$set` only) /
/// delete_one with top-level equality filters, collection_create /
/// collection_drop, and index_create / index_drop / index_stats_refresh (echoed,
/// not stored). Any tool can be scripted with
/// [`FakeServer::respond`] / [`FakeServer::fail`] (scripted answers win over
/// the built-in ones); an unscripted, unknown tool is an error. Every call is
/// logged.
#[derive(Default)]
pub(crate) struct FakeServer {
    /// (name, documents) in the order the server lists them
    pub(crate) collections: Mutex<Vec<(String, Vec<Value>)>>,
    /// `find` on this collection fails
    pub(crate) failing_find: Mutex<Option<String>>,
    /// tool name -> canned result (`Ok`) or tool error (`Err`)
    scripted: Mutex<HashMap<String, Result<Value, String>>>,
    /// (tool name, arguments) of every call, in order
    calls: Mutex<Vec<(String, Value)>>,
}

impl FakeServer {
    pub(crate) fn new(collections: Vec<(&str, Vec<Value>)>) -> Arc<Self> {
        Arc::new(Self {
            collections: Mutex::new(
                collections
                    .into_iter()
                    .map(|(name, docs)| (name.to_string(), docs))
                    .collect(),
            ),
            ..Default::default()
        })
    }

    /// Every later call of `tool` returns `result`
    pub(crate) fn respond(&self, tool: &str, result: Value) {
        self.scripted
            .lock()
            .unwrap()
            .insert(tool.to_string(), Ok(result));
    }

    /// Every later call of `tool` fails with `message`
    pub(crate) fn fail(&self, tool: &str, message: &str) {
        self.scripted
            .lock()
            .unwrap()
            .insert(tool.to_string(), Err(message.to_string()));
    }

    /// The call log: (tool name, arguments) in call order
    pub(crate) fn calls(&self) -> Vec<(String, Value)> {
        self.calls.lock().unwrap().clone()
    }

    /// Arguments of every call of `tool`, in call order
    pub(crate) fn calls_to(&self, tool: &str) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == tool)
            .map(|(_, args)| args.clone())
            .collect()
    }

    /// Forget the calls logged so far
    pub(crate) fn clear_calls(&self) {
        self.calls.lock().unwrap().clear();
    }

    /// Documents matching a query of top-level equalities
    fn matching(&self, args: &Value) -> Vec<Value> {
        let name = args["collection"].as_str().unwrap_or_default();
        let collections = self.collections.lock().unwrap();
        let docs = collections
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, docs)| docs.clone())
            .unwrap_or_default();
        docs.into_iter()
            .filter(|doc| matches(doc, &args["query"]))
            .collect()
    }

    /// Run `f` on the documents of `collection` (an error if it does not exist)
    fn with_docs<T>(
        &self,
        collection: &str,
        f: impl FnOnce(&mut Vec<Value>) -> T,
    ) -> Result<T, String> {
        let mut collections = self.collections.lock().unwrap();
        collections
            .iter_mut()
            .find(|(n, _)| n == collection)
            .map(|(_, docs)| f(docs))
            .ok_or_else(|| format!("Collection '{}' not found", collection))
    }

    fn insert_one(&self, p: InsertOneParams) -> Result<Value, String> {
        let mut collections = self.collections.lock().unwrap();
        if !collections.iter().any(|(n, _)| *n == p.collection) {
            collections.push((p.collection.clone(), Vec::new()));
        }
        let docs = &mut collections
            .iter_mut()
            .find(|(n, _)| *n == p.collection)
            .unwrap()
            .1;
        let mut document = p.document;
        let id = match document.get("_id") {
            Some(id) => id.clone(),
            None => {
                let next = docs.iter().filter_map(|d| d["_id"].as_i64()).max();
                Value::from(next.map_or(1, |n| n + 1))
            }
        };
        document["_id"] = id.clone();
        docs.push(document);
        // The server reports every `_id` as a string
        let inserted_id = id.as_str().map_or_else(|| id.to_string(), String::from);
        to_result(InsertOneResponse {
            inserted_id,
            auto_embedded: None,
        })
    }

    fn update_one(&self, p: UpdateOneParams) -> Result<Value, String> {
        let (matched_count, modified_count) = self.with_docs(&p.collection, |docs| {
            let Some(doc) = docs.iter_mut().find(|doc| matches(doc, &p.filter)) else {
                return (0, 0);
            };
            let before = doc.clone();
            if let Some(set) = p.update["$set"].as_object() {
                for (k, v) in set {
                    doc[k] = v.clone();
                }
            }
            (1, u64::from(*doc != before))
        })?;
        to_result(UpdateOneResponse {
            matched_count,
            modified_count,
            upserted_id: None,
            auto_embedded: None,
            auto_embedded_count: None,
            auto_embed_error: None,
        })
    }

    fn delete_one(&self, p: DeleteParams) -> Result<Value, String> {
        let deleted_count = self.with_docs(&p.collection, |docs| {
            match docs.iter().position(|doc| matches(doc, &p.filter)) {
                Some(i) => {
                    docs.remove(i);
                    1
                }
                None => 0,
            }
        })?;
        to_result(DeleteResponse { deleted_count })
    }

    fn collection_create(&self, p: CollectionCreateParams) -> Result<Value, String> {
        let mut collections = self.collections.lock().unwrap();
        if !collections.iter().any(|(n, _)| *n == p.collection) {
            collections.push((p.collection.clone(), Vec::new()));
        }
        to_result(CollectionCreateResponse {
            success: true,
            collection: p.collection,
        })
    }

    fn collection_drop(&self, p: CollectionDropParams) -> Result<Value, String> {
        let mut collections = self.collections.lock().unwrap();
        let before = collections.len();
        collections.retain(|(n, _)| *n != p.collection);
        if collections.len() == before {
            return Err(format!("Collection '{}' not found", p.collection));
        }
        to_result(CollectionDropResponse {
            success: true,
            dropped: p.collection,
            acl_deleted: false,
        })
    }

    fn call(&self, name: &str, args: &Value) -> Result<Value, String> {
        self.calls
            .lock()
            .unwrap()
            .push((name.to_string(), args.clone()));
        if let Some(scripted) = self.scripted.lock().unwrap().get(name) {
            return scripted.clone();
        }
        match name {
            CollectionList::NAME => {
                let collections = self.collections.lock().unwrap();
                let collections = collections.iter().map(|(n, _)| n.clone()).collect();
                to_result(CollectionListResponse { collections })
            }
            Find::NAME => {
                let collection = args["collection"].as_str().map(String::from);
                if *self.failing_find.lock().unwrap() == collection {
                    return Err("find failed".to_string());
                }
                let mut docs = self.matching(args);
                if let Some((field, dir)) = args["sort"].as_object().and_then(|s| s.iter().next()) {
                    docs.sort_by_key(|doc| doc[field].as_i64().unwrap_or(0));
                    if dir.as_i64() == Some(-1) {
                        docs.reverse();
                    }
                }
                let total = docs.len();
                let skip = args["skip"].as_u64().unwrap_or(0) as usize;
                let limit = args["limit"].as_u64().unwrap_or(u64::MAX) as usize;
                let documents: Vec<Value> = docs.into_iter().skip(skip).take(limit).collect();
                to_result(FindResponse {
                    count: documents.len(),
                    documents,
                    total: (args["include_total"] == true).then_some(total),
                })
            }
            Count::NAME => to_result(CountResponse {
                count: self.matching(args).len() as u64,
            }),
            // No indexes: every subtype empty, or the asked one
            IndexList::NAME => match args["type"].as_str() {
                None => to_result(IndexListResponse::All(IndexListAll::new(
                    vec![],
                    vec![],
                    vec![],
                    vec![],
                ))),
                Some(_) => to_result(IndexListResponse::BTree(TypedIndexList::new(vec![]))),
            },
            InsertOne::NAME => self.insert_one(parse(args)?),
            UpdateOne::NAME => self.update_one(parse(args)?),
            DeleteOne::NAME => self.delete_one(parse(args)?),
            CollectionCreate::NAME => self.collection_create(parse(args)?),
            CollectionDrop::NAME => self.collection_drop(parse(args)?),
            IndexCreate::NAME => to_result(index_created(parse(args)?)),
            IndexDrop::NAME => {
                let p: IndexDropParams = parse(args)?;
                to_result(IndexDropResponse {
                    success: true,
                    dropped: p.index_name,
                })
            }
            IndexStatsRefresh::NAME => {
                let p: IndexListParams = parse(args)?;
                to_result(IndexStatsRefreshResponse {
                    success: true,
                    message: format!(
                        "Index statistics refreshed for collection '{}'",
                        p.collection
                    ),
                })
            }
            other => Err(format!("unexpected tool: {}", other)),
        }
    }
}

#[async_trait::async_trait]
impl Transport for FakeServer {
    async fn send(&self, request: &JsonRpcRequest) -> McpResult<JsonRpcResponse> {
        let params = request.params.clone().unwrap_or_default();
        let name = params["name"].as_str().unwrap_or_default();
        let (text, is_error) = match self.call(name, &params["arguments"]) {
            Ok(value) => (value.to_string(), false),
            Err(message) => (message, true),
        };
        Ok(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(request.id),
            result: Some(serde_json::json!({
                "content": [{"type": "text", "text": text}],
                "isError": is_error
            })),
            error: None,
        })
    }

    async fn close(&self) -> McpResult<()> {
        Ok(())
    }
}

/// A built-in answer: the response struct as the server serializes it
fn to_result<T: serde::Serialize>(response: T) -> Result<Value, String> {
    serde_json::to_value(response).map_err(|e| e.to_string())
}

/// The tool's arguments, parsed with the shared argument type
fn parse<T: serde::de::DeserializeOwned>(args: &Value) -> Result<T, String> {
    serde_json::from_value(args.clone()).map_err(|e| e.to_string())
}

/// `doc` matches a filter of top-level equalities (no filter: everything)
fn matches(doc: &Value, filter: &Value) -> bool {
    filter
        .as_object()
        .is_none_or(|f| f.iter().all(|(k, v)| doc.get(k) == Some(v)))
}

/// The `index_create` answer for `args`, named the way the server names indexes
fn index_created(args: IndexCreateArgs) -> IndexCreateResponse {
    match args {
        IndexCreateArgs::BTree(p) => match p.fields {
            Some(fields) => IndexCreateResponse::Compound(CompoundIndexCreated {
                index_name: format!("{}_{}", p.collection, fields.join("_")),
                fields,
                unique: p.unique,
                sparse: p.sparse,
            }),
            None => {
                let field = p.field.unwrap_or_default();
                IndexCreateResponse::BTree(BTreeIndexCreated {
                    index_name: format!("{}_{}", p.collection, field),
                    field,
                    unique: p.unique,
                    sparse: p.sparse,
                })
            }
        },
        IndexCreateArgs::Fulltext(p) => IndexCreateResponse::Fulltext(FulltextIndexCreated {
            index_name: format!("{}_{}_fts", p.collection, p.field),
            field: p.field,
            language: p.language,
            min_word_length: p.min_word_length.unwrap_or(2),
            accent_folding: p.accent_folding.unwrap_or(true),
        }),
        IndexCreateArgs::Fuzzy(p) => IndexCreateResponse::Fuzzy(FuzzyIndexCreated {
            index_name: format!("{}_{}_fuzzy", p.collection, p.field),
            field: p.field,
            algorithm: p.algorithm,
            threshold: p.threshold,
        }),
        IndexCreateArgs::Vector(p) => IndexCreateResponse::Vector(VectorIndexCreated {
            index_name: format!("{}_vec_{}", p.collection, p.field),
            field: p.field,
            dim: p.dim,
            metric: p.metric,
            max_vectors: p.max_vectors,
            hnsw_params: HnswParams {
                m: p.m,
                ef_construction: p.ef_construction,
                ef_search: p.ef_search,
            },
        }),
    }
}

/// `count` documents: {"_id": i, "n": i, "even": i % 2 == 0}
pub(crate) fn numbered_docs(count: i64) -> Vec<Value> {
    (0..count)
        .map(|i| serde_json::json!({"_id": i, "n": i, "even": i % 2 == 0}))
        .collect()
}

/// An `App` connected to `server`, collections and first page loaded
pub(crate) async fn app_with_server(server: &Arc<FakeServer>) -> App {
    let mut app = App::new(Config::default());
    app.db = Some(DbWrapper::with_transport(server.clone()));
    app.refresh_collections_async().await.unwrap();
    app
}

/// Key press without modifiers
pub(crate) fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// Send one key (no modifiers) through the real handler entry
pub(crate) async fn press(app: &mut App, code: KeyCode) {
    crate::handlers::handle_key(app, key(code)).await;
}

/// Send one key with modifiers through the real handler entry
pub(crate) async fn press_with(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    crate::handlers::handle_key(app, KeyEvent::new(code, modifiers)).await;
}

/// Type `text` the way a terminal reports it: one `Char` key per character,
/// uppercase letters with SHIFT
pub(crate) async fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        let modifiers = if c.is_uppercase() {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        press_with(app, KeyCode::Char(c), modifiers).await;
    }
}

/// Render `app` into a `width` x `height` test terminal and return the
/// screen: lines joined by `\n`, trailing spaces trimmed
pub(crate) fn render_to_string(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| crate::ui::render_ui(frame, app))
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            let line: String = (0..width).map(|x| buffer[(x, y)].symbol()).collect();
            line.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
