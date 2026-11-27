//! Node shutdown against Elasticsearch, on any license: the fixture's node
//! marked for a restart, which keeps it running and moves no shards, read
//! back with its progress, and the mark taken off again. The mark comes
//! off before the checks, so a failing check doesn't leave it for the
//! tests after.

use oxsearch::admin::nodes;
use oxsearch::shutdown::get_node::{ShutdownStatus, ShutdownType};
use oxsearch::shutdown::{Type, delete_node, get_node, put_node};
use oxsearch_integration::client;

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn restart() {
    let client = client();
    let info = client.send(&nodes::info::Request::new()).await.unwrap();
    let node = info.nodes.keys().next().expect("a node").clone();

    let put = client
        .send(
            &put_node::Request::new(node.as_str()).body(
                put_node::RequestBody::new(Type::Restart, "oxsearch's shutdown test")
                    .allocation_delay("10m"),
            ),
        )
        .await;
    let read = client
        .send(&get_node::Request::new().node_id(node.as_str()))
        .await;
    let deleted = client.send(&delete_node::Request::new(node.as_str())).await;

    assert_eq!(put.unwrap().0.acknowledged, Some(true));
    let read = read.unwrap();
    let status = &read.nodes[0];
    assert_eq!(status.node_id.as_deref(), Some(node.as_str()));
    assert_eq!(status.r#type, Some(ShutdownType::Restart), "{status:?}");
    assert_eq!(status.reason.as_deref(), Some("oxsearch's shutdown test"));
    assert!(status.shutdown_started_millis.is_some(), "{status:?}");
    // A restart moves no shards.
    let shards = status.shard_migration.as_ref().expect("the shards' status");
    assert_eq!(shards.status, Some(ShutdownStatus::Complete), "{shards:?}");
    assert_eq!(deleted.unwrap().0.acknowledged, Some(true));

    let after = client.send(&get_node::Request::new()).await.unwrap();
    assert!(after.nodes.is_empty(), "{after:?}");
}
