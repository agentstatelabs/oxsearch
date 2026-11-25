//! Connectors against Elasticsearch, on any license: a connector created,
//! read, listed, changed in each of the ways it can be, and deleted; its
//! sync jobs, queued, claimed, reported on, failed, canceled and deleted;
//! and the secrets connectors keep. No connector service runs on the
//! fixture, so the test plays its part.

use oxsearch::connector::{
    ConnectorConfigProperties, ConnectorFeatures, ConnectorScheduling, ConnectorStatus,
    DisplayType, FeatureEnabled, FilteringPolicy, FilteringRule, FilteringRuleRule,
    FilteringRulesValidation, FilteringValidationState, IngestPipelineParams,
    SchedulingConfiguration, SyncJobTriggerMethod, SyncJobType, SyncStatus, check_in, delete, get,
    last_sync, list, post, put, secret_delete, secret_get, secret_post, secret_put,
    sync_job_cancel, sync_job_check_in, sync_job_claim, sync_job_delete, sync_job_error,
    sync_job_get, sync_job_list, sync_job_post, sync_job_update_stats, update_active_filtering,
    update_api_key_id, update_configuration, update_error, update_features, update_filtering,
    update_filtering_validation, update_index_name, update_name, update_native, update_pipeline,
    update_scheduling, update_service_type, update_status,
};
use oxsearch::shapes::{IndexMap, WithNullValue};
use oxsearch::{Error, Result as Outcome};
use oxsearch_integration::{TestClient, TestIndex, client};
use serde_json::json;

/// The connector, as `get` reads it.
async fn read(client: &TestClient, id: &str) -> oxsearch::connector::Connector {
    client.send(&get::Request::new(id)).await.unwrap().0
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn connectors() {
    let client = client();
    let test = TestIndex::named("connector");
    let id = test.name();
    let index = format!("{id}-content");

    // Created with an ID, and without one, which gives it one.
    let created = client
        .send(
            &put::Request::new().connector_id(id).body(
                put::RequestBody::new()
                    .index_name(index.as_str())
                    .name("notes")
                    .service_type("google_drive"),
            ),
        )
        .await
        .unwrap();
    assert_eq!(created.result, Some(Outcome::Created));
    let unnamed = client
        .send(&post::Request::new().body(post::RequestBody::new().name("unnamed")))
        .await
        .unwrap()
        .id
        .expect("the connector's ID");

    let connector = read(&client, id).await;
    assert_eq!(connector.index_name.as_deref(), Some(index.as_str()));
    assert_eq!(connector.status, Some(ConnectorStatus::Created));
    let listed = client
        .send(&list::Request::new().index_name(index.as_str()))
        .await
        .unwrap();
    assert_eq!(listed.count, Some(1), "{listed:?}");
    assert_eq!(listed.results[0].id.as_deref(), Some(id));

    // Renamed, moved to another index and service, and given a key.
    client
        .send(
            &update_name::Request::new(id).body(
                update_name::RequestBody::new()
                    .name("renamed")
                    .description("a test's notes"),
            ),
        )
        .await
        .unwrap();
    let moved = format!("{id}-moved");
    client
        .send(
            &update_index_name::Request::new(id).body(update_index_name::RequestBody::new(
                WithNullValue::from(moved.clone()),
            )),
        )
        .await
        .unwrap();
    client
        .send(
            &update_service_type::Request::new(id)
                .body(update_service_type::RequestBody::new("sharepoint_online")),
        )
        .await
        .unwrap();
    client
        .send(&update_native::Request::new(id).body(update_native::RequestBody::new(false)))
        .await
        .unwrap();
    client
        .send(
            &update_api_key_id::Request::new(id).body(
                update_api_key_id::RequestBody::new()
                    .api_key_id("key")
                    .api_key_secret_id("secret"),
            ),
        )
        .await
        .unwrap();
    let connector = read(&client, id).await;
    assert_eq!(connector.name.as_deref(), Some("renamed"));
    assert_eq!(connector.description.as_deref(), Some("a test's notes"));
    assert_eq!(connector.index_name.as_deref(), Some(moved.as_str()));
    assert_eq!(connector.service_type.as_deref(), Some("sharepoint_online"));
    assert_eq!(connector.is_native, Some(false));
    assert_eq!(connector.api_key_id.as_deref(), Some("key"));

    // Configured, with a property's value, and connected.
    let mut configuration = IndexMap::new();
    configuration.insert(
        "folder".to_owned(),
        ConnectorConfigProperties::new(
            "",
            vec![],
            DisplayType::Textbox,
            "Folder",
            vec![],
            true,
            false,
            json!("notes"),
        ),
    );
    client
        .send(
            &update_configuration::Request::new(id)
                .body(update_configuration::RequestBody::new().configuration(configuration)),
        )
        .await
        .unwrap();
    let connector = read(&client, id).await;
    let configuration = connector.configuration.as_ref().expect("the configuration");
    assert_eq!(
        configuration["folder"].value,
        Some(json!("notes")),
        "{configuration:?}"
    );
    assert_eq!(connector.status, Some(ConnectorStatus::Configured));
    client
        .send(
            &update_status::Request::new(id)
                .body(update_status::RequestBody::new(ConnectorStatus::Connected)),
        )
        .await
        .unwrap();
    assert_eq!(
        read(&client, id).await.status,
        Some(ConnectorStatus::Connected)
    );

    // What it can do, when it syncs, and its ingest pipeline.
    client
        .send(
            &update_features::Request::new(id).body(update_features::RequestBody::new(
                ConnectorFeatures::new().incremental_sync(FeatureEnabled::new(true)),
            )),
        )
        .await
        .unwrap();
    client
        .send(
            &update_scheduling::Request::new(id).body(update_scheduling::RequestBody::new(
                SchedulingConfiguration::new().full(ConnectorScheduling::new(true, "0 0 0 * * ?")),
            )),
        )
        .await
        .unwrap();
    client
        .send(
            &update_pipeline::Request::new(id).body(update_pipeline::RequestBody::new(
                IngestPipelineParams::new(true, "notes-pipeline", true, false),
            )),
        )
        .await
        .unwrap();
    let connector = read(&client, id).await;
    let features = connector.features.as_ref().expect("the features");
    assert_eq!(
        features.incremental_sync.as_ref().and_then(|f| f.enabled),
        Some(true)
    );
    let full = connector.scheduling.as_ref().and_then(|s| s.full.as_ref());
    assert_eq!(
        full.and_then(|f| f.interval.as_deref()),
        Some("0 0 0 * * ?")
    );
    let pipeline = connector.pipeline.as_ref().expect("the pipeline");
    assert_eq!(pipeline.name.as_deref(), Some("notes-pipeline"));

    // Filtering rules, drafted, validated and activated.
    client
        .send(
            &update_filtering::Request::new(id).body(update_filtering::RequestBody::new().rules(
                vec![
                    FilteringRule::new(
                        "name",
                        "drafts",
                        0,
                        FilteringPolicy::Exclude,
                        FilteringRuleRule::StartsWith,
                        "draft-",
                    ),
                    FilteringRule::new(
                        "_",
                        "DEFAULT",
                        1,
                        FilteringPolicy::Include,
                        FilteringRuleRule::Regex,
                        ".*",
                    ),
                ],
            )),
        )
        .await
        .unwrap();
    client
        .send(&update_filtering_validation::Request::new(id).body(
            update_filtering_validation::RequestBody::new(FilteringRulesValidation::new(
                vec![],
                FilteringValidationState::Valid,
            )),
        ))
        .await
        .unwrap();
    client
        .send(&update_active_filtering::Request::new(id))
        .await
        .unwrap();
    let connector = read(&client, id).await;
    let active = connector.filtering[0].active.as_ref().expect("the rules");
    assert!(
        active
            .rules
            .iter()
            .any(|r| r.id.as_deref() == Some("drafts")),
        "{active:?}"
    );

    // What the connector service reports: it's alive, how a sync went, and
    // that something broke.
    client.send(&check_in::Request::new(id)).await.unwrap();
    client
        .send(
            &last_sync::Request::new(id).body(
                last_sync::RequestBody::new()
                    .last_sync_status(SyncStatus::Completed)
                    .last_indexed_document_count(42),
            ),
        )
        .await
        .unwrap();
    client
        .send(
            &update_error::Request::new(id).body(update_error::RequestBody::new(
                WithNullValue::from("the folder is gone".to_owned()),
            )),
        )
        .await
        .unwrap();
    let connector = read(&client, id).await;
    assert!(connector.last_seen.is_some(), "{connector:?}");
    assert_eq!(connector.last_sync_status, Some(SyncStatus::Completed));
    assert_eq!(connector.last_indexed_document_count, Some(42));
    assert_eq!(connector.error.as_deref(), Some("the folder is gone"));
    assert_eq!(connector.status, Some(ConnectorStatus::Error));

    for id in [id, unnamed.as_str()] {
        let deleted = client
            .send(&delete::Request::new(id).delete_sync_jobs(true).hard(true))
            .await
            .unwrap();
        assert_eq!(deleted.acknowledged, Some(true));
    }
    match client.send(&get::Request::new(id)).await {
        Err(Error::Elasticsearch(e)) => assert_eq!(e.status, 404, "{e}"),
        other => panic!("a deleted connector: {other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn sync_jobs() {
    let client = client();
    let test = TestIndex::named("connector-sync");
    let id = test.name();
    client
        .send(
            &put::Request::new().connector_id(id).body(
                put::RequestBody::new()
                    .index_name(format!("{id}-content"))
                    .service_type("google_drive"),
            ),
        )
        .await
        .unwrap();
    let queue = || async {
        client
            .send(
                &sync_job_post::Request::new().body(
                    sync_job_post::RequestBody::new(id)
                        .job_type(SyncJobType::Full)
                        .trigger_method(SyncJobTriggerMethod::OnDemand),
                ),
            )
            .await
            .unwrap()
            .id
            .expect("the job's ID")
    };
    let job = |id: String| {
        let client = &client;
        async move {
            client
                .send(&sync_job_get::Request::new(id))
                .await
                .unwrap()
                .0
        }
    };

    // Queued, claimed by a worker, reported on, and failed.
    let failing = queue().await;
    assert_eq!(job(failing.clone()).await.status, Some(SyncStatus::Pending));
    client
        .send(
            &sync_job_claim::Request::new(failing.as_str())
                .body(sync_job_claim::RequestBody::new("worker-1")),
        )
        .await
        .unwrap();
    client
        .send(&sync_job_check_in::Request::new(failing.as_str()))
        .await
        .unwrap();
    client
        .send(
            &sync_job_update_stats::Request::new(failing.as_str())
                .body(sync_job_update_stats::RequestBody::new(1, 2, 3)),
        )
        .await
        .unwrap();
    let claimed = job(failing.clone()).await;
    assert_eq!(claimed.status, Some(SyncStatus::InProgress));
    assert_eq!(claimed.worker_hostname.as_deref(), Some("worker-1"));
    assert_eq!(claimed.indexed_document_count, Some(2));
    assert!(claimed.last_seen.is_some(), "{claimed:?}");
    client
        .send(
            &sync_job_error::Request::new(failing.as_str())
                .body(sync_job_error::RequestBody::new("the folder is gone")),
        )
        .await
        .unwrap();
    let failed = job(failing.clone()).await;
    assert_eq!(failed.status, Some(SyncStatus::Error));
    assert_eq!(failed.error.as_deref(), Some("the folder is gone"));

    // Queued and canceled before a worker claims it.
    let canceled = queue().await;
    client
        .send(&sync_job_cancel::Request::new(canceled.as_str()))
        .await
        .unwrap();
    assert_eq!(
        job(canceled.clone()).await.status,
        Some(SyncStatus::Canceled)
    );

    let listed = client
        .send(&sync_job_list::Request::new().connector_id(id))
        .await
        .unwrap();
    assert_eq!(listed.count, Some(2), "{listed:?}");
    for job in [failing, canceled] {
        let deleted = client
            .send(&sync_job_delete::Request::new(job))
            .await
            .unwrap();
        assert_eq!(deleted.acknowledged, Some(true));
    }
    client
        .send(&delete::Request::new(id).hard(true))
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn secrets() {
    let client = client();
    let posted = client
        .send(&secret_post::Request::new().body(secret_post::RequestBody::new().value("first")))
        .await
        .unwrap()
        .id
        .expect("the secret's ID");
    let read = client
        .send(&secret_get::Request::new(posted.as_str()))
        .await
        .unwrap();
    assert_eq!(read.value.as_deref(), Some("first"));

    // Put under its own ID, which makes or replaces it.
    client
        .send(
            &secret_put::Request::new(posted.as_str()).body(secret_put::RequestBody::new("second")),
        )
        .await
        .unwrap();
    let read = client
        .send(&secret_get::Request::new(posted.as_str()))
        .await
        .unwrap();
    assert_eq!(read.value.as_deref(), Some("second"));
    let deleted = client
        .send(&secret_delete::Request::new(posted.as_str()))
        .await
        .unwrap();
    assert_eq!(deleted.deleted, Some(true));
}
