//! Index lifecycle management against Elasticsearch: a policy written and
//! read, an index managed by it and moved through its phases, and ILM
//! stopped and started. One test, so that stopping ILM can't hold up an
//! index another test is moving.

use oxsearch::admin::indices::{IndexSettings, IndexSettingsLifecycle, create};
use oxsearch::ilm::explain_lifecycle::LifecycleExplain;
use oxsearch::ilm::move_to_step::StepKey;
use oxsearch::ilm::{
    Actions, Phase, Phases, Policy, delete_lifecycle, explain_lifecycle, get_lifecycle, get_status,
    migrate_to_data_tiers, move_to_step, put_lifecycle, remove_policy, retry, start, stop,
};
use oxsearch::{Duration, EmptyObject, Error, LifecycleOperationMode};
use oxsearch_integration::{TestClient, TestIndex, client};

/// The index's phase, action and step, as ILM explains them.
async fn step(client: &TestClient, index: &str) -> (String, String, String) {
    let explained = client
        .send(&explain_lifecycle::Request::new(index))
        .await
        .unwrap();
    let Some(LifecycleExplain::Managed(managed)) = explained.indices.get(index) else {
        panic!("{index} isn't managed: {explained:?}");
    };
    (
        managed.phase.clone().unwrap_or_default(),
        managed.action.clone().unwrap_or_default(),
        managed.step.clone().unwrap_or_default(),
    )
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn lifecycle() {
    let client = client();
    let policy = TestIndex::named("policy").name().to_owned();

    // Hot at once, then read-only after 30 days.
    client
        .send(
            &put_lifecycle::Request::new(policy.as_str()).body(
                put_lifecycle::RequestBody::new().policy(Policy::new(
                    Phases::new()
                        .hot(Phase::new().actions(Actions::new()))
                        .warm(
                            Phase::new()
                                .min_age(Duration::String("30d".into()))
                                .actions(Actions::new().readonly(EmptyObject::new())),
                        ),
                )),
            ),
        )
        .await
        .unwrap();
    let read = client
        .send(&get_lifecycle::Request::new().policy(policy.as_str()))
        .await
        .unwrap();
    let phases = read.0[&policy]
        .policy
        .as_ref()
        .and_then(|p| p.phases.as_ref())
        .unwrap();
    assert!(phases.warm.is_some() && phases.delete.is_none());

    // An index the policy manages, which enters the `hot` phase at once.
    let test = TestIndex::named("managed");
    client
        .send(
            &create::Request::new(test.name()).body(create::RequestBody::new().settings(
                IndexSettings::new().lifecycle(IndexSettingsLifecycle::new().name(policy.as_str())),
            )),
        )
        .await
        .unwrap();
    // Moved on to the warm phase without waiting 30 days, from the step it
    // is on. That's the end of `new` or, once ILM has moved it on, of
    // `hot`; if it moves between the two calls, the move names a step it
    // has left, and is tried again.
    let mut moved = false;
    for _ in 0..20 {
        let (phase, action, name) = step(&client, test.name()).await;
        assert!(["new", "hot"].contains(&phase.as_str()), "{phase}");
        let request = move_to_step::Request::new(test.name()).body(move_to_step::RequestBody::new(
            StepKey::new(phase).action(action).name(name),
            StepKey::new("warm"),
        ));
        match client.send(&request).await {
            Ok(_) => {
                moved = true;
                break;
            }
            Err(Error::Elasticsearch(e)) if e.status == 400 => {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
            Err(e) => panic!("{e}"),
        }
    }
    assert!(moved, "the index never rested on a step");
    assert_eq!(step(&client, test.name()).await.0, "warm");

    // Only a failed step can be retried.
    let retried = client.send(&retry::Request::new(test.name())).await;
    assert!(
        matches!(&retried, Err(Error::Elasticsearch(e)) if e.status == 400),
        "{retried:?}"
    );

    let removed = client
        .send(&remove_policy::Request::new(test.name()))
        .await
        .unwrap();
    assert_eq!(removed.has_failures, Some(false));
    let explained = client
        .send(&explain_lifecycle::Request::new(test.name()))
        .await
        .unwrap();
    assert!(
        matches!(
            explained.indices.get(test.name()),
            Some(LifecycleExplain::Unmanaged(_))
        ),
        "{explained:?}"
    );
    test.delete(&client).await;
    client
        .send(&delete_lifecycle::Request::new(policy.as_str()))
        .await
        .unwrap();

    stopped_and_started(&client).await;
}

/// ILM stopped and started again; after the policies, since a stopped ILM
/// moves no index on.
async fn stopped_and_started(client: &TestClient) {
    let status = client.send(&get_status::Request::new()).await.unwrap();
    assert_eq!(status.operation_mode, Some(LifecycleOperationMode::Running));

    client.send(&stop::Request::new()).await.unwrap();
    // Migrating to data tiers needs ILM stopped; a dry run changes nothing.
    let mut migrated = None;
    for _ in 0..40 {
        match client
            .send(&migrate_to_data_tiers::Request::new().dry_run(true))
            .await
        {
            Ok(response) => {
                migrated = Some(response);
                break;
            }
            // Until ILM has stopped, it refuses.
            Err(Error::Elasticsearch(e)) if e.status == 400 => {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
            Err(e) => panic!("{e}"),
        }
    }
    let migrated = migrated.expect("ILM stopped");
    assert_eq!(migrated.dry_run, Some(true));

    client.send(&start::Request::new()).await.unwrap();
    let status = client.send(&get_status::Request::new()).await.unwrap();
    assert_eq!(status.operation_mode, Some(LifecycleOperationMode::Running));
}
