//! Aggregations against Elasticsearch, by family: buckets, metrics and
//! pipelines, built with `Agg`, sent with `typed_keys`, and read back as
//! typed results.

use oxsearch::aggs::{Agg, Aggregates, Aggs, CalendarInterval, Results};
use oxsearch::search::search;
use oxsearch::{Method, Query, Refresh};
use oxsearch_docs::index;
use oxsearch_integration::{TestClient, TestIndex, client};
use serde_json::{Value, json};

/// Eight products over three categories and three months.
async fn products(client: &TestClient) -> TestIndex {
    let test = TestIndex::named("aggs");
    client
        .send_raw(
            Method::Put,
            &format!("/{}", test.name()),
            Some(&json!({"mappings": {"properties": {
                "name": {"type": "text"},
                "category": {"type": "keyword"},
                "price": {"type": "double"},
                "stock": {"type": "integer"},
                "released": {"type": "date"}
            }}})),
        )
        .await
        .unwrap();
    let rows = [
        ("hnsw guide", "books", 10.0, 5, "2026-01-10"),
        ("vector primer", "books", 20.0, 5, "2026-01-20"),
        ("ingest manual", "books", 30.0, 2, "2026-02-05"),
        ("cluster mug", "merch", 8.0, 10, "2026-02-14"),
        ("shard shirt", "merch", 25.0, 2, "2026-03-01"),
        ("node poster", "merch", 12.0, 10, "2026-03-02"),
        ("search course", "courses", 100.0, 5, "2026-03-15"),
        ("rag course", "courses", 150.0, 2, "2026-03-20"),
    ];
    for (n, (name, category, price, stock, released)) in rows.iter().enumerate() {
        client
            .send(
                &index::Request::new(test.name())
                    .id(n.to_string())
                    .refresh(Refresh::True)
                    .body(json!({"name": name, "category": category, "price": price, "stock": stock, "released": released})),
            )
            .await
            .unwrap();
    }
    test
}

/// Runs aggregations over every product, results typed.
async fn aggregate(client: &TestClient, test: &TestIndex, aggs: Aggs) -> Aggregates {
    let request = search::Request::new()
        .index(test.name())
        .typed_keys(true)
        .body(search::RequestBody::new().size(0).aggregations(aggs));
    let response: search::Response<Value, Aggregates> = client.send_as(&request).await.unwrap();
    response.0.aggregations.expect("aggregation results")
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn buckets() {
    let client = client();
    let test = products(&client).await;
    let results = aggregate(
        &client,
        &test,
        Aggs::new()
            .add(
                "by_category",
                Agg::terms("category", 10).sub("avg_price", Agg::avg("price")),
            )
            .add("by_stock", Agg::terms("stock", 10))
            .add("by_price", Agg::histogram("price", 50.0))
            .add(
                "by_month",
                Agg::date_histogram("released", CalendarInterval::Month),
            )
            .add(
                "price_bands",
                Agg::range(
                    "price",
                    [
                        (None, Some(20.0)),
                        (Some(20.0), Some(100.0)),
                        (Some(100.0), None),
                    ],
                ),
            )
            .add(
                "kinds",
                Agg::filters([
                    (
                        "cheap",
                        Query::range(
                            "price",
                            oxsearch::query_dsl::NumberRangeQuery::new().lt(15.0),
                        ),
                    ),
                    ("books", Query::term("category", "books")),
                ]),
            ),
    )
    .await;

    // Keyword terms, with each bucket's own sub-aggregation.
    let categories = results.string_terms("by_category");
    assert_eq!(
        results.terms("by_category"),
        [
            ("books".into(), 3),
            ("merch".into(), 3),
            ("courses".into(), 2)
        ]
    );
    let books = categories[0];
    assert!(close(books.aggregations.value("avg_price").unwrap(), 20.0));
    // Integer terms come back as long terms.
    assert_eq!(
        results.terms("by_stock"),
        [("2".into(), 3), ("5".into(), 3), ("10".into(), 2)]
    );
    let prices: Vec<(Option<f64>, Option<i64>)> = results
        .histogram("by_price")
        .iter()
        .map(|b| (b.key, b.doc_count))
        .collect();
    assert_eq!(
        prices,
        [
            (Some(0.0), Some(6)),
            (Some(50.0), Some(0)),
            (Some(100.0), Some(1)),
            (Some(150.0), Some(1))
        ]
    );
    let months: Vec<Option<i64>> = results
        .date_histogram("by_month")
        .iter()
        .map(|b| b.doc_count)
        .collect();
    assert_eq!(months, [Some(2), Some(2), Some(4)]);
    let bands: Vec<Option<i64>> = results
        .ranges("price_bands")
        .iter()
        .map(|b| b.doc_count)
        .collect();
    assert_eq!(bands, [Some(3), Some(3), Some(2)]);
    let kinds: Vec<(String, Option<i64>)> = results
        .filters("kinds")
        .into_iter()
        .map(|(k, b)| (k, b.doc_count))
        .collect();
    // Keyed buckets come back in their names' order.
    assert_eq!(
        kinds,
        [("books".into(), Some(3)), ("cheap".into(), Some(3))]
    );
    test.delete(&client).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn metrics() {
    let client = client();
    let test = products(&client).await;
    let results = aggregate(
        &client,
        &test,
        Aggs::new()
            .add("avg", Agg::avg("price"))
            .add("sum", Agg::sum("price"))
            .add("min", Agg::min("price"))
            .add("max", Agg::max("price"))
            .add("count", Agg::value_count("price"))
            .add("categories", Agg::cardinality("category"))
            .add("stats", Agg::stats("stock"))
            .add("median", Agg::percentiles("price", vec![50.0]))
            .add(
                "priciest",
                Agg::of(oxsearch::aggs::TopHitsAggregation::new().size(1).sort(
                    serde_json::from_value::<oxsearch::Sort>(json!([{"price": "desc"}])).unwrap(),
                )),
            ),
    )
    .await;
    assert!(close(results.value("sum").unwrap(), 355.0));
    assert!(close(results.value("avg").unwrap(), 355.0 / 8.0));
    assert!(close(results.value("min").unwrap(), 8.0));
    assert!(close(results.value("max").unwrap(), 150.0));
    assert!(close(results.value("count").unwrap(), 8.0));
    assert!(close(results.value("categories").unwrap(), 3.0));
    let stats = results.stats("stats").unwrap();
    assert_eq!(stats.count, Some(8));
    assert!(close(stats.sum.unwrap(), 41.0));
    let median = results.percentiles("median");
    assert_eq!(median.len(), 1);
    assert!(
        median[0].1.is_some_and(|v| (10.0..=30.0).contains(&v)),
        "{median:?}"
    );
    let top = results.top_hits("priciest");
    assert_eq!(top.len(), 1);
    assert_eq!(top[0].source.as_ref().unwrap()["name"], "rag course");
    test.delete(&client).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn pipelines() {
    let client = client();
    let test = products(&client).await;
    let results = aggregate(
        &client,
        &test,
        Aggs::new()
            .add(
                "by_month",
                Agg::date_histogram("released", CalendarInterval::Month)
                    .sub("sales", Agg::sum("price"))
                    .sub("running", Agg::cumulative_sum("sales"))
                    .sub("change", Agg::derivative("sales")),
            )
            .add("monthly_avg", Agg::avg_bucket("by_month>sales"))
            .add("best_month", Agg::max_bucket("by_month>sales"))
            .add("total", Agg::sum_bucket("by_month>sales")),
    )
    .await;
    let months = results.date_histogram("by_month");
    let sales: Vec<f64> = months
        .iter()
        .map(|m| m.aggregations.value("sales").unwrap())
        .collect();
    assert_eq!(sales, [30.0, 38.0, 287.0]);
    let running: Vec<f64> = months
        .iter()
        .map(|m| m.aggregations.value("running").unwrap())
        .collect();
    assert_eq!(running, [30.0, 68.0, 355.0]);
    // The first month has nothing to compare with.
    assert_eq!(months[0].aggregations.value("change"), None);
    assert!(close(
        months[2].aggregations.value("change").unwrap(),
        249.0
    ));
    assert!(close(results.value("monthly_avg").unwrap(), 355.0 / 3.0));
    assert!(close(results.value("best_month").unwrap(), 287.0));
    assert!(close(results.value("total").unwrap(), 355.0));
    test.delete(&client).await;
}
