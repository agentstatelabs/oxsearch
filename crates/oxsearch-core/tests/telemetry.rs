//! What the client records through `tracing` and `metrics`, against a
//! scripted transport, with names checked against oxtel-semconv's
//! constants.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use oxsearch_core::{
    Body, Client, HttpRequest, HttpResponse, Method, Transport, TransportError, telemetry,
};
use oxtel_semconv::attribute as semconv;
use oxtel_semconv::genai::attribute as genai;
use serde_json::json;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt};

/// A span's fields by name.
type Fieldset = BTreeMap<String, String>;

/// Every span's fields, in the order the spans opened.
#[derive(Clone, Default)]
struct Spans(Arc<Mutex<Vec<(Id, Fieldset)>>>);

struct Fields<'a>(&'a mut BTreeMap<String, String>);

impl Visit for Fields<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }
}

impl<S: tracing::Subscriber> Layer<S> for Spans {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, _: Context<'_, S>) {
        let mut fields = BTreeMap::new();
        attrs.record(&mut Fields(&mut fields));
        self.0.lock().unwrap().push((id.clone(), fields));
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, _: Context<'_, S>) {
        let mut spans = self.0.lock().unwrap();
        if let Some((_, fields)) = spans.iter_mut().rev().find(|(i, _)| i == id) {
            values.record(&mut Fields(fields));
        }
    }
}

impl Spans {
    fn last(&self) -> Fieldset {
        self.0.lock().unwrap().last().unwrap().1.clone()
    }
}

/// Answers by path.
#[derive(Clone)]
struct Scripted;

impl Transport for Scripted {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let (status, body) = if request.path_and_query.starts_with("/missing") {
            (
                404,
                json!({"error": {"type": "index_not_found_exception", "reason": "no such index"}, "status": 404}),
            )
        } else {
            (200, json!({"hits": {"hits": []}}))
        };
        Ok(HttpResponse::new(status, body.to_string().into_bytes()))
    }
}

/// A request a generated endpoint would make, by name.
struct Call {
    name: &'static str,
    method: Method,
    path: &'static str,
    body: Option<serde_json::Value>,
}

impl oxsearch_core::Endpoint for Call {
    type Response = serde_json::Value;

    fn name(&self) -> &'static str {
        self.name
    }

    fn route(&self) -> Result<oxsearch_core::Route, oxsearch_core::Error> {
        Ok(oxsearch_core::Route::new(self.method, self.path))
    }

    fn query(&self) -> Result<Vec<(&'static str, String)>, oxsearch_core::Error> {
        Ok(Vec::new())
    }

    fn body(&self) -> Result<Option<Body>, oxsearch_core::Error> {
        self.body.as_ref().map(Body::json).transpose()
    }
}

#[test]
fn spans_and_durations() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    metrics::set_global_recorder(recorder).unwrap();
    let spans = Spans::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(spans.clone()));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let client = Client::new(Scripted);

    // A search: named by its endpoint, on one index, with its query's shape.
    runtime
        .block_on(client.send(&Call {
            name: "search",
            method: Method::Post,
            path: "/articles/_search",
            body: Some(json!({"query": {"match": {"title": "private words"}}, "size": 3})),
        }))
        .unwrap();
    let span = spans.last();
    assert_eq!(span["otel.name"], "search");
    assert_eq!(span["otel.kind"], "client");
    assert_eq!(span[semconv::DB_SYSTEM_NAME], "elasticsearch");
    assert_eq!(span[semconv::DB_OPERATION_NAME], "search");
    assert_eq!(span[semconv::DB_COLLECTION_NAME], "articles");
    assert_eq!(span[semconv::HTTP_REQUEST_METHOD], "POST");
    assert_eq!(span[semconv::DB_RESPONSE_STATUS_CODE], "200");
    assert_eq!(
        span[semconv::DB_QUERY_TEXT],
        r#"{"query":{"match":{"title":"?"}},"size":"?"}"#
    );
    assert!(!span.contains_key(semconv::ERROR_TYPE));

    // A failure: the error's type, and an error status.
    let error = runtime.block_on(client.send(&Call {
        name: "search",
        method: Method::Post,
        path: "/missing/_search",
        body: None,
    }));
    assert!(error.is_err());
    let span = spans.last();
    assert_eq!(span[semconv::ERROR_TYPE], "index_not_found_exception");
    assert_eq!(span["otel.status_code"], "error");
    assert_eq!(span[semconv::DB_RESPONSE_STATUS_CODE], "404");

    // An inference call: what a GenAI client would call it.
    runtime
        .block_on(client.send(&Call {
            name: "inference.text_embedding",
            method: Method::Post,
            path: "/_inference/text_embedding/e5",
            body: Some(json!({"input": ["hello"]})),
        }))
        .unwrap();
    let span = spans.last();
    assert_eq!(span[genai::GEN_AI_OPERATION_NAME], "embeddings");
    assert!(!span.contains_key(semconv::DB_QUERY_TEXT));
    assert!(!span.contains_key(semconv::DB_COLLECTION_NAME));

    // Query text can be turned off.
    telemetry::record_query_text(false);
    runtime
        .block_on(client.send(&Call {
            name: "search",
            method: Method::Post,
            path: "/articles/_search",
            body: Some(json!({"query": {"match_all": {}}})),
        }))
        .unwrap();
    assert!(!spans.last().contains_key(semconv::DB_QUERY_TEXT));
    telemetry::record_query_text(true);

    // A raw call: no endpoint name, so named by the system.
    runtime
        .block_on(client.send_raw(Method::Get, "/_cluster/health", None))
        .unwrap();
    let span = spans.last();
    assert_eq!(span["otel.name"], "elasticsearch");
    assert!(!span.contains_key(semconv::DB_OPERATION_NAME));

    // Every call's duration, by operation and, for failures, error type.
    let histograms: Vec<(Vec<(String, String)>, usize)> = snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter(|(key, ..)| {
            key.key().name() == oxtel_semconv::metric::DB_CLIENT_OPERATION_DURATION.name
        })
        .map(|(key, _, _, value)| {
            let labels = key
                .key()
                .labels()
                .map(|l| (l.key().to_owned(), l.value().to_owned()))
                .collect();
            let DebugValue::Histogram(values) = value else {
                panic!("a histogram")
            };
            (labels, values.len())
        })
        .collect();
    let search_ok = vec![
        (
            semconv::DB_SYSTEM_NAME.to_owned(),
            "elasticsearch".to_owned(),
        ),
        (semconv::DB_OPERATION_NAME.to_owned(), "search".to_owned()),
    ];
    assert!(histograms.contains(&(search_ok, 2)), "{histograms:?}");
    let failed = vec![
        (
            semconv::DB_SYSTEM_NAME.to_owned(),
            "elasticsearch".to_owned(),
        ),
        (semconv::DB_OPERATION_NAME.to_owned(), "search".to_owned()),
        (
            semconv::ERROR_TYPE.to_owned(),
            "index_not_found_exception".to_owned(),
        ),
        (
            semconv::DB_RESPONSE_STATUS_CODE.to_owned(),
            "404".to_owned(),
        ),
    ];
    assert!(histograms.contains(&(failed, 1)), "{histograms:?}");
}

#[test]
fn transport_fields_have_their_conventional_names() {
    // The transport records these into the client span by name.
    assert_eq!(semconv::URL_FULL, "url.full");
    assert_eq!(semconv::SERVER_ADDRESS, "server.address");
    assert_eq!(semconv::SERVER_PORT, "server.port");
    assert_eq!(semconv::ELASTICSEARCH_NODE_NAME, "elasticsearch.node.name");
}
