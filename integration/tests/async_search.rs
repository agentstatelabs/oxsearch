//! Async search: a search that runs in the background, read by its id
//! while it's kept, with typed aggregation results.

use oxsearch::aggs::{Agg, Aggs, Results};
use oxsearch::async_search::{delete, get, status, submit};
use oxsearch::{Duration, Error, Query, Refresh};
use oxsearch_docs::index;
use oxsearch_integration::{TestIndex, client};
use serde_json::json;

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn background_search() {
    let client = client();
    let test = TestIndex::named("async-search");
    for (n, category) in ["books", "books", "merch"].into_iter().enumerate() {
        client
            .send(
                &index::Request::new(test.name())
                    .id(n.to_string())
                    .refresh(Refresh::True)
                    .body(json!({"category": category})),
            )
            .await
            .unwrap();
    }

    // Kept after it completes, so it can be read again by its id.
    let submitted = client
        .send(
            &submit::Request::new()
                .index(test.name())
                .wait_for_completion_timeout(Duration::String("10s".into()))
                .keep_on_completion(true)
                .typed_keys(true)
                .body(
                    submit::RequestBody::new()
                        .query(Query::match_all())
                        .aggregations(
                            Aggs::new()
                                .add("by_category", Agg::terms("category.keyword", 10))
                                .0,
                        ),
                ),
        )
        .await
        .unwrap();
    assert_eq!(submitted.is_running, Some(false));
    let id = submitted.id.clone().expect("a kept search's id");
    let results = submitted
        .response
        .as_ref()
        .and_then(|r| r.aggregations.as_ref())
        .expect("aggregations");
    assert_eq!(
        results.terms("by_category"),
        [("books".into(), 2), ("merch".into(), 1)]
    );

    let again = client.send(&get::Request::new(id.as_str())).await.unwrap();
    let total = again
        .response
        .as_ref()
        .and_then(|r| r.hits.as_ref())
        .and_then(|h| h.total.as_ref());
    assert!(total.is_some(), "{again:?}");
    let progress = client
        .send(&status::Request::new(id.as_str()))
        .await
        .unwrap();
    assert_eq!(progress.is_running, Some(false));

    client
        .send(&delete::Request::new(id.as_str()))
        .await
        .unwrap();
    let gone = client.send(&get::Request::new(id.as_str())).await;
    assert!(
        matches!(&gone, Err(Error::Elasticsearch(e)) if e.status == 404),
        "{gone:?}"
    );
    test.delete(&client).await;
}
