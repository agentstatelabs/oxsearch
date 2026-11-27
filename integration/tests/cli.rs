//! The `oxsearch` command's export, import and transfer against
//! Elasticsearch, through the library it runs: documents round-tripped
//! through a gzip file with their ids and routing, samples and limits,
//! plain documents with ids from a field, a transfer that Elasticsearch
//! refuses part of, and a file that stops at a malformed line.

use std::path::PathBuf;

use oxsearch::{Method, Refresh};
use oxsearch_cli::{
    Error, ExportOptions, ImportOptions, Input, Output, TransferOptions, export, import, transfer,
};
use oxsearch_docs::index;
use oxsearch_integration::{TestClient, TestIndex, client};
use serde_json::{Value, json};

/// A path for the test's file, which doesn't exist yet.
fn file(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{name}-{}", std::process::id()))
}

async fn count(client: &TestClient, index: &str) -> u64 {
    client
        .send_raw(Method::Post, &format!("/{index}/_refresh"), None)
        .await
        .unwrap();
    let counted = client
        .send_raw(Method::Get, &format!("/{index}/_count"), None)
        .await
        .unwrap();
    counted["count"].as_u64().unwrap()
}

/// An index of `n` notes on two shards, each routed by its owner.
async fn notes(client: &TestClient, prefix: &str, n: u64) -> TestIndex {
    let test = TestIndex::named(prefix);
    client
        .send_raw(
            Method::Put,
            &format!("/{}", test.name()),
            Some(&json!({"settings": {"number_of_shards": 2}})),
        )
        .await
        .unwrap();
    for i in 0..n {
        client
            .send(
                &index::Request::new(test.name())
                    .id(format!("note-{i}"))
                    .routing(format!("owner-{}", i % 3))
                    .body(json!({"title": format!("note {i}"), "views": i, "price": 1.10})),
            )
            .await
            .unwrap();
    }
    client
        .send_raw(Method::Post, &format!("/{}/_refresh", test.name()), None)
        .await
        .unwrap();
    test
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn round_trip_through_a_gzip_file() {
    let client = client();
    let source = notes(&client, "cli-source", 25).await;
    let path = file("oxsearch-cli-round-trip.ndjson.gz");

    let mut options = ExportOptions::new(source.name());
    options.slices = 2;
    options.page_size = 4;
    let exported = export(&client, &options, &Output::file(&path))
        .await
        .unwrap();
    assert_eq!((exported.read, exported.written), (25, 25));
    let refused = export(&client, &options, &Output::file(&path)).await;
    assert!(matches!(refused, Err(e) if matches!(e.error, Error::Io { .. })));

    // Into another index, then back into its own once it's gone.
    let copy = TestIndex::named("cli-copy");
    let mut into = ImportOptions::new();
    into.index = Some(copy.name().to_owned());
    into.batch_size = 10;
    let imported = import(&client, &into, &Input::file(&path)).await.unwrap();
    assert!(imported.is_ok(), "{imported}");
    assert_eq!(imported.written, 25);
    assert_eq!(count(&client, copy.name()).await, 25);
    let note = client
        .send_raw(
            Method::Get,
            &format!("/{}/_doc/note-7?routing=owner-1", copy.name()),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        note["_source"],
        json!({"title": "note 7", "views": 7, "price": 1.10})
    );
    client
        .send_raw(Method::Delete, &format!("/{}", source.name()), None)
        .await
        .unwrap();
    let restored = import(&client, &ImportOptions::new(), &Input::file(&path))
        .await
        .unwrap();
    assert_eq!(restored.written, 25);
    assert_eq!(count(&client, source.name()).await, 25);

    std::fs::remove_file(&path).unwrap();
    copy.delete(&client).await;
    source.delete(&client).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn samples_limits_and_plain_documents() {
    let client = client();
    let source = notes(&client, "cli-sample", 20).await;

    // One in three of those that match, at most four, without ids.
    let path = file("oxsearch-cli-sample.ndjson");
    let mut options = ExportOptions::new(source.name());
    options.query = Some(json!({"range": {"views": {"gte": 5}}}));
    options.every = 3;
    options.limit = Some(4);
    options.plain = true;
    let exported = export(&client, &options, &Output::file(&path))
        .await
        .unwrap();
    // Kept 0, 3, 6 and 9 of the 15 that match, then stopped.
    assert_eq!(
        (exported.read, exported.skipped, exported.written),
        (10, 6, 4),
        "{exported}"
    );
    let lines: Vec<Value> = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 4);
    assert!(
        lines
            .iter()
            .all(|l| l["views"].as_u64().unwrap() >= 5 && l.get("_id").is_none())
    );

    // Plain documents, with their ids from a field.
    let skus = file("oxsearch-cli-skus.ndjson");
    std::fs::write(
        &skus,
        "{\"sku\": \"a-1\", \"n\": 1}\n\n{\"sku\": 2, \"n\": 2}\r\n{\"sku\": \"a-3\", \"n\": 3}\n",
    )
    .unwrap();
    let target = TestIndex::named("cli-skus");
    let mut into = ImportOptions::new();
    into.index = Some(target.name().to_owned());
    into.plain = true;
    into.id_field = Some("sku".to_owned());
    into.every = 2;
    let imported = import(&client, &into, &Input::file(&skus)).await.unwrap();
    assert_eq!(
        (imported.read, imported.skipped, imported.written),
        (3, 1, 2)
    );
    count(&client, target.name()).await;
    let read = client
        .send_raw(Method::Get, &format!("/{}/_doc/a-3", target.name()), None)
        .await
        .unwrap();
    assert_eq!(read["_source"]["n"], 3);

    for path in [path, skus] {
        std::fs::remove_file(path).unwrap();
    }
    target.delete(&client).await;
    source.delete(&client).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn a_transfer_says_what_was_refused() {
    let client = client();
    let source = notes(&client, "cli-from", 12).await;
    // Two more notes have a field the destination's strict mapping doesn't
    // have.
    for id in ["odd-1", "odd-2"] {
        client
            .send(
                &index::Request::new(source.name())
                    .id(id)
                    .routing("owner-x")
                    .refresh(Refresh::True)
                    .body(json!({"title": "odd", "views": 0, "price": 0.0, "extra": true})),
            )
            .await
            .unwrap();
    }
    let target = TestIndex::named("cli-to");
    client
        .send_raw(
            Method::Put,
            &format!("/{}", target.name()),
            Some(&json!({"mappings": {"dynamic": "strict", "properties": {
                "title": {"type": "text"}, "views": {"type": "long"}, "price": {"type": "double"},
            }}})),
        )
        .await
        .unwrap();
    let mut options = TransferOptions::new(source.name());
    options.to_index = Some(target.name().to_owned());
    options.slices = 2;
    options.batch_size = 5;
    let report = transfer(&client, &client, &options).await.unwrap();
    assert!(!report.is_ok());
    assert_eq!(
        (report.read, report.written, report.failed),
        (14, 12, 2),
        "{report}"
    );
    let mut refused: Vec<&str> = report
        .failures
        .iter()
        .filter_map(|f| f.id.as_deref())
        .collect();
    refused.sort();
    assert_eq!(refused, ["odd-1", "odd-2"]);
    assert!(
        report.failures[0]
            .reason
            .contains("strict_dynamic_mapping_exception")
    );
    assert_eq!(count(&client, target.name()).await, 12);
    target.delete(&client).await;
    source.delete(&client).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn an_import_stops_at_a_malformed_line() {
    let client = client();
    let target = TestIndex::named("cli-malformed");
    let path = file("oxsearch-cli-malformed.ndjson");
    let record = |id: u32| json!({"_id": id.to_string(), "_source": {"n": id}}).to_string();
    std::fs::write(
        &path,
        format!(
            "{}\n{}\n{{\"_id\": \"3\", \"_source\": \n",
            record(1),
            record(2)
        ),
    )
    .unwrap();
    let mut into = ImportOptions::new();
    into.index = Some(target.name().to_owned());
    let stopped = import(&client, &into, &Input::file(&path))
        .await
        .unwrap_err();
    assert!(
        matches!(stopped.error, Error::Line { line: 3, .. }),
        "{stopped}"
    );
    // What was read before it is written.
    assert_eq!(stopped.report.written, 2);
    assert_eq!(count(&client, target.name()).await, 2);
    std::fs::remove_file(&path).unwrap();
    target.delete(&client).await;
}
