//! The fixture is what the job asked for: Elasticsearch 9 on the expected
//! license.

use oxsearch_core::Method;
use oxsearch_integration::{License, client};

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn runs_elasticsearch_9() {
    let info = client()
        .send(&oxsearch_core::info::Request::new())
        .await
        .unwrap();
    let version = info
        .version
        .expect("info reports a version")
        .number
        .expect("and its number");
    assert!(version.starts_with("9."), "Elasticsearch {version}");
    if let Ok(expected) = std::env::var("ELASTICSEARCH_VERSION") {
        assert_eq!(version, expected);
    }
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn runs_the_expected_license() {
    // The license API isn't in the first slice's crates: the raw call.
    let license = client()
        .send_raw(Method::Get, "/_license", None)
        .await
        .unwrap();
    assert_eq!(license["license"]["type"], License::from_env().as_str());
    assert_eq!(license["license"]["status"], "active");
}
