//! The cluster's encryption key against Elasticsearch, on any license:
//! reset, which loses what the old one encrypted.

use oxsearch::security::encryption;
use oxsearch_integration::client;

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
