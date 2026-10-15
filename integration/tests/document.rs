//! The typed document layer against Elasticsearch: the mapping from the
//! struct, drift, writes and reads with versions, and searches through
//! field handles.

use oxsearch::document::{Document, Indexes};
use oxsearch::fusion::Fusion;
use oxsearch::{Error, Method, Refresh, RetrieverContainer};
use oxsearch_integration::{License, TestIndex, client};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Document, Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Note {
    #[id]
    id: String,
    title: String,
    #[keyword]
    tags: Vec<String>,
    #[dense_vector(dims = 3, similarity = "cosine")]
    embedding: Vec<f32>,
    #[date]
    published: String,
    views: u32,
}

fn note(id: &str, title: &str, tag: &str, embedding: [f32; 3], views: u32) -> Note {
    Note {
        id: id.to_owned(),
        title: title.to_owned(),
        tags: vec![tag.to_owned()],
        embedding: embedding.to_vec(),
        published: "2026-10-01".to_owned(),
        views,
    }
}

fn notes() -> Vec<Note> {
    vec![
        note("1", "Tuning HNSW graphs", "vector", [1.0, 0.0, 0.0], 40),
        note("2", "Bulk loading at speed", "ingest", [0.0, 1.0, 0.0], 10),
        note("3", "Quantizing vectors", "vector", [0.7, 0.0, 0.7], 25),
    ]
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn write_read_and_delete() {
    let client = client();
    let test = TestIndex::named("notes");
    let notes_index = client
        .index_named::<Note>(test.name())
        .refresh(Refresh::WaitFor);
    let report = notes_index.ensure_mapping().await.unwrap();
    assert!(report.created && report.is_current());
    let again = notes_index.ensure_mapping().await.unwrap();
    assert!(!again.created && again.added.is_empty() && again.is_current());

    for note in notes() {
        let written = notes_index.put(&note).await.unwrap();
        assert!(written.created);
        assert_eq!(written.id, note.id);
    }
    // Documents read back whole, vectors included.
    let read = notes_index.get("1").await.unwrap().expect("note 1");
    assert_eq!(read.document, notes()[0]);
    assert_eq!(read.title, "Tuning HNSW graphs");
    assert!(notes_index.get("9").await.unwrap().is_none());
    let many = notes_index.get_many(["3", "9", "2"]).await.unwrap();
    let ids: Vec<Option<&str>> = many
        .iter()
        .map(|n| n.as_ref().map(|n| n.id.as_str()))
        .collect();
    assert_eq!(ids, [Some("3"), None, Some("2")]);

    assert!(notes_index.delete("2").await.unwrap());
    assert!(!notes_index.delete("2").await.unwrap());
    test.delete(&client).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn stale_writes_conflict() {
    let client = client();
    let test = TestIndex::named("versions");
    let notes_index = client.index_named::<Note>(test.name());
    let _ = notes_index.ensure_mapping().await.unwrap();
    notes_index.put(&notes()[0]).await.unwrap();

    let mut first = notes_index.get("1").await.unwrap().unwrap();
    let mut second = first.clone();
    first.views += 1;
    let written = notes_index.put_versioned(&first).await.unwrap();
    assert!(!written.created);
    assert!(written.version.seq_no > first.version.seq_no);
    second.views += 2;
    match notes_index.put_versioned(&second).await {
        Err(Error::Conflict(e)) => {
            assert_eq!(e.error_type(), Some("version_conflict_engine_exception"))
        }
        other => panic!("expected a conflict, got {other:?}"),
    }
    assert_eq!(notes_index.get("1").await.unwrap().unwrap().views, 41);
    test.delete(&client).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn drift_is_reported_never_applied() {
    let client = client();
    let test = TestIndex::named("drift");
    // An index whose tags are text, without the embedding and views.
    client
        .send_raw(
            Method::Put,
            &format!("/{}", test.name()),
            Some(&json!({"mappings": {"properties": {
                "id": {"type": "text"},
                "title": {"type": "text"},
                "tags": {"type": "text"},
                "published": {"type": "date"}
            }}})),
        )
        .await
        .unwrap();
    let notes_index = client.index_named::<Note>(test.name());
    let report = notes_index.ensure_mapping().await.unwrap();
    assert!(!report.created);
    assert_eq!(report.added, ["embedding", "views"]);
    let drifted: Vec<&str> = report.drift.iter().map(|d| d.field.as_str()).collect();
    assert_eq!(drifted, ["tags"]);
    let mapping = test.mapping(&client).await;
    assert_eq!(mapping["properties"]["tags"]["type"], "text");
    assert_eq!(mapping["properties"]["embedding"]["dims"], 3);
    test.delete(&client).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn search_through_field_handles() {
    let client = client();
    let test = TestIndex::named("handles");
    let notes_index = client
        .index_named::<Note>(test.name())
        .refresh(Refresh::WaitFor);
    let _ = notes_index.ensure_mapping().await.unwrap();
    for note in notes() {
        notes_index.put(&note).await.unwrap();
    }
    let ids = |hits: Vec<oxsearch::search::search::Hit<Note>>| -> Vec<String> {
        hits.into_iter().filter_map(|hit| hit.id).collect()
    };

    let found = notes_index
        .search(Note::TITLE.r#match("vectors"))
        .await
        .unwrap();
    assert_eq!(ids(found), ["3"]);
    let found = notes_index
        .search(Note::EMBEDDING.knn(vec![1.0, 0.0, 0.1], 1, 10))
        .await
        .unwrap();
    assert_eq!(ids(found), ["1"]);
    let found = notes_index.search(Note::VIEWS.range(20.0..)).await.unwrap();
    let mut found = ids(found);
    found.sort();
    assert_eq!(found, ["1", "3"]);
    let found = notes_index.search(Note::TAGS.term("ingest")).await.unwrap();
    assert_eq!(ids(found), ["2"]);

    let fused = notes_index
        .fuse(
            &Fusion::rrf([
                RetrieverContainer::from(Note::TITLE.r#match("vectors")),
                Note::EMBEDDING.knn(vec![0.7, 0.0, 0.7], 2, 10),
            ])
            .size(2),
        )
        .await
        .unwrap();
    assert_eq!(
        fused.fallback.is_some(),
        License::from_env() == License::Basic
    );
    assert_eq!(fused.hits[0].id.as_deref(), Some("3"));
    assert_eq!(
        fused.hits[0].source.as_ref().unwrap().title,
        "Quantizing vectors"
    );
    test.delete(&client).await;
}
