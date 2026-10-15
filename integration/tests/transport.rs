//! The reqwest transport's layers against Elasticsearch: failover past a
//! node that's down, and sniffing the cluster's own node list.

use std::time::Duration;

use oxsearch_core::Client;
use oxsearch_core::info;
use oxsearch_transport_reqwest::ReqwestTransport;

fn url() -> String {
    std::env::var("OXSEARCH_TEST_URL").unwrap_or_else(|_| "http://127.0.0.1:9200".to_owned())
}

/// A URL nothing listens on.
fn down() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    format!("http://{}", listener.local_addr().unwrap())
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn fails_over_past_a_node_that_is_down() {
    let transport = ReqwestTransport::builder()
        .nodes([down(), url()])
        .retries(1)
        .build()
        .unwrap();
    let client = Client::new(transport);
    for _ in 0..3 {
        let response = client.send(&info::Request::new()).await.unwrap();
        assert!(response.version.is_some());
    }
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn sniffs_the_cluster() {
    // Seeded with a node that's down and the real one: sniffing replaces
    // both with what the cluster publishes.
    let transport = ReqwestTransport::builder()
        .nodes([down(), url()])
        .retries(2)
        .sniff_every(Duration::from_secs(3600))
        .build()
        .unwrap();
    let client = Client::new(transport.clone());
    client.send(&info::Request::new()).await.unwrap();
    client.send(&info::Request::new()).await.unwrap();
    let port = url()
        .rsplit_once(':')
        .unwrap()
        .1
        .trim_end_matches('/')
        .to_owned();
    assert_eq!(transport.nodes(), [format!("http://127.0.0.1:{port}")]);
}
