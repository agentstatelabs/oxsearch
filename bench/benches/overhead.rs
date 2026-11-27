//! Request overhead: one search and one write through each client, over
//! HTTP to a local server that answers at once with a canned response, so
//! what's measured is the clients' own work: building the request, the
//! HTTP exchange through reqwest, and reading the response.

use criterion::{Criterion, criterion_group, criterion_main};
use elasticsearch::http::transport::Transport;
use elasticsearch::{Elasticsearch, IndexParts, SearchParts};
use oxsearch::Query;
use oxsearch::docs::index;
use oxsearch::search::search;
use oxsearch::transport_reqwest::ReqwestTransport;
use oxsearch_bench::{
    Doc, MinimalSearch, MinimalWrite, doc, index_response, search_response, serve,
};
use serde_json::{Value, json};

fn clients(url: &str) -> (oxsearch::Client<ReqwestTransport>, Elasticsearch) {
    let ours = oxsearch::Client::new(ReqwestTransport::new(url).unwrap());
    let theirs = Elasticsearch::new(Transport::single_node(url).unwrap());
    (ours, theirs)
}

fn search_10_hits(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let url = runtime.block_on(serve(search_response(10)));
    let (ours, theirs) = clients(&url);
    let mut group = c.benchmark_group("search, 10 hits");
    group.bench_function("oxsearch, typed response", |b| {
        b.to_async(&runtime).iter(|| async {
            let request = search::Request::new().index("bench").body(
                search::RequestBody::new()
                    .query(Query::r#match("title", "hnsw"))
                    .size(10),
            );
            let found: search::Response<Doc> = ours.send_as(&request).await.unwrap();
            assert_eq!(found.into_results().len(), 10);
        })
    });
    group.bench_function("oxsearch, hand-written response", |b| {
        b.to_async(&runtime).iter(|| async {
            let request = search::Request::new().index("bench").body(
                search::RequestBody::new()
                    .query(Query::r#match("title", "hnsw"))
                    .size(10),
            );
            let found: MinimalSearch<Doc> = ours.send_as(&request).await.unwrap();
            assert_eq!(found.hits.hits.len(), 10);
        })
    });
    group.bench_function("oxsearch, send_raw into Value", |b| {
        b.to_async(&runtime).iter(|| async {
            let body = json!({"query": {"match": {"title": "hnsw"}}, "size": 10});
            let found = ours
                .send_raw(oxsearch::Method::Post, "/bench/_search", Some(&body))
                .await
                .unwrap();
            assert_eq!(found["hits"]["hits"].as_array().unwrap().len(), 10);
        })
    });
    group.bench_function("official, hand-written response", |b| {
        b.to_async(&runtime).iter(|| async {
            let response = theirs
                .search(SearchParts::Index(&["bench"]))
                .body(json!({"query": {"match": {"title": "hnsw"}}, "size": 10}))
                .send()
                .await
                .unwrap();
            let found: MinimalSearch<Doc> = response.json().await.unwrap();
            assert_eq!(found.hits.hits.len(), 10);
        })
    });
    group.bench_function("official, Value", |b| {
        b.to_async(&runtime).iter(|| async {
            let response = theirs
                .search(SearchParts::Index(&["bench"]))
                .body(json!({"query": {"match": {"title": "hnsw"}}, "size": 10}))
                .send()
                .await
                .unwrap();
            let found: Value = response.json().await.unwrap();
            assert_eq!(found["hits"]["hits"].as_array().unwrap().len(), 10);
        })
    });
    group.finish();
}

fn index_one(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let url = runtime.block_on(serve(index_response()));
    let (ours, theirs) = clients(&url);
    let document = doc(1);
    let mut group = c.benchmark_group("index one document");
    group.bench_function("oxsearch, typed response", |b| {
        b.to_async(&runtime).iter(|| async {
            let request = index::Request::new("bench").id("1").body(&document);
            let written = ours.send(&request).await.unwrap();
            assert_eq!(written.0.id.as_deref(), Some("1"));
        })
    });
    group.bench_function("official, hand-written response", |b| {
        b.to_async(&runtime).iter(|| async {
            let response = theirs
                .index(IndexParts::IndexId("bench", "1"))
                .body(&document)
                .send()
                .await
                .unwrap();
            let written: MinimalWrite = response.json().await.unwrap();
            assert_eq!(written.id, "1");
        })
    });
    group.finish();
}

criterion_group!(benches, search_10_hits, index_one);
criterion_main!(benches);
