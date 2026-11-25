//! The document layer's updates, bulk writes and routing, against a
//! transport that records each request and answers from a script.

use std::sync::{Arc, Mutex};

use oxsearch::document::{Change, Document, Indexes, Patch, Update};
use oxsearch::fusion::Fusion;
use oxsearch::{
    Client, Error, HttpRequest, HttpResponse, Query, Refresh, Retriever, Script, Transport,
    TransportError,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Document, Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Order {
    #[id]
    id: String,
    #[routing]
    #[keyword]
    customer: Option<String>,
    total: f32,
    address: Option<Address>,
}

#[derive(Document, Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Address {
    #[keyword]
    city: String,
}

fn order(id: &str, customer: Option<&str>) -> Order {
    Order {
        id: id.to_owned(),
        customer: customer.map(str::to_owned),
        total: 0.1,
        address: None,
    }
}

/// Answers with each response in turn, and keeps every request.
#[derive(Clone, Default)]
struct Scripted {
    answers: Arc<Mutex<Vec<(u16, Value)>>>,
    requests: Arc<Mutex<Vec<(String, String)>>>,
}

impl Scripted {
    fn new(answers: impl IntoIterator<Item = (u16, Value)>) -> Scripted {
        let scripted = Scripted::default();
        *scripted.answers.lock().unwrap() = answers.into_iter().collect();
        scripted
    }

    /// The requests, as `METHOD path?query`, and their bodies.
    fn requests(&self) -> Vec<(String, String)> {
        self.requests.lock().unwrap().clone()
    }

    fn lines(&self) -> Vec<String> {
        self.requests().into_iter().map(|(line, _)| line).collect()
    }
}

impl Transport for Scripted {
    fn send(
        &self,
        request: HttpRequest,
    ) -> impl Future<Output = Result<HttpResponse, TransportError>> + Send {
        let line = format!("{} {}", request.method.as_str(), request.path_and_query);
        let body = request
            .body
            .as_ref()
            .map(|body| String::from_utf8(body.bytes().to_vec()).unwrap())
            .unwrap_or_default();
        self.requests.lock().unwrap().push((line, body));
        let (status, answer) = self.answers.lock().unwrap().remove(0);
        async move { Ok(HttpResponse::new(status, answer.to_string().into_bytes())) }
    }
}

fn written(result: &str) -> Value {
    json!({"_index": "orders", "_id": "1", "_version": 2, "result": result, "_seq_no": 7,
        "_primary_term": 1, "_shards": {"total": 1, "successful": 1, "failed": 0}})
}

fn read(routing: Option<&str>) -> Value {
    let mut read = json!({"_index": "orders", "_id": "1", "found": true, "_seq_no": 4,
        "_primary_term": 2, "_source": order("1", None)});
    if let Some(routing) = routing {
        read["_routing"] = json!(routing);
    }
    read
}

#[tokio::test]
async fn patches_scripts_and_upserts() {
    let transport = Scripted::new([
        (200, written("updated")),
        (200, written("noop")),
        (201, written("created")),
        (
            404,
            json!({"error": {"type": "document_missing_exception", "reason": "[9]: document missing"}, "status": 404}),
        ),
    ]);
    let client = Client::new(transport.clone());
    let orders = client
        .index_named::<Order>("orders")
        .refresh(Refresh::WaitFor);

    let city = Order::ADDRESS.field(Address::CITY);
    let patched = orders
        .update("1", Patch::new().set(Order::TOTAL, 2).set(city, "London"))
        .await
        .unwrap()
        .expect("the order");
    assert_eq!(patched.change, Change::Updated);
    assert_eq!(
        (patched.version.seq_no, patched.version.primary_term),
        (7, 1)
    );

    let script = Script::inline("ctx._source.total += params.by").param("by", 1);
    let unchanged = orders
        .update("1", Update::from(script).retry_on_conflict(3))
        .await
        .unwrap()
        .expect("the order");
    assert_eq!(unchanged.change, Change::Unchanged);

    // The upserted document's routing is the update's.
    let update = Update::from(Patch::new().set(Order::TOTAL, 3)).upsert(order("1", Some("ada")));
    let upserted = orders.update("1", update).await.unwrap().unwrap();
    assert_eq!(upserted.change, Change::Created);

    let missing = orders
        .update("9", Patch::new().set(Order::TOTAL, 4))
        .await
        .unwrap();
    assert_eq!(missing, None);

    let requests = transport.requests();
    let body = |n: usize| -> Value { serde_json::from_str(&requests[n].1).unwrap() };
    assert_eq!(requests[0].0, "POST /orders/_update/1?refresh=wait_for");
    assert_eq!(
        body(0),
        json!({"doc": {"total": 2, "address": {"city": "London"}}})
    );
    assert_eq!(
        requests[1].0,
        "POST /orders/_update/1?refresh=wait_for&retry_on_conflict=3"
    );
    assert_eq!(body(1)["script"]["params"], json!({"by": 1}));
    assert!(requests[2].0.contains("routing=ada"), "{}", requests[2].0);
    assert_eq!(body(2)["upsert"]["customer"], "ada");
}

#[tokio::test]
async fn an_update_conditional_on_a_version() {
    let conflict = json!({"error": {"type": "version_conflict_engine_exception", "reason": "conflict"}, "status": 409});
    let transport = Scripted::new([(200, read(Some("ada"))), (409, conflict)]);
    let client = Client::new(transport.clone());
    let orders = client.index_named::<Order>("orders").routed("ada");
    let order = orders.get("1").await.unwrap().unwrap();
    let update = Update::from(Patch::new().set(Order::TOTAL, 5)).if_version(order.version);
    let refused = orders.update("1", update).await;
    assert!(matches!(refused, Err(Error::Conflict(_))), "{refused:?}");
    let line = &transport.lines()[1];
    assert!(
        line.contains("if_seq_no=4") && line.contains("if_primary_term=2"),
        "{line}"
    );
}

#[tokio::test]
async fn bulk_writes_in_order_with_their_outcomes() {
    let item = |kind: &str, status: u16, result: &str| {
        json!({kind: {"_index": "orders", "_id": "1", "status": status, "result": result,
            "_seq_no": 9, "_primary_term": 1}})
    };
    let failed = |kind: &str, status: u16, error: &str| {
        json!({kind: {"_index": "orders", "_id": "1", "status": status,
            "error": {"type": error, "reason": "no"}}})
    };
    let transport = Scripted::new([
        (200, read(Some("ada"))),
        (
            200,
            json!({"took": 1, "errors": true, "items": [
                item("index", 201, "created"),
                failed("create", 409, "version_conflict_engine_exception"),
                failed("index", 409, "version_conflict_engine_exception"),
                item("update", 200, "noop"),
                failed("update", 404, "document_missing_exception"),
                item("delete", 404, "not_found"),
                failed("delete", 400, "document_parsing_exception"),
            ]}),
        ),
    ]);
    let client = Client::new(transport.clone());
    let orders = client.index_named::<Order>("orders");
    let mut stale = orders.get("1").await.unwrap().unwrap();
    stale.total = 9.5;

    let bulk = orders
        .bulk()
        .put(&order("1", Some("ada")))
        .create(&order("2", None))
        .put_versioned(&stale)
        .update(
            "4",
            Update::from(Patch::new().set(Order::TOTAL, 1)).retry_on_conflict(2),
        )
        .update("5", Script::inline("ctx.op = 'none'"))
        .delete("6")
        .delete_versioned(&stale);
    assert_eq!(bulk.len(), 7);
    let report = bulk.send().await.unwrap();

    assert!(!report.is_ok());
    let failed: Vec<usize> = report.failures().map(|(n, _)| n).collect();
    assert_eq!(failed, [1, 2, 6]);
    assert!(matches!(report.items[1], Err(Error::Conflict(_))));
    assert!(matches!(report.items[2], Err(Error::Conflict(_))));
    assert!(matches!(&report.items[6], Err(Error::Elasticsearch(e)) if e.status == 400));
    let change = |n: usize| report.items[n].as_ref().unwrap().change.clone();
    assert_eq!(change(0), Change::Created);
    assert_eq!(change(3), Change::Unchanged);
    assert_eq!(change(4), Change::NotFound);
    assert_eq!(change(5), Change::NotFound);

    let (line, body) = transport.requests().remove(1);
    assert_eq!(line, "POST /orders/_bulk");
    let lines: Vec<Value> = body
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines[0], json!({"index": {"_id": "1", "routing": "ada"}}));
    assert_eq!(lines[2], json!({"create": {"_id": "2"}}));
    // The document has no routing of its own: it's written where it was read.
    assert_eq!(
        lines[4],
        json!({"index": {"_id": "1", "routing": "ada", "if_seq_no": 4, "if_primary_term": 2}})
    );
    assert_eq!(
        lines[6],
        json!({"update": {"_id": "4", "retry_on_conflict": 2}})
    );
    assert_eq!(lines[7], json!({"doc": {"total": 1}}));
    assert_eq!(lines[8], json!({"update": {"_id": "5"}}));
    assert_eq!(lines[10], json!({"delete": {"_id": "6"}}));
    assert_eq!(
        lines[11],
        json!({"delete": {"_id": "1", "routing": "ada", "if_seq_no": 4, "if_primary_term": 2}})
    );
    assert_eq!(lines.len(), 12);
    // Numbers go as the document writes them, not through a `Value`.
    assert!(body.contains("\"total\":0.1,"), "{body}");
}

#[tokio::test]
async fn an_empty_bulk_sends_nothing() {
    let transport = Scripted::default();
    let client = Client::new(transport.clone());
    let report = client
        .index_named::<Order>("orders")
        .bulk()
        .send()
        .await
        .unwrap();
    assert!(report.is_ok() && report.items.is_empty());
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn what_isnt_json_fails_before_anything_is_sent() {
    let transport = Scripted::default();
    let client = Client::new(transport.clone());
    let orders = client.index_named::<Order>("orders");
    let sent = orders
        .bulk()
        .put(&order("1", None))
        .update("1", Patch::new().merge("not an object"))
        .send()
        .await;
    assert!(matches!(sent, Err(Error::Encode(_))), "{sent:?}");
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn routing_reaches_every_request() {
    let empty = json!({"took": 1, "timed_out": false, "_shards": {"total": 1, "successful": 1, "failed": 0},
        "hits": {"hits": []}});
    let unlicensed = json!({"error": {"type": "security_exception",
        "reason": "current license is non-compliant for [rrf]",
        "license.expired.feature": "rrf"}, "status": 403});
    let transport = Scripted::new([
        (200, read(Some("ada"))),
        (200, json!({"docs": [read(Some("ada"))]})),
        (200, written("updated")),
        (200, written("deleted")),
        (200, written("deleted")),
        (200, written("updated")),
        (200, written("updated")),
        (200, empty.clone()),
        (403, unlicensed),
        (200, json!({"took": 1, "responses": [empty]})),
    ]);
    let client = Client::new(transport.clone());
    let all = client.index_named::<Order>("orders");
    let orders = all.clone().routed("ada");

    let order_1 = orders.get("1").await.unwrap().unwrap();
    orders.get_many(["1"]).await.unwrap();
    orders
        .update("1", Patch::new().set(Order::TOTAL, 1))
        .await
        .unwrap();
    assert!(orders.delete("1").await.unwrap());
    // A document that was read is written and deleted where it was,
    // whatever the index's routing.
    all.delete_versioned(&order_1).await.unwrap();
    all.put_versioned(&order_1).await.unwrap();
    // A document's own routing comes first.
    orders.put(&order("2", Some("bob"))).await.unwrap();
    orders.search(Query::match_all()).await.unwrap();
    let fused = orders
        .fuse(&Fusion::rrf([Retriever::standard(Query::match_all())]))
        .await
        .unwrap();
    assert!(fused.fallback.is_some());

    let requests = transport.requests();
    for (n, (line, _)) in requests.iter().enumerate().take(9) {
        let routing = if n == 6 { "routing=bob" } else { "routing=ada" };
        assert!(line.contains(routing), "{n}: {line}");
    }
    // The multi-search has it on each header.
    let header: Value = serde_json::from_str(requests[9].1.lines().next().unwrap()).unwrap();
    assert_eq!(header["routing"], "ada");
}
