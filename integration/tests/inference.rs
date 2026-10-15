//! The inference API: the endpoints a cluster has, including the ones
//! Elasticsearch defines itself.

use oxsearch_inference::get;
use oxsearch_integration::client;

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn lists_inference_endpoints() {
    let response = client().send(&get::Request::new()).await.unwrap();
    let ids: Vec<&str> = response
        .endpoints
        .iter()
        .filter_map(|e| e.inference_id.as_deref())
        .collect();
    assert!(ids.contains(&".elser-2-elasticsearch"), "{ids:?}");
    for endpoint in &response.endpoints {
        assert!(endpoint.service.is_some(), "{endpoint:?}");
        assert!(endpoint.task_type.is_some(), "{endpoint:?}");
    }
}
