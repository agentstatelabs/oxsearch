//! Telemetry through the `tracing` and `metrics` facades, following the
//! OpenTelemetry semantic conventions for Elasticsearch: one client span
//! per request and the `db.client.operation.duration` histogram. Install a
//! subscriber and a recorder, such as oxtel's, to export them; without
//! them, nothing is recorded.
//!
//! - The span is named by the endpoint, such as `search`, and carries
//!   `db.system.name`, `db.operation.name`, `db.collection.name` (for a
//!   single index), `http.request.method`, `db.response.status_code` and,
//!   on failure, `error.type`. Transports record where the request went
//!   (`url.full`, `server.address`, `server.port`) and retries, as events.
//! - Search-type requests record their body as `db.query.text`, sanitized:
//!   every value becomes `?`, and lists of values, such as vectors, one `?`,
//!   so field names and the query's shape remain. Turn it off with
//!   [`record_query_text`].
//! - Inference calls add `gen_ai.operation.name`: `embeddings`, `chat`,
//!   `text_completion` or `rerank`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use metrics::Label;
use serde_json::Value;
use tracing::field::Empty;

use crate::endpoint::{Body, HttpRequest};
use crate::error::Error;

static QUERY_TEXT: AtomicBool = AtomicBool::new(true);

/// Whether search-type requests record their sanitized body as
/// `db.query.text`: yes by default, for the whole process.
pub fn record_query_text(record: bool) {
    QUERY_TEXT.store(record, Ordering::Relaxed);
}

/// Describes the metrics oxsearch records (units and descriptions) to the
/// current `metrics` recorder.
pub fn describe_metrics() {
    metrics::describe_histogram!(
        "db.client.operation.duration",
        metrics::Unit::Seconds,
        "Duration of database client operations."
    );
}

/// Endpoints whose body is a query.
const SEARCHES: &[&str] = &[
    "search",
    "msearch",
    "count",
    "explain",
    "knn_search",
    "search_template",
    "msearch_template",
    "delete_by_query",
    "update_by_query",
    "async_search.submit",
];

/// The client span for a request to the endpoint `name`, if known.
pub(crate) fn span(name: Option<&'static str>, request: &HttpRequest) -> tracing::Span {
    let span = tracing::info_span!(
        target: "oxsearch",
        "elasticsearch request",
        otel.name = name.unwrap_or("elasticsearch"),
        otel.kind = "client",
        db.system.name = "elasticsearch",
        db.operation.name = Empty,
        db.collection.name = Empty,
        db.query.text = Empty,
        db.response.status_code = Empty,
        http.request.method = request.method.as_str(),
        url.full = Empty,
        server.address = Empty,
        server.port = Empty,
        elasticsearch.node.name = Empty,
        gen_ai.operation.name = Empty,
        error.type = Empty,
        otel.status_code = Empty,
    );
    if span.is_disabled() {
        return span;
    }
    if let Some(name) = name {
        span.record("db.operation.name", name);
        if let Some(operation) = gen_ai(name, &request.path_and_query) {
            span.record("gen_ai.operation.name", operation);
        }
        if SEARCHES.contains(&name)
            && QUERY_TEXT.load(Ordering::Relaxed)
            && let Some(text) = request.body.as_ref().and_then(sanitize)
        {
            span.record("db.query.text", text.as_str());
        }
    }
    if let Some(index) = collection(&request.path_and_query) {
        span.record("db.collection.name", index);
    }
    span
}

/// Records how a request went on its span, and its duration.
pub(crate) fn finish(
    span: &tracing::Span,
    name: Option<&'static str>,
    status: Option<u16>,
    error: Option<&Error>,
    elapsed: Duration,
) {
    let error_type = error.map(error_type);
    if let Some(status) = status {
        span.record("db.response.status_code", status.to_string().as_str());
    }
    if let Some(error_type) = &error_type {
        span.record("error.type", error_type.as_str());
        span.record("otel.status_code", "error");
    }
    let mut labels = Vec::with_capacity(5);
    labels.push(Label::new("db.system.name", "elasticsearch"));
    if let Some(name) = name {
        labels.push(Label::new("db.operation.name", name));
    }
    if let Some(error_type) = error_type {
        labels.push(Label::new("error.type", error_type));
        if let Some(status) = status {
            labels.push(Label::new("db.response.status_code", status.to_string()));
        }
    }
    metrics::histogram!("db.client.operation.duration", labels).record(elapsed.as_secs_f64());
}

/// A low-cardinality name for what went wrong.
fn error_type(error: &Error) -> String {
    match error {
        Error::Transport(_) => "transport".to_owned(),
        Error::Elasticsearch(e) => e
            .error_type()
            .map_or_else(|| e.status.to_string(), str::to_owned),
        Error::License(e) => e
            .error
            .error_type()
            .unwrap_or("security_exception")
            .to_owned(),
        Error::Conflict(e) => e
            .error_type()
            .unwrap_or("version_conflict_engine_exception")
            .to_owned(),
        Error::Encode(_) => "encode".to_owned(),
        Error::Parameter(_) => "parameter".to_owned(),
        Error::Decode { .. } | Error::Row(_) => "decode".to_owned(),
    }
}

/// The index a request targets, when it targets exactly one: the path's
/// first segment, unless it's an API's (`_search`), a list or a pattern.
fn collection(path_and_query: &str) -> Option<&str> {
    let path = path_and_query.split('?').next()?;
    let first = path.trim_start_matches('/').split('/').next()?;
    (!first.is_empty() && !first.starts_with('_') && !first.contains(['%', '*', ',']))
        .then_some(first)
}

/// What a GenAI client would call an inference endpoint's operation.
fn gen_ai(name: &str, path_and_query: &str) -> Option<&'static str> {
    let task = match name {
        "inference.text_embedding" | "inference.sparse_embedding" | "inference.embedding" => {
            "text_embedding"
        }
        "inference.completion" | "inference.stream_completion" => "completion",
        "inference.chat_completion_unified" => "chat_completion",
        "inference.rerank" => "rerank",
        // `/_inference/{task_type}/{inference_id}`, or without the task.
        "inference.inference" => {
            let path = path_and_query.split('?').next()?;
            let mut segments = path.trim_start_matches('/').split('/');
            segments.next();
            let task = segments.next()?;
            segments.next()?;
            task
        }
        _ => return None,
    };
    Some(match task {
        "text_embedding" | "sparse_embedding" | "embedding" => "embeddings",
        "completion" => "text_completion",
        "chat_completion" => "chat",
        "rerank" => "rerank",
        _ => return None,
    })
}

/// A request body with every value replaced by `?`, or `None` if it isn't
/// JSON.
fn sanitize(body: &Body) -> Option<String> {
    let lines: Vec<&[u8]> = match body {
        Body::Json(bytes) => vec![bytes],
        Body::Ndjson(bytes) => bytes
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
            .collect(),
    };
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        let mut value: Value = serde_json::from_slice(line).ok()?;
        mask(&mut value);
        out.push(value.to_string());
    }
    Some(out.join("\n"))
}

fn mask(value: &mut Value) {
    match value {
        Value::Object(fields) => fields.values_mut().for_each(mask),
        Value::Array(items) if items.iter().any(|i| i.is_object() || i.is_array()) => {
            items.iter_mut().for_each(mask)
        }
        _ => *value = Value::String("?".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn query_text_keeps_the_shape_only() {
        let body = Body::json(&json!({
            "query": {"bool": {"must": [{"match": {"title": "secret plans"}}], "filter": {"term": {"owner": 42}}}},
            "knn": {"field": "embedding", "query_vector": [0.1, 0.2, 0.3], "k": 10},
            "size": 5
        }))
        .unwrap();
        let text: Value = serde_json::from_str(&sanitize(&body).unwrap()).unwrap();
        assert_eq!(
            text,
            json!({
                "query": {"bool": {"must": [{"match": {"title": "?"}}], "filter": {"term": {"owner": "?"}}}},
                "knn": {"field": "?", "query_vector": "?", "k": "?"},
                "size": "?"
            })
        );
        let lines =
            Body::ndjson([json!({"index": "a"}), json!({"query": {"match_all": {}}})]).unwrap();
        assert_eq!(
            sanitize(&lines).unwrap(),
            "{\"index\":\"?\"}\n{\"query\":{\"match_all\":{}}}"
        );
    }

    #[test]
    fn collections() {
        assert_eq!(collection("/articles/_search?size=1"), Some("articles"));
        assert_eq!(collection("/_search"), None);
        assert_eq!(collection("/a%2Cb/_search"), None);
        assert_eq!(collection("/logs-*/_search"), None);
        assert_eq!(collection("/"), None);
    }

    #[test]
    fn gen_ai_operations() {
        assert_eq!(
            gen_ai("inference.text_embedding", "/_inference/text_embedding/e5"),
            Some("embeddings")
        );
        assert_eq!(
            gen_ai("inference.inference", "/_inference/rerank/r1"),
            Some("rerank")
        );
        assert_eq!(
            gen_ai("inference.inference", "/_inference/my-endpoint"),
            None
        );
        assert_eq!(
            gen_ai("inference.chat_completion_unified", "/"),
            Some("chat")
        );
        assert_eq!(gen_ai("search", "/_search"), None);
    }
}
