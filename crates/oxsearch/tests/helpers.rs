//! The point-in-time scan and the bulk loader against a scripted transport:
//! paging, slices, closing, batching, backoff and stopping.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::TryStreamExt;
use oxsearch::helpers::{Action, BulkLoader, Scan, SendError};
use oxsearch::{Client, Error, HttpRequest, HttpResponse, Transport, TransportError};
use serde_json::{Value, json};

type Handler = dyn Fn(&HttpRequest, &Value) -> (u16, Value) + Send + Sync;

/// Answers each request with what a function of it says, and keeps every
/// request for the test to check.
#[derive(Clone)]
struct Scripted {
    handler: Arc<Handler>,
    requests: Arc<Mutex<Vec<(String, Value)>>>,
}

impl Scripted {
    fn new(handler: impl Fn(&HttpRequest, &Value) -> (u16, Value) + Send + Sync + 'static) -> Self {
        Scripted {
            handler: Arc::new(handler),
            requests: Arc::default(),
        }
    }

    fn requests(&self) -> Vec<(String, Value)> {
        self.requests.lock().unwrap().clone()
    }
}

/// A request's body: JSON, or NDJSON as an array of its lines.
fn body(request: &HttpRequest) -> Value {
    let Some(body) = &request.body else {
        return Value::Null;
    };
    let text = String::from_utf8(body.bytes().to_vec()).unwrap();
    if body.content_type() == "application/x-ndjson" {
        Value::Array(
            text.lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect(),
        )
    } else {
        serde_json::from_str(&text).unwrap()
    }
}

impl Transport for Scripted {
    fn send(
        &self,
        request: HttpRequest,
    ) -> impl Future<Output = Result<HttpResponse, TransportError>> + Send {
        let body = body(&request);
        let (status, answer) = (self.handler)(&request, &body);
        let line = format!("{} {}", request.method.as_str(), request.path_and_query);
        self.requests.lock().unwrap().push((line, body));
        async move { Ok(HttpResponse::new(status, answer.to_string().into_bytes())) }
    }
}

fn hit(id: &str, sort: i64) -> Value {
    json!({"_index": "idx", "_id": id, "_score": null, "_source": {"title": id}, "sort": [sort]})
}

#[tokio::test]
async fn scans_page_by_page_and_closes() {
    let transport = Scripted::new(|request, body| {
        match (request.method.as_str(), request.path_and_query.as_str()) {
            ("POST", "/idx/_pit?keep_alive=1m") => (200, json!({"id": "pit-1"})),
            ("POST", "/_search") => {
                let page = match body.get("search_after") {
                    None => json!([hit("a", 1), hit("b", 2)]),
                    Some(after) if after == &json!([2]) => json!([hit("c", 3)]),
                    Some(_) => json!([]),
                };
                let pit = format!(
                    "pit-{}",
                    2 + body["search_after"]
                        .as_array()
                        .map_or(0, |a| a[0].as_i64().unwrap())
                );
                (200, json!({"pit_id": pit, "hits": {"hits": page}}))
            }
            ("DELETE", "/_pit") => (200, json!({"succeeded": true, "num_freed": 1})),
            other => panic!("unexpected {other:?}"),
        }
    });
    let client = Client::new(transport.clone());
    let hits: Vec<_> = Scan::new("idx")
        .page_size(2)
        .stream::<Value, _>(&client)
        .try_collect()
        .await
        .unwrap();
    let ids: Vec<_> = hits.iter().map(|h| h.id.clone().unwrap()).collect();
    assert_eq!(ids, ["a", "b", "c"]);

    let requests = transport.requests();
    let lines: Vec<&str> = requests.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(
        lines,
        [
            "POST /idx/_pit?keep_alive=1m",
            "POST /_search",
            "POST /_search",
            "POST /_search",
            "DELETE /_pit"
        ]
    );
    let first = &requests[1].1;
    assert_eq!(first["pit"], json!({"id": "pit-1", "keep_alive": "1m"}));
    assert_eq!(first["sort"], json!("_shard_doc"));
    assert_eq!(first["size"], 2);
    assert_eq!(first["track_total_hits"], false);
    // Each page uses the latest id, and the last one is closed.
    assert_eq!(requests[2].1["pit"]["id"], "pit-2");
    assert_eq!(requests[3].1["pit"]["id"], "pit-4");
    assert_eq!(requests[4].1, json!({"id": "pit-5"}));
}

#[tokio::test]
async fn scans_in_slices() {
    let transport = Scripted::new(|request, body| match request.path_and_query.as_str() {
        "/idx/_pit?keep_alive=1m" => (200, json!({"id": "pit"})),
        "/_search" => {
            let slice = body["slice"]["id"].as_str().unwrap().to_owned();
            let page = if body.get("search_after").is_some() {
                json!([])
            } else {
                json!([hit(&format!("doc-{slice}"), 1)])
            };
            (200, json!({"hits": {"hits": page}}))
        }
        _ => (200, json!({"succeeded": true, "num_freed": 1})),
    });
    let client = Client::new(transport.clone());
    let hits: Vec<_> = Scan::new("idx")
        .slices(2)
        .stream::<Value, _>(&client)
        .try_collect()
        .await
        .unwrap();
    let mut ids: Vec<_> = hits.iter().map(|h| h.id.clone().unwrap()).collect();
    ids.sort();
    assert_eq!(ids, ["doc-0", "doc-1"]);
    let slices: Vec<Value> = transport
        .requests()
        .iter()
        .filter(|(l, b)| l == "POST /_search" && b.get("search_after").is_none())
        .map(|(_, b)| b["slice"].clone())
        .collect();
    assert!(slices.contains(&json!({"id": "0", "max": 2})));
    assert!(slices.contains(&json!({"id": "1", "max": 2})));
}

#[tokio::test]
async fn a_failed_page_ends_the_scan_and_closes() {
    let transport = Scripted::new(|request, _| match request.path_and_query.as_str() {
        "/idx/_pit?keep_alive=1m" => (200, json!({"id": "pit"})),
        "/_search" => (
            500,
            json!({"error": {"type": "search_phase_execution_exception"}, "status": 500}),
        ),
        _ => (200, json!({"succeeded": true, "num_freed": 1})),
    });
    let client = Client::new(transport.clone());
    let result: Result<Vec<_>, _> = Scan::new("idx")
        .stream::<Value, _>(&client)
        .try_collect()
        .await;
    assert!(matches!(result, Err(Error::Elasticsearch(e)) if e.status == 500));
    assert_eq!(transport.requests().last().unwrap().0, "DELETE /_pit");
}

/// A bulk response with an item per action, by status.
fn items(body: &Value, statuses: &[u16]) -> Value {
    let actions = body
        .as_array()
        .unwrap()
        .iter()
        .filter(|line| line.get("index").is_some());
    let items: Vec<Value> = actions
        .zip(statuses)
        .map(|(action, status)| {
            let mut item = json!({"_index": "idx", "_id": action["index"]["_id"], "status": status});
            if *status >= 300 {
                item["error"] = json!({"type": if *status == 429 { "es_rejected_execution_exception" } else { "document_parsing_exception" }});
            } else {
                item["result"] = json!("created");
            }
            json!({"index": item})
        })
        .collect();
    json!({"took": 1, "errors": statuses.iter().any(|s| *s >= 300), "items": items})
}

fn loader(transport: &Scripted) -> BulkLoader<Scripted> {
    BulkLoader::new(&Client::new(transport.clone()))
        .index("idx")
        .retries(3, Duration::from_millis(1), Duration::from_millis(2))
}

#[tokio::test]
async fn loads_in_batches() {
    let transport = Scripted::new(|_, body| {
        let n = body.as_array().unwrap().len() / 2;
        (200, items(body, &vec![201; n]))
    });
    let (sender, results) = loader(&transport)
        .max_actions(2)
        .flush_interval(Duration::from_secs(60))
        .start();
    for n in 0..5 {
        sender
            .send(Action::index(json!({"n": n})).id(n.to_string()))
            .await
            .unwrap();
    }
    drop(sender);
    let summary = results.summary().await.unwrap();
    assert_eq!(summary.succeeded, 5);
    assert!(summary.failed.is_empty());
    let sizes: Vec<usize> = transport
        .requests()
        .iter()
        .map(|(line, body)| {
            assert_eq!(line, "POST /idx/_bulk");
            body.as_array().unwrap().len() / 2
        })
        .collect();
    assert_eq!(sizes.iter().sum::<usize>(), 5);
    assert!(sizes.iter().all(|n| *n <= 2));
}

#[tokio::test]
async fn retries_what_is_refused_with_429() {
    let calls = Arc::new(Mutex::new(0));
    let counter = calls.clone();
    let transport = Scripted::new(move |_, body| {
        let mut calls = counter.lock().unwrap();
        *calls += 1;
        match *calls {
            // The whole batch, then one of its actions, is refused.
            1 => (
                429,
                json!({"error": {"type": "es_rejected_execution_exception"}, "status": 429}),
            ),
            2 => (200, items(body, &[201, 429])),
            _ => (200, items(body, &[201])),
        }
    });
    let (sender, results) = loader(&transport).start();
    sender.send(Action::index(json!({})).id("1")).await.unwrap();
    sender.send(Action::index(json!({})).id("2")).await.unwrap();
    drop(sender);
    let summary = results.summary().await.unwrap();
    assert_eq!(summary.succeeded, 2);
    let requests = transport.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[2].1[0]["index"]["_id"], "2");
}

#[tokio::test]
async fn refused_documents_are_outcomes() {
    let transport = Scripted::new(|_, body| (200, items(body, &[201, 400])));
    let (sender, results) = loader(&transport).start();
    sender
        .send(Action::index(json!({"ok": true})).id("1"))
        .await
        .unwrap();
    sender
        .send(Action::index(json!({"ok": false})).id("2"))
        .await
        .unwrap();
    drop(sender);
    let summary = results.summary().await.unwrap();
    assert_eq!(summary.succeeded, 1);
    assert_eq!(summary.failed.len(), 1);
    assert_eq!(summary.failed[0].id.as_deref(), Some("2"));
    assert_eq!(summary.failed[0].position, 1);
}

#[tokio::test]
async fn a_failed_batch_stops_the_loader() {
    let transport = Scripted::new(|_, _| {
        (
            400,
            json!({"error": {"type": "illegal_argument_exception", "reason": "no"}, "status": 400}),
        )
    });
    let (sender, mut results) = loader(&transport).start();
    sender.send(Action::index(json!({})).id("1")).await.unwrap();
    let error = tokio::time::timeout(Duration::from_secs(5), results.next())
        .await
        .unwrap();
    assert!(matches!(error, Some(Err(Error::Elasticsearch(e))) if e.status == 400));
    assert!(results.next().await.is_none());
    assert!(matches!(
        sender.send(Action::index(json!({}))).await,
        Err(SendError::Stopped)
    ));
}

#[tokio::test]
async fn flushes_after_the_interval() {
    let transport = Scripted::new(|_, body| (200, items(body, &[201])));
    let (sender, mut results) = loader(&transport)
        .flush_interval(Duration::from_millis(20))
        .start();
    sender.send(Action::index(json!({})).id("1")).await.unwrap();
    // The sender is still open: only the interval sends the batch.
    let outcome = tokio::time::timeout(Duration::from_secs(5), results.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(outcome.is_ok());
    assert_eq!(outcome.id.as_deref(), Some("1"));
    drop(sender);
    assert!(results.next().await.is_none());
}
