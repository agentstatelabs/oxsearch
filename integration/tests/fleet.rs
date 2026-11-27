//! Fleet's APIs against Elasticsearch, on any license: an index's global
//! checkpoints, searches that wait until one is visible, a multi-search, and
//! Fleet's secrets.

use oxsearch::fleet::{
    delete_secret, get_secret, global_checkpoints, msearch, post_secret, search,
};
use oxsearch::search::msearch::{MultisearchHeader, RequestItem, ResponseItem};
use oxsearch::search::search::SearchRequestBody;
use oxsearch::{Error, Query};
use oxsearch_docs::index;
use oxsearch_integration::{TestIndex, client};
use serde_json::{Value, json};

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn checkpoints() {
    let client = client();
    let test = TestIndex::named("fleet");
    for n in 0..3 {
        client
            .send(&index::Request::new(test.name()).body(json!({"title": format!("note {n}")})))
            .await
            .unwrap();
    }

    // Three documents in one shard: its checkpoint is the third's, 2.
    let read = client
        .send(
            &global_checkpoints::Request::new(test.name())
                .wait_for_advance(true)
                .checkpoints(vec![1])
                .timeout("30s"),
        )
        .await
        .unwrap();
    assert_eq!(read.global_checkpoints, vec![2], "{read:?}");
    assert_eq!(read.timed_out, Some(false));

    // Nothing was refreshed: the searches wait until the checkpoint is
    // visible, and find every document.
    let found: search::Response<Value> = client
        .send_as(
            &search::Request::new(test.name())
                .wait_for_checkpoints(vec![2])
                .body(search::RequestBody::new().query(Query::r#match("title", "note"))),
        )
        .await
        .unwrap();
    let hits = found.hits.expect("the hits");
    assert_eq!(hits.hits.len(), 3, "{hits:?}");

    // The specification gives the multi-search the same parameter, which
    // Elasticsearch refuses there.
    let lines = || {
        vec![
            RequestItem::Header(MultisearchHeader::new()),
            RequestItem::Body(SearchRequestBody::new().query(Query::r#match("title", "note"))),
        ]
    };
    let refused = client
        .send(
            &msearch::Request::new()
                .index(test.name())
                .wait_for_checkpoints(vec![2])
                .body(lines()),
        )
        .await;
    match refused {
        Err(Error::Elasticsearch(e)) => assert!(
            e.reason()
                .is_some_and(|r| r.contains("unrecognized parameter")),
            "{e}"
        ),
        other => panic!("a multi-search's checkpoints in the URL: {other:?}"),
    }
    // Read as JSON first: the type takes the specification's `docs` too.
    let many = msearch::Request::new().index(test.name()).body(lines());
    let raw: Value = client.send_as(&many).await.unwrap();
    assert!(raw["responses"].is_array(), "{raw}");
    let many = client.send(&many).await.unwrap();
    assert_eq!(many.responses.len(), 1, "{many:?}");
    match &many.responses[0] {
        ResponseItem::Result(item) => assert_eq!(
            item.hits.as_ref().map(|h| h.hits.len()),
            Some(3),
            "{item:?}"
        ),
        other => panic!("{other:?}"),
    }
    test.delete(&client).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn secrets() {
    let client = client();
    let id = client
        .send(&post_secret::Request::new().body(post_secret::RequestBody::new("a value")))
        .await
        .unwrap()
        .id
        .expect("the secret's ID");
    let read = client
        .send(&get_secret::Request::new(id.as_str()))
        .await
        .unwrap();
    assert_eq!(read.value.as_deref(), Some("a value"));
    let deleted = client
        .send(&delete_secret::Request::new(id.as_str()))
        .await
        .unwrap();
    assert_eq!(deleted.deleted, Some(true));
}
