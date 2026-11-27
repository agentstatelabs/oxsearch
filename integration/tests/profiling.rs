//! Universal Profiling against Elasticsearch: its status, and the stack
//! traces, flame graph and top functions of a query. The queries need a
//! license above Basic, and indices the fixture doesn't have, so on a trial
//! each gets as far as looking for them.

use oxsearch::Error;
use oxsearch::profiling::{flamegraph, stacktraces, status, topn_functions};
use oxsearch_integration::{License, client};
use serde_json::json;

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn profiles() {
    let client = client();
    let status = client.send(&status::Request::new()).await.unwrap();
    assert_eq!(status["resources"]["has_data"], false, "{status:?}");

    let query = json!({"sample_size": 10, "query": {"match_all": {}}});
    let results = [
        client
            .send(&stacktraces::Request::new().body(query.clone()))
            .await
            .map(drop),
        client
            .send(&flamegraph::Request::new().body(query.clone()))
            .await
            .map(drop),
        client
            .send(&topn_functions::Request::new().body(query))
            .await
            .map(drop),
    ];
    for result in results {
        match (License::from_env(), result) {
            (License::Basic, Err(Error::License(_))) => {}
            (License::Trial, Err(Error::Elasticsearch(e))) => {
                assert_eq!(e.error_type(), Some("index_not_found_exception"), "{e}")
            }
            (license, other) => panic!("on {license:?}: {other:?}"),
        }
    }
}
