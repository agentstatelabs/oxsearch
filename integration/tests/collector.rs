//! oxsearch's telemetry, exported by oxtel to a real OpenTelemetry
//! Collector, as the Collector decodes it: the client span of a search and
//! its duration. CI's integration jobs start the Collector; to run it
//! locally:
//!
//! ```text
//! mkdir -p integration/collector-out && chmod 777 integration/collector-out
//! docker run -d --name otelcol -p 4318:4318 -p 13133:13133 \
//!   -v "$PWD/integration/collector.yaml:/etc/otelcol-contrib/config.yaml:ro" \
//!   -v "$PWD/integration/collector-out:/out" \
//!   otel/opentelemetry-collector-contrib:0.161.0
//! cargo test -p oxsearch-integration --test collector -- --ignored
//! ```

use std::path::PathBuf;
use std::time::{Duration, Instant};

use oxsearch::Retriever;
use oxsearch::search::search;
use oxsearch_integration::fixtures::{article_mapping, articles};
use oxsearch_integration::{TestIndex, client};
use oxtel_otlp::OtlpHttpExporter;
use oxtel_sdk::metrics::{MeterProvider, PeriodicReaderConfig};
use oxtel_sdk::trace::{Sampler, TracerProvider};
use oxtel_sdk::{KeyValue, Resource, TraceId};
use serde_json::Value;
use tracing_subscriber::layer::SubscriberExt;

fn otlp() -> String {
    std::env::var("OXSEARCH_COLLECTOR_OTLP").unwrap_or_else(|_| "http://localhost:4318".into())
}

fn out_dir() -> PathBuf {
    std::env::var("OXSEARCH_COLLECTOR_OUT").map_or_else(
        |_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("collector-out"),
        PathBuf::from,
    )
}

/// The JSON lines in `file` that mention `marker`, once there are any.
fn lines_with(file: &str, marker: &str) -> Vec<Value> {
    let path = out_dir().join(file);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let found: Vec<Value> = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(marker))
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        if !found.is_empty() {
            return found;
        }
        assert!(
            Instant::now() < deadline,
            "nothing with {marker} in {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// An attribute's OTLP/JSON value, such as `{"stringValue": "search"}`.
fn attr<'a>(attributes: &'a Value, key: &str) -> &'a Value {
    attributes
        .as_array()
        .and_then(|a| a.iter().find(|kv| kv["key"] == key))
        .map_or(&Value::Null, |kv| &kv["value"])
}

fn resource(run: &str) -> Resource {
    Resource::builder()
        .without_env()
        .with_service_name("oxsearch-integration")
        .with_attribute(KeyValue::new("test.run", run.to_owned()))
        .build()
}

#[test]
#[ignore = "needs the Elasticsearch fixture and an OpenTelemetry Collector; see this file"]
fn a_search_reaches_the_collector() {
    let run = TraceId::random().to_string();
    let tracer = TracerProvider::builder()
        .with_resource(resource(&run))
        .with_sampler(Sampler::AlwaysOn)
        .with_batch_exporter(OtlpHttpExporter::builder().with_endpoint(&otlp()).build())
        .build();
    let meters = MeterProvider::builder()
        .with_resource(resource(&run))
        .with_periodic_exporter_config(
            OtlpHttpExporter::builder().with_endpoint(&otlp()).build(),
            PeriodicReaderConfig {
                interval: Duration::from_secs(3600),
                timeout: Duration::from_secs(20),
            },
        )
        .build();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = client();
    let test = runtime.block_on(TestIndex::create(&client, "telemetry", &article_mapping()));
    let index = test.name().to_owned();
    runtime.block_on(async {
        for (id, article) in articles() {
            client
                .send(
                    &oxsearch::docs::index::Request::new(index.as_str())
                        .id(id)
                        .refresh(oxsearch::Refresh::True)
                        .body(article),
                )
                .await
                .unwrap();
        }
    });

    let subscriber = tracing_subscriber::registry().with(tracer.layer());
    tracing::subscriber::with_default(subscriber, || {
        metrics::with_local_recorder(&meters.recorder(), || {
            oxsearch::telemetry::describe_metrics();
            let request = search::Request::new().index(index.as_str()).body(
                search::RequestBody::new().retriever(Retriever::standard(
                    oxsearch::Query::r#match("title", "hnsw"),
                )),
            );
            runtime.block_on(client.send(&request)).unwrap();
        })
    });
    tracer.force_flush().expect("the Collector took the spans");
    meters
        .force_flush()
        .expect("the Collector took the metrics");
    runtime.block_on(test.delete(&client));

    let traces = lines_with("traces.json", &run);
    let span = traces
        .iter()
        .flat_map(|r| r["resourceSpans"].as_array().into_iter().flatten())
        .flat_map(|rs| rs["scopeSpans"].as_array().into_iter().flatten())
        .flat_map(|ss| ss["spans"].as_array().into_iter().flatten())
        .find(|s| s["name"] == "search")
        .unwrap_or_else(|| panic!("no search span in {traces:?}"));
    assert_eq!(span["kind"], 3, "a client span");
    let attributes = &span["attributes"];
    assert_eq!(
        attr(attributes, "db.system.name")["stringValue"],
        "elasticsearch"
    );
    assert_eq!(
        attr(attributes, "db.operation.name")["stringValue"],
        "search"
    );
    assert_eq!(
        attr(attributes, "db.collection.name")["stringValue"],
        index.as_str()
    );
    assert_eq!(
        attr(attributes, "db.response.status_code")["stringValue"],
        "200"
    );
    assert_eq!(
        attr(attributes, "http.request.method")["stringValue"],
        "POST"
    );
    assert_eq!(
        attr(attributes, "server.address")["stringValue"],
        "127.0.0.1"
    );
    assert!(attr(attributes, "server.port")["intValue"].is_string());
    assert!(
        attr(attributes, "url.full")["stringValue"]
            .as_str()
            .unwrap()
            .ends_with("/_search")
    );
    assert_eq!(
        attr(attributes, "db.query.text")["stringValue"],
        r#"{"retriever":{"standard":{"query":{"match":{"title":"?"}}}}}"#
    );

    let metrics = lines_with("metrics.json", &run);
    let duration = metrics
        .iter()
        .flat_map(|b| b["resourceMetrics"].as_array().into_iter().flatten())
        .flat_map(|rm| rm["scopeMetrics"].as_array().into_iter().flatten())
        .flat_map(|sm| sm["metrics"].as_array().into_iter().flatten())
        .find(|m| m["name"] == "db.client.operation.duration")
        .unwrap_or_else(|| panic!("no duration in {metrics:?}"));
    assert_eq!(duration["unit"], "s");
    let point = duration["histogram"]["dataPoints"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| attr(&p["attributes"], "db.operation.name")["stringValue"] == "search")
        .expect("the search's duration");
    assert_eq!(point["count"], "1");
    assert_eq!(
        attr(&point["attributes"], "db.system.name")["stringValue"],
        "elasticsearch"
    );
}
