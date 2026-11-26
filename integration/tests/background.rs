//! Background tasks: a slow reindex, followed through the reindex
//! management API and the task API, and cancelled through each.

use oxsearch::docs::{cancel_reindex, get_reindex, list_reindex, reindex};
use oxsearch::tasks;
use oxsearch::{Query, Refresh};
use oxsearch_docs::index;
use oxsearch_integration::fixtures::{article_mapping, articles};
use oxsearch_integration::{TestClient, TestIndex, client};

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

/// Starts a reindex that copies one document every ten seconds, and returns
/// its task's id.
async fn slow_reindex(client: &TestClient, from: &TestIndex, to: &TestIndex) -> String {
    let started = client
        .send(
            &reindex::Request::new()
                .wait_for_completion(false)
                .requests_per_second(0.1)
                .body(reindex::RequestBody::new(
                    reindex::Destination::new(to.name()),
                    reindex::Source::new(from.name())
                        .size(1)
                        .query(Query::match_all()),
                )),
        )
        .await
        .unwrap();
    started.task.expect("a background task").to_string()
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn follow_and_cancel_tasks() {
    let client = client();
    let test = articles_index(&client, "tasks").await;
    let (first, second) = (TestIndex::named("copy-a"), TestIndex::named("copy-b"));

    // Through the reindex management API.
    let task = slow_reindex(&client, &test, &first).await;
    let status = client
        .send(&get_reindex::Request::new(task.as_str()))
        .await
        .unwrap();
    assert_eq!(status.completed, Some(false), "{status:?}");
    let running = client.send(&list_reindex::Request::new()).await.unwrap();
    assert!(!running.reindex.is_empty(), "{running:?}");
    client
        .send(&cancel_reindex::Request::new(task.as_str()).wait_for_completion(true))
        .await
        .unwrap();

    // Through the task API.
    let task = slow_reindex(&client, &test, &second).await;
    let status = client
        .send(&tasks::get::Request::new(task.as_str()))
        .await
        .unwrap();
    assert_eq!(status.completed, Some(false), "{status:?}");
    let listed = client
        .send(&tasks::list::Request::new().actions("*reindex"))
        .await
        .unwrap();
    assert!(
        listed.nodes.as_ref().is_some_and(|n| !n.is_empty()),
        "{listed:?}"
    );
    let cancelled = client
        .send(&tasks::cancel::Request::new().task_id(task.as_str()))
        .await
        .unwrap();
    assert!(cancelled.node_failures.is_none(), "{cancelled:?}");

    for index in [first, second] {
        let _ = client
            .send_raw(
                oxsearch::Method::Delete,
                &format!("/{}", index.name()),
                None,
            )
            .await;
    }
    test.delete(&client).await;
}
