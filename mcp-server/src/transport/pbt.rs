//! P8 (property-based testing plan): random JSON-RPC requests for the
//! transport handlers (stdio `HandlerContext` and the HTTP handler), and the
//! answer every request must get.

use super::{McpRequest, McpResponse};
use serde_json::json;

/// Cases per property: `PROPTEST_CASES` when set, else 1500
pub(crate) fn cases() -> usize {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1500)
}

/// Every shipped tool but `db_compact` (it rewrites the whole database
/// file: slow, and covered by its own tests)
pub(crate) fn tool_names() -> Vec<String> {
    crate::get_tools_list()["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str().map(String::from))
        .filter(|n| n != "db_compact")
        .collect()
}

/// Deterministic generator (xorshift) of random JSON-RPC requests: no
/// extra dependency, reproducible from the seed
pub(crate) struct Gen(pub(crate) u64);

impl Gen {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
    /// Any JSON value, `depth` levels deep at most; object keys are
    /// mostly real tool parameter names, so calls reach the handlers
    fn json(&mut self, depth: usize) -> serde_json::Value {
        const KEYS: &[&str] = &[
            "collection",
            "filter",
            "query",
            "document",
            "documents",
            "update",
            "pipeline",
            "field",
            "fields",
            "limit",
            "skip",
            "sort",
            "name",
            "code",
            "index_name",
            "unique",
            "old_collection",
            "new_collection",
            "text",
            "k",
            "vector",
            "options",
            "projection",
            "$set",
            "$gt",
            "",
            "\u{0}",
        ];
        const STRINGS: &[&str] = &["users", "c1", "_system.acl", "", "a.b", "é😀"];
        match self.below(if depth == 0 { 6 } else { 9 }) {
            0 => serde_json::Value::Null,
            1 => json!(self.below(2) == 0),
            2 => json!((self.next() as i64) >> self.below(64)),
            3 => json!((self.next() % 100_000) as f64 / 7.0),
            4 => json!(self.pick(STRINGS)),
            5 => json!(self.pick(KEYS)),
            6 | 7 => {
                let mut map = serde_json::Map::new();
                for _ in 0..self.below(5) {
                    let key = self.pick(KEYS).to_string();
                    let value = self.json(depth - 1);
                    map.insert(key, value);
                }
                serde_json::Value::Object(map)
            }
            _ => {
                let len = self.below(4);
                serde_json::Value::Array((0..len).map(|_| self.json(depth - 1)).collect())
            }
        }
    }
}

/// A random request: id absent, null, number, string or another value;
/// a real method or not; tool calls to real tools with random arguments
pub(crate) fn random_request(gen: &mut Gen, tools: &[String]) -> McpRequest {
    let id = match gen.below(6) {
        0 => None,
        1 => Some(serde_json::Value::Null),
        2 => Some(json!(gen.next() as i64)),
        3 => Some(json!(format!("id-{}", gen.next()))),
        4 => Some(json!(gen.below(10))),
        _ => Some(gen.json(1)),
    };
    let methods = [
        "tools/call",
        "tools/call",
        "tools/call",
        "tools/list",
        "ping",
        "initialize",
        "prompts/list",
        "prompts/get",
        "resources/list",
        "resources/read",
        "notifications/cancelled",
        "nope",
        "",
    ];
    let method = gen.pick(&methods).to_string();
    let params = if method == "tools/call" && gen.below(5) != 0 {
        let name = tools[gen.below(tools.len())].clone();
        json!({ "name": name, "arguments": gen.json(3) })
    } else {
        gen.json(3)
    };
    McpRequest {
        jsonrpc: Some("2.0".to_string()),
        id,
        method,
        params,
    }
}

/// `handle` does not panic on `request`; a request with an id gets one
/// response carrying that id, with either a result or an error; a
/// notification gets none
pub(crate) fn assert_answers(
    case: usize,
    request: &McpRequest,
    handle: impl FnOnce(&McpRequest) -> Option<McpResponse>,
) {
    let shown = format!(
        "case {case}: {} {:?} {}",
        request.method, request.id, request.params
    );
    let response = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle(request)))
        .unwrap_or_else(|_| panic!("panicked on {shown}"));
    match (&request.id, response) {
        (None, None) => {}
        (None, Some(r)) => panic!("answered a notification: {shown} -> {r:?}"),
        (Some(_), None) => panic!("no answer: {shown}"),
        (Some(id), Some(r)) => {
            let got = match &r {
                McpResponse::Success { id, .. } | McpResponse::Error { id, .. } => id,
            };
            assert_eq!(got, id, "{shown}");
            let text = serde_json::to_value(&r).unwrap();
            assert_eq!(text["jsonrpc"], "2.0", "{shown}");
            assert!(
                text.get("result").is_some() != text.get("error").is_some(),
                "{shown}: {text}"
            );
        }
    }
}
