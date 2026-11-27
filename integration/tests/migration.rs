//! Migration against Elasticsearch, on any license: the deprecated settings
//! the cluster and an index use, the system features' upgrade status, and
//! an upgrade asked for, which a cluster on the current version doesn't
//! need.

use oxsearch::Method;
use oxsearch::migration::get_feature_upgrade_status::MigrationStatus;
use oxsearch::migration::{deprecations, get_feature_upgrade_status, post_feature_upgrade};
use oxsearch_integration::{TestIndex, client};

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn upgrades() {
    let client = client();
    let cluster = client.send(&deprecations::Request::new()).await.unwrap();
    assert!(cluster.cluster_settings.is_empty(), "{cluster:?}");

    let test = TestIndex::named("migration");
    client
        .send_raw(Method::Put, &format!("/{}", test.name()), None)
        .await
        .unwrap();
    let index = client
        .send(&deprecations::Request::new().index(test.name()))
        .await
        .unwrap();
    assert!(!index.index_settings.contains_key(test.name()), "{index:?}");

    let status = client
        .send(&get_feature_upgrade_status::Request::new())
        .await
        .unwrap();
    assert!(!status.features.is_empty(), "{status:?}");
    assert_eq!(
        status.migration_status,
        Some(MigrationStatus::NoMigrationNeeded),
        "{status:?}"
    );
    let upgrade = client
        .send(&post_feature_upgrade::Request::new())
        .await
        .unwrap();
    assert_eq!(upgrade.accepted, Some(false), "{upgrade:?}");
    assert!(upgrade.reason.is_some(), "{upgrade:?}");

    test.delete(&client).await;
}
