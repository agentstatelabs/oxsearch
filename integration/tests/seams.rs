//! The search seam, as an application uses it: aggregations written with
//! `json!` into the request, and their results read into the application's
//! own type through the response's type parameter.

use oxsearch_core::Refresh;
use oxsearch_core::query_dsl::{MatchQuery, MatchQueryQuery, QueryContainer};
use oxsearch_core::shapes::SingleKey;
use oxsearch_docs::index;
use oxsearch_integration::fixtures::{article_mapping, articles};
use oxsearch_integration::{Article, TestIndex, client};
use oxsearch_search::search;
use serde::Deserialize;
use serde_json::json;

/// The results of the one aggregation the test asks for.
#[derive(Debug, Deserialize)]
struct TagAggregations {
    by_tag: Terms,
}

#[derive(Debug, Deserialize)]
struct Terms {
    buckets: Vec<Bucket>,
}

#[derive(Debug, Deserialize)]
struct Bucket {
    key: String,
    doc_count: u64,
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn aggregations_through_the_seam() {
    let client = client();
    let test = TestIndex::create(&client, "seams", &article_mapping()).await;
    for (id, article) in articles() {
        client
            .send(
                &index::Request::new(test.name())
                    .id(id)
                    .refresh(Refresh::True)
                    .body(article),
            )
            .await
            .unwrap();
    }
    let body = search::RequestBody::new()
        .size(0)
        .aggregations(json!({"by_tag": {"terms": {"field": "tag"}}}));
    let request = search::Request::new().index(test.name()).body(body);

    // Typed: the application's own aggregation results.
    let response: search::Response<Article, TagAggregations> =
        client.send_as(&request).await.unwrap();
    let by_tag = &response
        .aggregations
        .as_ref()
        .expect("aggregation results")
        .by_tag;
    let counts: Vec<(&str, u64)> = by_tag
        .buckets
        .iter()
        .map(|b| (b.key.as_str(), b.doc_count))
        .collect();
    assert_eq!(counts, [("vector", 2), ("ingest", 1)]);

    // Raw, by default: JSON by aggregation name.
    let raw = client.send(&request).await.unwrap();
    let buckets = &raw.aggregations.as_ref().expect("aggregation results")["by_tag"]["buckets"];
    assert_eq!(buckets[0]["key"], "vector");

    // Hits and aggregations together, with a query.
    let query = QueryContainer::Match(SingleKey::new(
        "title",
        MatchQuery::new(MatchQueryQuery::String("vectors".into())),
    ));
    let request = search::Request::new().index(test.name()).body(
        search::RequestBody::new()
            .query(query)
            .aggregations(json!({"by_tag": {"terms": {"field": "tag"}}})),
    );
    let response: search::Response<Article, TagAggregations> =
        client.send_as(&request).await.unwrap();
    let hits = &response.hits.as_ref().expect("hits").hits;
    assert_eq!(hits.len(), 1);
    assert_eq!(
        response
            .aggregations
            .as_ref()
            .expect("aggregation results")
            .by_tag
            .buckets
            .len(),
        1
    );
    test.delete(&client).await;
}
