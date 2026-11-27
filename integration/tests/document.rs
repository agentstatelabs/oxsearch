//! The typed document layer against Elasticsearch: the mapping from the
//! struct, drift, writes and reads with versions, partial and scripted
//! updates, bulk writes, routing, and searches through field handles.

use oxsearch::document::{Change, Document, Indexes, Patch, Update};
use oxsearch::fusion::Fusion;
use oxsearch::{Error, Method, Query, Refresh, RetrieverContainer, Script};
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

    notes_index.delete_index().await.unwrap();
    let gone = notes_index.get("1").await;
    assert!(
        matches!(&gone, Err(Error::Elasticsearch(e)) if e.error_type() == Some("index_not_found_exception")),
        "{gone:?}"
    );
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

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn updates_change_part_of_a_document() {
    let client = client();
    let test = TestIndex::named("updates");
    let notes_index = client.index_named::<Note>(test.name());
    let _ = notes_index.ensure_mapping().await.unwrap();
    notes_index.put(&notes()[0]).await.unwrap();

    // Some fields, and the same again, which changes nothing.
    let patch = || {
        Patch::new()
            .set(Note::VIEWS, 41)
            .set(Note::TITLE, "Tuning HNSW")
    };
    let patched = notes_index.update("1", patch()).await.unwrap().unwrap();
    assert_eq!(patched.change, Change::Updated);
    let again = notes_index.update("1", patch()).await.unwrap().unwrap();
    assert_eq!(again.change, Change::Unchanged);
    let read = notes_index.get("1").await.unwrap().unwrap();
    assert_eq!(read.version, patched.version);
    assert_eq!((read.views, read.title.as_str()), (41, "Tuning HNSW"));
    // The rest of the document is as it was, its vector included.
    assert_eq!(read.tags, notes()[0].tags);
    assert_eq!(read.embedding, notes()[0].embedding);

    // A script, which writes the document when there's none.
    let visit = || Script::inline("ctx._source.views += params.by").param("by", 2);
    notes_index.update("1", visit()).await.unwrap().unwrap();
    assert_eq!(notes_index.get("1").await.unwrap().unwrap().views, 43);
    assert_eq!(notes_index.update("2", visit()).await.unwrap(), None);
    let first = Update::from(visit())
        .upsert(notes()[1].clone())
        .retry_on_conflict(3);
    let created = notes_index.update("2", first).await.unwrap().unwrap();
    assert_eq!(created.change, Change::Created);
    assert_eq!(notes_index.get("2").await.unwrap().unwrap().views, 10);

    // Only if the document is as it was read.
    let stale = notes_index.update("1", Update::from(patch()).if_version(read.version));
    assert!(matches!(stale.await, Err(Error::Conflict(_))));
    let read = notes_index.get("1").await.unwrap().unwrap();
    let fresh = notes_index.update("1", Update::from(patch()).if_version(read.version));
    assert_eq!(fresh.await.unwrap().unwrap().change, Change::Updated);

    // A script can delete the document.
    let removed = notes_index
        .update("2", Script::inline("ctx.op = 'delete'"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(removed.change, Change::Deleted);
    assert!(notes_index.get("2").await.unwrap().is_none());
    test.delete(&client).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn bulk_writes_succeed_or_fail_one_by_one() {
    let client = client();
    let test = TestIndex::named("bulk-notes");
    let notes_index = client
        .index_named::<Note>(test.name())
        .refresh(Refresh::WaitFor);
    let _ = notes_index.ensure_mapping().await.unwrap();
    let loaded = notes_index.bulk().put_all(&notes()).send().await.unwrap();
    assert!(loaded.is_ok(), "{loaded:?}");

    let mut edited = notes_index.get("1").await.unwrap().unwrap();
    edited.views = 100;
    let stale = notes_index.get("2").await.unwrap().unwrap();
    let gone = notes_index.get("3").await.unwrap().unwrap();
    notes_index
        .update("2", Patch::new().set(Note::VIEWS, 11))
        .await
        .unwrap();
    let report = notes_index
        .bulk()
        .put_versioned(&edited)
        .put_versioned(&stale)
        .create(&notes()[0])
        .update("2", Patch::new().set(Note::VIEWS, 12))
        .update("9", Patch::new().set(Note::VIEWS, 1))
        .delete_versioned(&gone)
        .delete("9")
        .send()
        .await
        .unwrap();
    let failed: Vec<usize> = report.failures().map(|(n, _)| n).collect();
    assert_eq!(failed, [1, 2], "{report:?}");
    for n in failed {
        assert!(matches!(report.items[n], Err(Error::Conflict(_))));
    }
    let change = |n: usize| report.items[n].as_ref().unwrap().change.clone();
    assert_eq!(change(0), Change::Updated);
    assert_eq!(change(3), Change::Updated);
    assert_eq!(change(4), Change::NotFound);
    assert_eq!(change(5), Change::Deleted);
    assert_eq!(change(6), Change::NotFound);

    let read = notes_index.get("1").await.unwrap().unwrap();
    assert_eq!(read.views, 100);
    assert_eq!(read.version, report.items[0].as_ref().unwrap().version);
    assert_eq!(notes_index.get("2").await.unwrap().unwrap().views, 12);
    assert!(notes_index.get("3").await.unwrap().is_none());
    // The version read before the edit is stale now.
    assert!(matches!(
        notes_index.delete_versioned(&edited).await,
        Err(Error::Conflict(_))
    ));
    notes_index.delete_versioned(&read).await.unwrap();
    test.delete(&client).await;
}

#[derive(Document, Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Order {
    #[id]
    id: String,
    #[routing]
    #[keyword]
    customer: String,
    total: u32,
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn routing_finds_documents_where_they_live() {
    let client = client();
    let test = TestIndex::named("orders");
    // Several shards, so that most documents aren't on the one their id
    // names.
    client
        .send_raw(
            Method::Put,
            &format!("/{}", test.name()),
            Some(&json!({"settings": {"number_of_shards": 5}})),
        )
        .await
        .unwrap();
    let orders = client
        .index_named::<Order>(test.name())
        .refresh(Refresh::WaitFor);
    let _ = orders.ensure_mapping().await.unwrap();
    let ids: Vec<String> = (0..10).map(|n| format!("order-{n}")).collect();
    let all: Vec<Order> = ids
        .iter()
        .map(|id| Order {
            id: id.clone(),
            customer: "ada".to_owned(),
            total: 1,
        })
        .collect();
    assert!(orders.bulk().put_all(&all).send().await.unwrap().is_ok());

    let unrouted = orders.get_many(ids.clone()).await.unwrap();
    let missed = unrouted.iter().position(Option::is_none).expect("a miss");
    let id = ids[missed].as_str();
    assert!(orders.get(id).await.unwrap().is_none());
    assert_eq!(
        orders
            .update(id, Patch::new().set(Order::TOTAL, 2))
            .await
            .unwrap(),
        None
    );
    assert!(!orders.delete(id).await.unwrap());

    let adas = orders.clone().routed("ada");
    assert!(
        adas.get_many(ids.clone())
            .await
            .unwrap()
            .iter()
            .all(Option::is_some)
    );
    let read = adas.get(id).await.unwrap().expect("the order");
    adas.update(id, Patch::new().set(Order::TOTAL, 2))
        .await
        .unwrap()
        .expect("the order");
    // A document that was read is written back where it was.
    let mut read_again = adas.get(id).await.unwrap().unwrap();
    assert_eq!(read_again.total, 2);
    assert!(matches!(
        orders.put_versioned(&read).await,
        Err(Error::Conflict(_))
    ));
    read_again.total = 3;
    assert!(!orders.put_versioned(&read_again).await.unwrap().created);
    assert_eq!(adas.get(id).await.unwrap().unwrap().total, 3);

    // Searches and fusion look only on that shard, which has every order.
    let found = adas.search(Query::match_all()).await.unwrap();
    assert_eq!(found.len(), ids.len());
    let fused = adas
        .fuse(&Fusion::rrf([
            RetrieverContainer::from(Order::CUSTOMER.term("ada")),
            RetrieverContainer::from(Query::match_all()),
        ]))
        .await
        .unwrap();
    assert_eq!(fused.hits.len(), ids.len());

    assert!(adas.delete(id).await.unwrap());
    let last = adas
        .get(&ids[(missed + 1) % ids.len()])
        .await
        .unwrap()
        .unwrap();
    orders.delete_versioned(&last).await.unwrap();
    assert_eq!(
        adas.search(Query::match_all()).await.unwrap().len(),
        ids.len() - 2
    );
    test.delete(&client).await;
}
