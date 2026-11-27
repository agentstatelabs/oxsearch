//! Graph against Elasticsearch: the tags that go with a search's, found in
//! a few notes, one hop out. Graph needs a license above Basic, so on Basic
//! the test checks that exploring is refused, and stops there.

use oxsearch::graph::{ExploreControls, Hop, VertexDefinition, explore};
use oxsearch::{Error, Method, Query, Refresh};
use oxsearch_docs::index;
use oxsearch_integration::{License, TestIndex, client};
use serde_json::json;

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn explored() {
    let client = client();
    let test = TestIndex::named("graph");
    client
        .send_raw(
            Method::Put,
            &format!("/{}", test.name()),
            Some(&json!({"mappings": {"properties": {
                "title": {"type": "text"},
                "tag": {"type": "keyword"},
            }}})),
        )
        .await
        .unwrap();
    for tags in [["rust", "search"], ["rust", "search"], ["rust", "async"]] {
        client
            .send(
                &index::Request::new(test.name())
                    .refresh(Refresh::True)
                    .body(json!({"title": "a note", "tag": tags})),
            )
            .await
            .unwrap();
    }

    // Few documents, so every term counts, not only the significant ones.
    let vertex = || {
        VertexDefinition::new("tag")
            .min_doc_count(1)
            .shard_min_doc_count(1)
    };
    let explored = client
        .send(
            &explore::Request::new(test.name()).body(
                explore::RequestBody::new()
                    .query(Query::r#match("title", "note"))
                    .controls(ExploreControls::new(false).sample_size(100))
                    .vertices(vec![vertex()])
                    .connections(Hop::new(vec![vertex()])),
            ),
        )
        .await;
    if License::from_env() == License::Basic {
        assert!(matches!(explored, Err(Error::License(_))), "{explored:?}");
    } else {
        let explored = explored.unwrap();
        assert!(explored.failures.is_empty(), "{explored:?}");
        let terms: Vec<_> = explored
            .vertices
            .iter()
            .filter_map(|v| v.term.as_deref())
            .collect();
        assert!(
            terms.contains(&"rust") && terms.contains(&"search"),
            "{terms:?}"
        );
        assert!(!explored.connections.is_empty(), "{explored:?}");
    }
    test.delete(&client).await;
}
