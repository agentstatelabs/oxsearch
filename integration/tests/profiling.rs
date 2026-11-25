//! Universal Profiling against Elasticsearch: its status, and the stack
//! traces, flame graph and top functions of a query, which need a license
//! above Basic. The fixture has no profiles to read.

use oxsearch::Error;
use oxsearch::profiling::{flamegraph, stacktraces, status, topn_functions};
use oxsearch_integration::{License, client};
use serde_json::json;

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn profiles() {
    let client = client();
    let status = client.send(&status::Request::new()).await;
    let query = json!({"sample_size": 10, "query": {"match_all": {}}});
    let traces = client
        .send(&stacktraces::Request::new().body(query.clone()))
        .await
        .map(|r| format!("{r:?}"));
    let graph = client
        .send(&flamegraph::Request::new().body(query.clone()))
        .await
        .map(|r| format!("{r:?}"));
    let functions = client
        .send(&topn_functions::Request::new().body(query))
        .await
        .map(|r| format!("{r:?}"));
    let all = format!("{status:?}\n{traces:?}\n{graph:?}\n{functions:?}");
    assert!(status.is_ok_and(|s| s.operation_mode.is_some()), "{all}");
    for result in [traces, graph, functions] {
        if License::from_env() == License::Basic {
            assert!(matches!(result, Err(Error::License(_))), "{all}");
        } else {
            assert!(result.is_ok(), "{all}");
        }
    }
}
