//! Text structure against Elasticsearch, on any license: the structure of
//! NDJSON documents, of log messages and of an index's field, with the
//! mappings that fit each, and a grok pattern tested on a log line.

use oxsearch::Refresh;
use oxsearch::text_structure::{
    FormatType, find_field_structure, find_message_structure, find_structure, test_grok_pattern,
};
use oxsearch_docs::index;
use oxsearch_integration::{TestIndex, client};
use serde_json::json;

/// A node's log lines, one a second.
fn lines() -> Vec<String> {
    [
        "starting",
        "loaded modules",
        "joined the cluster",
        "started",
        "ready",
    ]
    .iter()
    .enumerate()
    .map(|(n, event)| format!("2026-11-01T00:00:0{n}Z INFO node-1 {event}"))
    .collect()
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn structure() {
    let client = client();

    // NDJSON documents, a line each.
    let documents: Vec<_> = lines()
        .iter()
        .enumerate()
        .map(|(n, line)| json!({"@timestamp": &line[..20], "level": "INFO", "bytes": n * 100}))
        .collect();
    let found = client
        .send(&find_structure::Request::new().body(documents))
        .await
        .unwrap();
    assert_eq!(found.format.as_deref(), Some("ndjson"), "{found:?}");
    assert_eq!(found.num_messages_analyzed, Some(5));
    assert_eq!(found.timestamp_field.as_deref(), Some("@timestamp"));
    let bytes = &found.field_stats["bytes"];
    assert_eq!(bytes.max_value, Some(400.0), "{bytes:?}");
    assert_eq!(bytes.mean_value, Some(200.0), "{bytes:?}");
    assert!(found.mappings.is_some(), "{found:?}");

    // Log messages.
    let messages = client
        .send(
            &find_message_structure::Request::new()
                .body(find_message_structure::RequestBody::new(lines())),
        )
        .await
        .unwrap();
    assert_eq!(
        messages.format,
        Some(FormatType::SemiStructuredText),
        "{messages:?}"
    );
    assert_eq!(messages.num_messages_analyzed, Some(5));
    assert!(messages.grok_pattern.is_some(), "{messages:?}");

    // The same lines as an index's field.
    let test = TestIndex::named("text-structure");
    for line in lines() {
        client
            .send(
                &index::Request::new(test.name())
                    .refresh(Refresh::True)
                    .body(json!({"message": line})),
            )
            .await
            .unwrap();
    }
    let field = client
        .send(&find_field_structure::Request::new("message", test.name()))
        .await
        .unwrap();
    assert_eq!(
        field.format,
        Some(FormatType::SemiStructuredText),
        "{field:?}"
    );
    assert_eq!(field.num_messages_analyzed, Some(5));

    // A grok pattern on the first line.
    let tested = client
        .send(
            &test_grok_pattern::Request::new().body(test_grok_pattern::RequestBody::new(
                "%{TIMESTAMP_ISO8601:time} %{LOGLEVEL:level} %{NOTSPACE:node} %{GREEDYDATA:event}",
                vec![lines()[0].clone()],
            )),
        )
        .await
        .unwrap();
    let matched = &tested.matches[0];
    assert_eq!(matched.matched, Some(true), "{tested:?}");
    let event = &matched.fields.as_ref().expect("the fields")["event"][0];
    assert_eq!(event.r#match.as_deref(), Some("starting"));
    assert_eq!(event.offset, Some(33));

    test.delete(&client).await;
}
