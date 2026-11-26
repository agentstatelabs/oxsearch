//! Streams against Elasticsearch, on any license: the logs stream's status,
//! and the stream turned on and off again.

use oxsearch::streams::{StreamType, logs_disable, logs_enable, status};
use oxsearch_integration::{TestClient, client};

/// Whether the logs stream is on.
async fn enabled(client: &TestClient) -> Option<bool> {
    let status = client.send(&status::Request::new()).await.unwrap();
    status.logs.and_then(|s| s.enabled)
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn logs() {
    let client = client();
    assert_eq!(enabled(&client).await, Some(false));
    let on = client
        .send(&logs_enable::Request::new(StreamType::Logs))
        .await
        .unwrap();
    let while_on = enabled(&client).await;
    let off = client
        .send(&logs_disable::Request::new(StreamType::Logs))
        .await
        .unwrap();

    // Off before the checks, so a failing one doesn't leave it on.
    assert_eq!(on.acknowledged, Some(true));
    assert_eq!(while_on, Some(true));
    assert_eq!(off.acknowledged, Some(true));
    assert_eq!(enabled(&client).await, Some(false));
}
