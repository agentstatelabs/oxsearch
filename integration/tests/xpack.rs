//! X-Pack information and usage against Elasticsearch, on Basic and on a
//! trial: the build, the license and which features it makes available,
//! and the statistics of each feature, after the other tests have used
//! them; and a monitoring document sent as Kibana would.

use oxsearch::license::LicenseType;
use oxsearch::xpack::{info, monitoring, usage};
use oxsearch_docs::bulk::{IndexOperation, OperationContainer};
use oxsearch_integration::{License, client};
use serde_json::json;

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn information_and_usage() {
    let client = client();
    let license = match License::from_env() {
        License::Basic => LicenseType::Basic,
        License::Trial => LicenseType::Trial,
    };

    let info = client.send(&info::Request::new()).await.unwrap();
    assert_eq!(info.license.and_then(|l| l.r#type), Some(license.clone()));
    assert!(info.build.is_some(), "{:?}", info.build);
    // Machine learning is enabled on the node, and available only above
    // Basic.
    let ml = info.features.and_then(|f| f.ml).expect("machine learning");
    assert_eq!(ml.enabled, Some(true));
    assert_eq!(ml.available, Some(license == LicenseType::Trial));

    // Only some categories, with the build's details left out.
    let only = client
        .send(&info::Request::new().categories(vec![info::XPackCategory::License]))
        .await
        .unwrap();
    assert!(only.license.is_some() && only.build.is_none(), "{only:?}");

    let usage = client.send(&usage::Request::new()).await.unwrap();
    let ml = usage.ml.as_ref().expect("machine learning's usage");
    assert_eq!(ml.available, Some(license == LicenseType::Trial));
    assert!(usage.slm.is_some() && usage.watcher.is_some(), "{usage:?}");
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn monitored() {
    let client = client();
    let lines: Vec<monitoring::bulk::RequestBody> = vec![
        monitoring::bulk::RequestBody::OperationContainer(OperationContainer::Index(
            IndexOperation::new(),
        )),
        monitoring::bulk::RequestBody::TDocument(json!({"kibana_stats": {"requests": 1}})),
    ];
    let sent = client
        .send(&monitoring::bulk::Request::new("kibana", "7", "10s").body(lines))
        .await
        .unwrap();
    assert_eq!(sent.errors, Some(false), "{sent:?}");
}
