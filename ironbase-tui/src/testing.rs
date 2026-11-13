//! Test support: a scriptable in-memory MCP server, an `App` connected to it,
//! key-press helpers that drive the real handler entry, and a renderer that
//! returns the screen as text

use crate::app::App;
use crate::config::Config;
use crate::db::DbWrapper;
use crate::mcp::{JsonRpcRequest, JsonRpcResponse, McpResult, Transport};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// In-memory stand-in for the MCP server.
///
/// Built in: just enough of collection_list / find / count / index_list /
/// collection_drop over `collections`. Any tool can be scripted with
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
        let query = args["query"].as_object().cloned().unwrap_or_default();
        docs.into_iter()
            .filter(|doc| query.iter().all(|(k, v)| doc.get(k) == Some(v)))
            .collect()
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
            "collection_list" => {
                let collections = self.collections.lock().unwrap();
                let names: Vec<&String> = collections.iter().map(|(n, _)| n).collect();
                Ok(serde_json::json!({ "collections": names }))
            }
            "find" => {
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
                let skip = args["skip"].as_u64().unwrap_or(0) as usize;
                let limit = args["limit"].as_u64().unwrap_or(u64::MAX) as usize;
                let page: Vec<Value> = docs.into_iter().skip(skip).take(limit).collect();
                Ok(serde_json::json!({ "documents": page }))
            }
            "count" => Ok(serde_json::json!({ "count": self.matching(args).len() })),
            "index_list" => Ok(serde_json::json!({ "btree_indexes": [] })),
            "collection_drop" => {
                let name = args["collection"].as_str().unwrap_or_default();
                self.collections.lock().unwrap().retain(|(n, _)| n != name);
                Ok(serde_json::json!({ "success": true }))
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
