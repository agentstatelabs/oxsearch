//! EQL against Elasticsearch: events matching a condition, a sequence of
//! events in order, and a search kept to be read again by its id.

use oxsearch::eql::{delete, get, get_status, search};
use oxsearch::{Duration, Error, Refresh};
use oxsearch_docs::index;
use oxsearch_integration::{TestIndex, client};
use serde_json::json;

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn events_and_sequences() {
    let client = client();
    let test = TestIndex::named("eql");
    let events = [
        ("2026-10-19T10:00:00Z", "process", "explorer.exe"),
        ("2026-10-19T10:00:05Z", "process", "cmd.exe"),
        ("2026-10-19T10:00:09Z", "network", "cmd.exe"),
    ];
    for (n, (at, category, process)) in events.into_iter().enumerate() {
        client
            .send(
                &index::Request::new(test.name())
                    .id(n.to_string())
                    .refresh(Refresh::True)
                    .body(json!({
                        "@timestamp": at,
                        "event": {"category": category},
                        "process": {"name": process}
                    })),
            )
            .await
            .unwrap();
    }

    let found = client
        .send(
            &search::Request::new(test.name()).body(search::RequestBody::new(
                r#"process where process.name == "cmd.exe""#,
            )),
        )
        .await
        .unwrap();
    let hits = found.hits.as_ref().unwrap();
    let events = hits.events.as_ref().expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].id.as_deref(), Some("1"));

    // A process, then network traffic from it.
    let sequence = client
        .send(
            &search::Request::new(test.name()).body(search::RequestBody::new(
                "sequence by process.name [process where true] [network where true]",
            )),
        )
        .await
        .unwrap();
    let sequences = sequence.hits.as_ref().unwrap().sequences.as_ref().unwrap();
    assert_eq!(sequences.len(), 1);
    let ids: Vec<Option<&str>> = sequences[0]
        .events
        .iter()
        .map(|e| e.id.as_deref())
        .collect();
    assert_eq!(ids, [Some("1"), Some("2")]);

    // Kept after it completes, to be read again by its id.
    let kept = client
        .send(
            &search::Request::new(test.name())
                .keep_on_completion(true)
                .wait_for_completion_timeout(Duration::String("10s".into()))
                .body(search::RequestBody::new("any where true")),
        )
        .await
        .unwrap();
    let id = kept.id.clone().expect("a kept search's id");
    let again = client.send(&get::Request::new(id.as_str())).await.unwrap();
    assert_eq!(
        again
            .hits
            .as_ref()
            .and_then(|h| h.events.as_ref())
            .map(Vec::len),
        Some(3)
    );
    let status = client
        .send(&get_status::Request::new(id.as_str()))
        .await
        .unwrap();
    assert_eq!(status.is_running, Some(false));
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
