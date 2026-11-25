//! The cluster's TLS certificates and encryption key against
//! Elasticsearch, on any license: the certificates, none on a fixture
//! without TLS, and the key reset.

use oxsearch::security::{encryption, ssl};
use oxsearch_integration::client;

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn certificates() {
    let client = client();
    let certificates = client
        .send(&ssl::certificates::Request::new())
        .await
        .unwrap();
    assert!(certificates.is_empty(), "{certificates:?}");
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn key_reset() {
    let client = client();
    let reset = client
        .send(&encryption::reset::Request::new(true))
        .await
        .unwrap();
    assert_eq!(reset.acknowledged, Some(true));
}
