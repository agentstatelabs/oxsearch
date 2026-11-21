//! Features against Elasticsearch, on any license: the features whose
//! state a snapshot can include, and a reset of every one, which deletes
//! that state, as Elasticsearch's own tests do between runs. The test
//! binaries run one at a time, so the reset takes nothing from a test
//! running beside it.

use oxsearch::features::{get_features, reset_features};
use oxsearch_integration::client;

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn features() {
    let client = client();
    let listed = client.send(&get_features::Request::new()).await.unwrap();
    let names: Vec<&str> = listed
        .features
        .iter()
        .filter_map(|f| f.name.as_deref())
        .collect();
    assert!(names.contains(&"tasks"), "{names:?}");

    let reset = client.send(&reset_features::Request::new()).await.unwrap();
    let tasks = reset
        .features
        .iter()
        .find(|f| f["feature_name"] == "tasks")
        .expect("the tasks feature");
    assert_eq!(tasks["status"], "SUCCESS", "{reset:?}");
}
