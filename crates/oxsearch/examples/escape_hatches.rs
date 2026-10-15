//! The escape hatches, for what oxsearch doesn't cover yet:
//!
//! 1. your own endpoint, sent like a generated one;
//! 2. raw variants, for a query, retriever or value the types don't know;
//! 3. a raw call, for any request at all.
//!
//! It runs against Elasticsearch at `OXSEARCH_URL` (default
//! `http://localhost:9200`), such as the test fixture:
//!
//! ```text
//! scripts/es-fixture.sh up
//! cargo run -p oxsearch --example escape_hatches --features transport-reqwest
//! ```

use oxsearch::docs::index;
use oxsearch::query_dsl::QueryContainer;
use oxsearch::search::search;
use oxsearch::transport_reqwest::ReqwestTransport;
use oxsearch::{
    Body, Client, Endpoint, Error, Method, Refresh, RetrieverContainer, Route, SortOrder,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const INDEX: &str = "oxsearch-example-escape-hatches";

/// 1. An endpoint of your own: creating an index, whose API isn't
///    generated yet. It goes through the client like any other, with the
///    same transport, errors and, later, middleware.
struct CreateIndex<'a> {
    name: &'a str,
    mappings: Value,
}

/// What Elasticsearch answers, in your own type.
#[derive(Debug, Deserialize)]
struct Created {
    acknowledged: bool,
    index: String,
}

impl Endpoint for CreateIndex<'_> {
    type Response = Created;

    fn name(&self) -> &'static str {
        "indices.create"
    }

    fn route(&self) -> Result<Route, Error> {
        // Each segment is percent-encoded.
        Ok(Route::segments(Method::Put, [self.name]))
    }

    fn query(&self) -> Result<Vec<(&'static str, String)>, Error> {
        Ok(vec![("wait_for_active_shards", "1".to_owned())])
    }

    fn body(&self) -> Result<Option<Body>, Error> {
        Body::json(&json!({ "mappings": self.mappings })).map(Some)
    }
}

/// Another: counting an index's documents with the cat API, whose rows
/// come back as an array.
struct CatCount<'a> {
    index: &'a str,
}

#[derive(Debug, Deserialize)]
struct CountRow {
    count: String,
}

impl Endpoint for CatCount<'_> {
    type Response = Vec<CountRow>;

    fn name(&self) -> &'static str {
        "cat.count"
    }

    fn route(&self) -> Result<Route, Error> {
        Ok(Route::segments(Method::Get, ["_cat", "count", self.index]))
    }

    fn query(&self) -> Result<Vec<(&'static str, String)>, Error> {
        Ok(vec![("format", "json".to_owned())])
    }

    fn body(&self) -> Result<Option<Body>, Error> {
        Ok(None)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Note {
    text: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let url = std::env::var("OXSEARCH_URL").unwrap_or_else(|_| "http://localhost:9200".to_owned());
    let client = Client::new(ReqwestTransport::new(&url)?);
    let _ = client
        .send_raw(Method::Delete, &format!("/{INDEX}"), None)
        .await;

    // 1. Your own endpoints.
    let created = client
        .send(&CreateIndex {
            name: INDEX,
            mappings: json!({ "properties": { "text": { "type": "text" } } }),
        })
        .await?;
    println!(
        "created {} (acknowledged: {})",
        created.index, created.acknowledged
    );
    for (n, text) in ["tuning hnsw graphs", "bulk loading at speed"]
        .iter()
        .enumerate()
    {
        client
            .send(
                &index::Request::new(INDEX)
                    .id(n.to_string())
                    .refresh(Refresh::True)
                    .body(Note {
                        text: (*text).to_owned(),
                    }),
            )
            .await?;
    }
    let rows = client.send(&CatCount { index: INDEX }).await?;
    println!("{} documents", rows[0].count);

    // 2. Raw variants. A query the types don't know, written as JSON...
    let query_string = QueryContainer::Raw(json!({
        "query_string": { "query": "hnsw OR graphs", "default_field": "text" }
    }));
    // ...inside a retriever written as JSON too.
    let retriever = RetrieverContainer::Raw(json!({
        "standard": { "query": query_string }
    }));
    let request = search::Request::new()
        .index(INDEX)
        .body(search::RequestBody::new().retriever(retriever));
    let response: search::Response<Note> = client.send_as(&request).await?;
    for note in response.documents() {
        println!("found: {}", note.text);
    }
    // A value a newer Elasticsearch writes still reads: as a raw variant.
    let order: SortOrder = serde_json::from_value(json!("sideways"))?;
    println!("an unknown sort order reads as {order:?}");

    // 3. A raw call, for anything else.
    let health = client
        .send_raw(Method::Get, "/_cluster/health", None)
        .await?;
    println!("cluster status: {}", health["status"]);

    client
        .send_raw(Method::Delete, &format!("/{INDEX}"), None)
        .await?;
    Ok(())
}
