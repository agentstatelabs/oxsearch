//! The document endpoints beyond index, get, bulk and delete: create,
//! existence checks, sources, updates, by-query updates and deletes,
//! reindexing, term vectors, and throttling tasks.

use std::time::Duration;

use oxsearch::docs::{
    create, delete_by_query, delete_by_query_rethrottle, exists, exists_source, get_source,
    mtermvectors, reindex, reindex_rethrottle, termvectors, update, update_by_query,
    update_by_query_rethrottle,
};
use oxsearch::{Error, Query, Refresh, Script};
use oxsearch_docs::index;
use oxsearch_integration::fixtures::{article_mapping, articles};
use oxsearch_integration::{Article, TestClient, TestIndex, client};
use serde_json::json;

/// An index holding the three articles.
async fn articles_index(client: &TestClient, prefix: &str) -> TestIndex {
    let test = TestIndex::create(client, prefix, &article_mapping()).await;
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
    test
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn create_check_read_and_update() {
    let client = client();
    let test = articles_index(&client, "writing").await;
    let note = articles().remove(0).1;

    let created = client
        .send(
            &create::Request::new("10", test.name())
                .refresh(Refresh::True)
                .body(note.clone()),
        )
        .await
        .unwrap();
    assert_eq!(created.id.as_deref(), Some("10"));
    // Creating it again is a conflict.
    let again = client
        .send(&create::Request::new("10", test.name()).body(note))
        .await;
    assert!(matches!(again, Err(Error::Conflict(_))), "{again:?}");

    client
        .send(&exists::Request::new("10", test.name()))
        .await
        .unwrap();
    let missing = client.send(&exists::Request::new("99", test.name())).await;
    assert!(matches!(missing, Err(Error::Elasticsearch(e)) if e.status == 404));
    client
        .send(&exists_source::Request::new("10", test.name()))
        .await
        .unwrap();

    let source: get_source::Response<Article> = client
        .send_as(&get_source::Request::new("10", test.name()))
        .await
        .unwrap();
    assert_eq!(source.0.title, "Tuning HNSW graphs");

    let updated = client
        .send(
            &update::Request::new("10", test.name())
                .refresh(Refresh::True)
                .body(update::RequestBody::new().doc(json!({"tag": "updated"}))),
        )
        .await
        .unwrap();
    assert_eq!(updated.result, Some(oxsearch::Result::Updated));
    let source: get_source::Response<Article> = client
        .send_as(&get_source::Request::new("10", test.name()))
        .await
        .unwrap();
    assert_eq!(source.0.tag, "updated");

    let vectors = client
        .send(
            &termvectors::Request::new(test.name())
                .id("1")
                .fields("title"),
        )
        .await
        .unwrap();
    assert_eq!(vectors.found, Some(true));
    assert!(vectors.term_vectors.unwrap().contains_key("title"));
    let many = client
        .send(
            &mtermvectors::Request::new()
                .index(test.name())
                .ids(vec!["1".to_owned(), "2".to_owned()])
                .fields("title"),
        )
        .await
        .unwrap();
    assert_eq!(many.docs.len(), 2);
    test.delete(&client).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn by_query_and_reindex() {
    let client = client();
    let test = articles_index(&client, "by-query").await;

    let updated = client
        .send(
            &update_by_query::Request::new(test.name())
                .refresh(true)
                .body(
                    update_by_query::RequestBody::new()
                        .query(Query::term("tag", "vector"))
                        .script(Script::new().source("ctx._source.tag = 'vectors'")),
                ),
        )
        .await
        .unwrap();
    assert_eq!(updated.updated, Some(2));

    let copy = TestIndex::named("reindexed");
    let copied = client
        .send(
            &reindex::Request::new()
                .refresh(true)
                .body(reindex::RequestBody::new(
                    reindex::Destination::new(copy.name()),
                    reindex::Source::new(test.name()),
                )),
        )
        .await
        .unwrap();
    assert_eq!(copied.created, Some(3));

    let deleted = client
        .send(
            &delete_by_query::Request::new(copy.name())
                .refresh(true)
                .body(delete_by_query::RequestBody::new().query(Query::term("tag", "vectors"))),
        )
        .await
        .unwrap();
    assert_eq!(deleted.deleted, Some(2));
    copy.delete(&client).await;
    test.delete(&client).await;
}

/// The id of a slow task: one batch of one document a second, so throttling
/// it has time to matter.
fn slow_task(response_task: Option<String>) -> String {
    response_task.expect("a task runs in the background")
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn throttling_background_tasks() {
    let client = client();
    let test = articles_index(&client, "throttle").await;

    // A reindex in the background, throttled hard, then let go.
    let copy = TestIndex::named("throttled");
    let started = client
        .send(
            &reindex::Request::new()
                .wait_for_completion(false)
                .requests_per_second(0.1)
                .body(reindex::RequestBody::new(
                    reindex::Destination::new(copy.name()),
                    reindex::Source::new(test.name()).size(1),
                )),
        )
        .await
        .unwrap();
    let task = slow_task(started.task.map(|t| t.to_string()));
    client
        .send(&reindex_rethrottle::Request::new(task.as_str(), -1.0))
        .await
        .unwrap();

    let started = client
        .send(
            &update_by_query::Request::new(test.name())
                .wait_for_completion(false)
                .requests_per_second(0.1)
                .scroll_size(1),
        )
        .await
        .unwrap();
    let task = slow_task(started.task.map(|t| t.to_string()));
    client
        .send(&update_by_query_rethrottle::Request::new(
            task.as_str(),
            -1.0,
        ))
        .await
        .unwrap();

    let started = client
        .send(
            &delete_by_query::Request::new(test.name())
                .wait_for_completion(false)
                .requests_per_second(0.1)
                .scroll_size(1)
                .body(delete_by_query::RequestBody::new().query(Query::match_all())),
        )
        .await
        .unwrap();
    let task = slow_task(started.task.map(|t| t.to_string()));
    client
        .send(&delete_by_query_rethrottle::Request::new(
            task.as_str(),
            -1.0,
        ))
        .await
        .unwrap();

    // Let the tasks finish before the indices go.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let _ = client
        .send_raw(oxsearch::Method::Delete, &format!("/{}", copy.name()), None)
        .await;
    test.delete(&client).await;
}
