//! Snapshot lifecycle management against Elasticsearch, on any license: a
//! policy that snapshots an index every night into a repository on the
//! node's file system, run now, its statistics and retention, and the
//! service stopped and started.

use std::time::{Duration, Instant};

use oxsearch::LifecycleOperationMode;
use oxsearch::Refresh;
use oxsearch::slm::{
    Configuration, Retention, delete_lifecycle, execute_lifecycle, execute_retention,
    get_lifecycle, get_stats, get_status, put_lifecycle, start, stop,
};
use oxsearch::snapshot::{
    Repository, SharedFileSystemRepository, SharedFileSystemRepositorySettings, create_repository,
    delete, delete_repository,
};
use oxsearch_docs::index;
use oxsearch_integration::{TestClient, TestIndex, client};
use serde_json::json;

/// Waits until snapshot lifecycle management is in `mode`.
async fn until(client: &TestClient, mode: LifecycleOperationMode) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let status = client.send(&get_status::Request::new()).await.unwrap();
        if status.operation_mode.as_ref() == Some(&mode) {
            return;
        }
        assert!(Instant::now() < deadline, "SLM isn't {mode:?}: {status:?}");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn policies() {
    let client = client();
    let test = TestIndex::named("slm");
    client
        .send(
            &index::Request::new(test.name())
                .refresh(Refresh::True)
                .body(json!({"title": "kept nightly"})),
        )
        .await
        .unwrap();
    let repository = format!("{}-repo", test.name());
    client
        .send(
            &create_repository::Request::new(repository.as_str()).body(Repository::from(
                SharedFileSystemRepository::new(SharedFileSystemRepositorySettings::new(
                    repository.clone(),
                )),
            )),
        )
        .await
        .unwrap();

    // Every night at 01:30, the index, kept for 30 days: at least one
    // snapshot and at most five.
    let policy = format!("{}-nightly", test.name());
    client
        .send(
            &put_lifecycle::Request::new(policy.as_str()).body(
                put_lifecycle::RequestBody::new()
                    .name("<nightly-{now/d}>")
                    .schedule("0 30 1 * * ?")
                    .repository(repository.as_str())
                    // Only the index: the global state takes in every
                    // feature's state, machine learning's models among them,
                    // and would take minutes to snapshot.
                    .config(
                        Configuration::new()
                            .indices(test.name())
                            .include_global_state(false),
                    )
                    .retention(Retention::new("30d", 5, 1)),
            ),
        )
        .await
        .unwrap();
    let read = client
        .send(&get_lifecycle::Request::new().policy_id(policy.as_str()))
        .await
        .unwrap();
    assert!(read.contains_key(policy.as_str()), "{read:?}");

    // Run it now, and wait for the snapshot.
    let ran = client
        .send(&execute_lifecycle::Request::new(policy.as_str()))
        .await
        .unwrap();
    let snapshot = ran.snapshot_name.expect("the snapshot's name");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let read = client
            .send(&get_lifecycle::Request::new().policy_id(policy.as_str()))
            .await
            .unwrap();
        let last = read[policy.as_str()].last_success.as_ref();
        if last.and_then(|l| l.snapshot_name.as_deref()) == Some(snapshot.as_str()) {
            break;
        }
        assert!(Instant::now() < deadline, "no snapshot: {read:?}");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let stats = client.send(&get_stats::Request::new()).await.unwrap();
    assert!(
        stats.total_snapshots_taken.unwrap_or_default() >= 1,
        "{stats:?}"
    );
    client
        .send(&execute_retention::Request::new())
        .await
        .unwrap();

    // The service, stopped and started again.
    until(&client, LifecycleOperationMode::Running).await;
    client.send(&stop::Request::new()).await.unwrap();
    until(&client, LifecycleOperationMode::Stopped).await;
    client.send(&start::Request::new()).await.unwrap();
    until(&client, LifecycleOperationMode::Running).await;

    client
        .send(&delete_lifecycle::Request::new(policy.as_str()))
        .await
        .unwrap();
    client
        .send(&delete::Request::new(
            repository.as_str(),
            snapshot.as_str(),
        ))
        .await
        .unwrap();
    client
        .send(&delete_repository::Request::new(repository.as_str()))
        .await
        .unwrap();
    test.delete(&client).await;
}
