//! Watcher against Elasticsearch: a watch put, read, run, acknowledged,
//! deactivated and activated again, found by a query and deleted; a watch
//! run without being stored; the service's settings and statistics; and
//! the service stopped and started.
//!
//! Watcher needs a license above Basic, so on Basic the test checks that
//! putting a watch is refused, and stops there.

use std::time::{Duration, Instant};

use oxsearch::Error;
use oxsearch::shapes::IndexMap;
use oxsearch::watcher::stats::WatcherState;
use oxsearch::watcher::{
    Action, AlwaysCondition, ConditionContainer, ExecutionStatus, InputContainer, LoggingAction,
    ScheduleContainer, TriggerContainer, Watch, ack_watch, activate_watch, deactivate_watch,
    delete_watch, execute_watch, get_settings, get_watch, put_watch, query_watches, start, stats,
    stop, update_settings,
};
use oxsearch_integration::{License, TestClient, TestIndex, client};
use serde_json::json;

/// A watch that runs every hour, takes a fixed payload, and logs it.
fn hourly() -> put_watch::RequestBody {
    let mut payload = IndexMap::new();
    payload.insert("disk_used_percent".to_owned(), json!(97));
    let mut actions = IndexMap::new();
    actions.insert(
        "log".to_owned(),
        Action::new().logging(LoggingAction::new(
            "disk at {{ctx.payload.disk_used_percent}}%",
        )),
    );
    put_watch::RequestBody::new()
        .trigger(TriggerContainer::Schedule(ScheduleContainer::Interval(
            "1h".into(),
        )))
        .input(InputContainer::Simple(payload))
        .condition(ConditionContainer::Always(AlwaysCondition::new()))
        .actions(actions)
}

/// Waits until Watcher is in `state`.
async fn until(client: &TestClient, state: WatcherState) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let stats = client.send(&stats::Request::new()).await.unwrap();
        if stats
            .stats
            .iter()
            .all(|s| s.watcher_state.as_ref() == Some(&state))
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Watcher isn't {state:?}: {stats:?}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn watches() {
    let client = client();
    let id = TestIndex::named("watch").name().to_owned();
    let put = client
        .send(&put_watch::Request::new(id.as_str()).body(hourly()))
        .await;
    if License::from_env() == License::Basic {
        assert!(matches!(put, Err(Error::License(_))), "{put:?}");
        return;
    }
    assert_eq!(put.unwrap().created, Some(true));

    let read = client
        .send(&get_watch::Request::new(id.as_str()))
        .await
        .unwrap();
    assert_eq!(read.found, Some(true));
    let watch = read.watch.expect("the watch");
    assert!(watch.actions.contains_key("log"), "{watch:?}");

    // Run it now, whatever its condition says, and keep the record.
    let ran = client
        .send(
            &execute_watch::Request::new().id(id.as_str()).body(
                execute_watch::RequestBody::new()
                    .ignore_condition(true)
                    .record_execution(true),
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        ran.watch_record.and_then(|r| r.state),
        Some(ExecutionStatus::Executed)
    );
    let acked = client
        .send(&ack_watch::Request::new(id.as_str()))
        .await
        .unwrap();
    assert!(acked.status.is_some(), "{acked:?}");

    let inactive = client
        .send(&deactivate_watch::Request::new(id.as_str()))
        .await
        .unwrap();
    assert_eq!(
        inactive.status.and_then(|s| s.state).and_then(|s| s.active),
        Some(false)
    );
    let active = client
        .send(&activate_watch::Request::new(id.as_str()))
        .await
        .unwrap();
    assert_eq!(
        active.status.and_then(|s| s.state).and_then(|s| s.active),
        Some(true)
    );

    let found = client
        .send(&query_watches::Request::new().body(query_watches::RequestBody::new().size(100)))
        .await
        .unwrap();
    assert!(
        found
            .watches
            .iter()
            .any(|w| w.id.as_deref() == Some(id.as_str())),
        "{found:?}"
    );

    // A watch run once, from the request, without storing it.
    let mut payload = IndexMap::new();
    payload.insert("queue_depth".to_owned(), json!(12));
    let mut actions = IndexMap::new();
    actions.insert(
        "log".to_owned(),
        Action::new().logging(LoggingAction::new("queue at {{ctx.payload.queue_depth}}")),
    );
    let inline = client
        .send(
            &execute_watch::Request::new().body(execute_watch::RequestBody::new().watch(
                Watch::new(
                    actions,
                    InputContainer::Simple(payload),
                    TriggerContainer::Schedule(ScheduleContainer::Interval("10m".into())),
                ),
            )),
        )
        .await
        .unwrap();
    assert_eq!(
        inline.watch_record.and_then(|r| r.state),
        Some(ExecutionStatus::Executed)
    );

    // The settings of Watcher's own index, which the watch created.
    client.send(&get_settings::Request::new()).await.unwrap();
    let updated = client
        .send(
            &update_settings::Request::new()
                .body(update_settings::RequestBody::new().index_auto_expand_replicas("0-1")),
        )
        .await
        .unwrap();
    assert_eq!(updated.acknowledged, Some(true));

    let deleted = client
        .send(&delete_watch::Request::new(id.as_str()))
        .await
        .unwrap();
    assert_eq!(deleted.found, Some(true));
    let gone = client.send(&get_watch::Request::new(id.as_str())).await;
    match gone {
        Ok(read) => assert_eq!(read.found, Some(false), "{read:?}"),
        Err(Error::Elasticsearch(e)) => assert_eq!(e.status, 404),
        Err(e) => panic!("{e}"),
    }

    // The service itself: its statistics, stopped and started again.
    let before = client.send(&stats::Request::new()).await.unwrap();
    assert!(!before.stats.is_empty(), "{before:?}");
    client.send(&stop::Request::new()).await.unwrap();
    until(&client, WatcherState::Stopped).await;
    client.send(&start::Request::new()).await.unwrap();
    until(&client, WatcherState::Started).await;
}
