//! Machine learning against Elasticsearch: anomaly detection jobs fed
//! directly and through datafeeds, with their results, forecasts and model
//! snapshots; calendars and filters; data frame analytics; and trained
//! models, imported, inferred with and deployed.
//!
//! Machine learning needs a license above Basic, except for validating a
//! detector and managing filters, so on Basic each test checks that its
//! first licensed call is refused, and stops there.

use std::fmt::Debug;
use std::future::Future;
use std::time::{Duration, Instant};

use oxsearch::Refresh;
use oxsearch::ml::{
    AnalysisConfig, AnalysisMemoryLimit, CalendarEvent, DataDescription, DatafeedState,
    DataframeAnalysisContainer, DataframeAnalysisOutlierDetection, DataframeAnalyticsDestination,
    DataframeAnalyticsSource, DataframeEvaluationContainer, DataframeEvaluationOutlierDetection,
    DataframeState, DeploymentAllocationState, DeploymentAssignmentState, Detector, JobState,
    clear_trained_model_deployment_cache, close_job, delete_calendar, delete_calendar_event,
    delete_calendar_job, delete_data_frame_analytics, delete_datafeed, delete_expired_data,
    delete_filter, delete_forecast, delete_job, delete_model_snapshot, delete_trained_model,
    delete_trained_model_alias, estimate_model_memory, evaluate_data_frame,
    explain_data_frame_analytics, flush_job, forecast, get_buckets, get_calendar_events,
    get_calendars, get_categories, get_data_frame_analytics, get_data_frame_analytics_stats,
    get_datafeed_stats, get_datafeeds, get_filters, get_influencers, get_job_stats, get_jobs,
    get_memory_stats, get_model_snapshot_upgrade_stats, get_model_snapshots, get_overall_buckets,
    get_records, get_trained_models, get_trained_models_stats, infer_trained_model, info, open_job,
    post_calendar_events, post_data, preview_data_frame_analytics, preview_datafeed, put_calendar,
    put_calendar_job, put_data_frame_analytics, put_datafeed, put_filter, put_job,
    put_trained_model, put_trained_model_alias, put_trained_model_definition_part,
    put_trained_model_vocabulary, reset_job, revert_model_snapshot, set_upgrade_mode,
    start_data_frame_analytics, start_datafeed, start_trained_model_deployment,
    stop_data_frame_analytics, stop_datafeed, stop_trained_model_deployment,
    update_data_frame_analytics, update_datafeed, update_filter, update_job, update_model_snapshot,
    update_trained_model_deployment, upgrade_job_snapshot, validate, validate_detector,
};
use oxsearch::{Endpoint, Error, inference};
use oxsearch_docs::index;
use oxsearch_integration::{License, TestClient, TestIndex, client, deployed, understood};
use serde_json::{Value, json};

/// Held by each test while it runs: machine learning's jobs, analyses and
/// deployments share the node's memory for them, and on a CI runner tests
/// running at once leave a job no room to open (429).
static ML: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The start of the hour the test data begins at, in milliseconds.
const START: i64 = 1_700_000_000_000 - 1_700_000_000_000 % 3_600_000;
const HOUR: i64 = 3_600_000;

/// Whether the test goes on: on a trial, the call succeeded; on Basic,
/// machine learning is refused.
fn licensed<T: Debug>(result: &Result<T, Error>) -> bool {
    match License::from_env() {
        License::Trial => {
            assert!(result.is_ok(), "{result:?}");
            true
        }
        License::Basic => {
            assert!(matches!(result, Err(Error::License(_))), "{result:?}");
            false
        }
    }
}

/// An ID for a job, datafeed, calendar or model that no other test uses.
fn id(prefix: &str) -> String {
    TestIndex::named(prefix).name().to_owned()
}

/// Sends a request, and again while machine learning's own indices are
/// being created: on a fresh cluster, the first calls, made at once by
/// several tests, can find those indices' shards not yet allocated, as a
/// 503 or inside a 500 ("error deleting prior documents").
async fn send<E: Endpoint>(client: &TestClient, request: &E) -> Result<E::Response, Error> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match client.send(request).await {
            Err(Error::Elasticsearch(e))
                if (e.status == 500 || e.status == 503)
                    && Instant::now() < deadline
                    && [
                        "no_shard_available_action_exception",
                        "unavailable_shards_exception",
                        "due to missing shards",
                    ]
                    .iter()
                    .any(|t| e.body.to_string().contains(t)) =>
            {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            other => return other,
        }
    }
}

/// Calls `check` until it gives a value, for up to three minutes.
async fn eventually<T, F, Fut>(what: &str, mut check: F) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        if let Some(value) = check().await {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "{what} didn't happen in 3 minutes"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Hourly buckets of a mean over `value`, influenced by `host`.
fn analysis() -> AnalysisConfig {
    AnalysisConfig::new(vec![
        Detector::new()
            .function("mean")
            .field_name("value")
            .detector_description("mean value"),
    ])
    .bucket_span("1h")
    .influencers(vec!["host".to_owned()])
}

/// The same without influencers, whose cardinality validating and
/// estimating a job would need to be told.
fn plain_analysis() -> AnalysisConfig {
    AnalysisConfig::new(vec![Detector::new().function("mean").field_name("value")])
        .bucket_span("1h")
}

fn data_description() -> DataDescription {
    DataDescription::new().time_field("timestamp")
}

/// A record every ten minutes for `hours` hours from `from`, with a spike
/// in the last hour.
fn records(from: i64, hours: i64) -> Vec<Value> {
    (0..hours * 6)
        .map(|n| {
            let value = if n >= (hours - 1) * 6 {
                900.0
            } else {
                10.0 + (n % 3) as f64
            };
            json!({
                "timestamp": from + n * HOUR / 6,
                "value": value,
                "host": if n % 2 == 0 { "a" } else { "b" },
            })
        })
        .collect()
}

async fn snapshot_ids(client: &TestClient, job: &str) -> Vec<String> {
    send(
        client,
        &get_model_snapshots::Request::new(job).sort("timestamp"),
    )
    .await
    .unwrap()
    .model_snapshots
    .into_iter()
    .filter_map(|s| s.snapshot_id)
    .collect()
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn cluster_wide() {
    let _ml = ML.lock().await;
    let client = client();
    let trial = License::from_env() == License::Trial;
    let read = send(&client, &info::Request::new()).await;
    match (trial, read) {
        (true, Ok(info)) => assert_eq!(info.upgrade_mode, Some(false), "{info:?}"),
        (false, Ok(_) | Err(Error::License(_))) => {}
        (_, other) => panic!("{other:?}"),
    }
    let memory = send(&client, &get_memory_stats::Request::new()).await;
    match (trial, memory) {
        (true, Ok(memory)) => assert!(!memory.nodes.is_empty(), "{memory:?}"),
        (false, Ok(_) | Err(Error::License(_))) => {}
        (_, other) => panic!("{other:?}"),
    }
    // Turning upgrade mode off when it's off changes nothing, and leaves the
    // other tests' jobs running.
    let upgrade = send(&client, &set_upgrade_mode::Request::new().enabled(false)).await;
    assert!(trial == upgrade.is_ok() || !trial, "{upgrade:?}");
    let expired = send(&client, &delete_expired_data::Request::new()).await;
    match (trial, expired) {
        (true, Ok(expired)) => assert_eq!(expired.deleted, Some(true)),
        (false, Ok(_) | Err(Error::License(_))) => {}
        (_, other) => panic!("{other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn anomaly_detection() {
    let _ml = ML.lock().await;
    let client = client();
    // Validating a detector needs no license; the rest does.
    send(
        &client,
        &validate_detector::Request::new().body(Detector::new().function("count")),
    )
    .await
    .unwrap();
    let job = id("ml-job");
    let created = send(
        &client,
        &put_job::Request::new(job.as_str()).body(
            put_job::RequestBody::new(analysis(), data_description())
                .description("oxsearch's anomaly detection test")
                .analysis_limits(oxsearch::ml::AnalysisLimits::new().model_memory_limit("16mb")),
        ),
    )
    .await;
    if !licensed(&created) {
        return;
    }
    let created = created.unwrap();
    assert_eq!(created.job_id.as_deref(), Some(job.as_str()));
    send(
        &client,
        &validate::Request::new().body(
            validate::RequestBody::new()
                .analysis_config(plain_analysis())
                .data_description(data_description()),
        ),
    )
    .await
    .unwrap();
    let estimate = send(
        &client,
        &estimate_model_memory::Request::new()
            .body(estimate_model_memory::RequestBody::new().analysis_config(plain_analysis())),
    )
    .await
    .unwrap();
    assert!(estimate.model_memory_estimate.is_some(), "{estimate:?}");
    let jobs = send(&client, &get_jobs::Request::new().job_id(job.as_str()))
        .await
        .unwrap();
    assert_eq!(jobs.jobs.len(), 1);
    let updated = send(
        &client,
        &update_job::Request::new(job.as_str()).body(
            update_job::RequestBody::new()
                .description("updated")
                .groups(vec!["oxsearch".to_owned()])
                .analysis_limits(AnalysisMemoryLimit::new("16mb")),
        ),
    )
    .await
    .unwrap();
    assert_eq!(updated.description.as_deref(), Some("updated"));

    // Two days of data, then a flush, a forecast and the job's results.
    let opened = send(&client, &open_job::Request::new(job.as_str()))
        .await
        .unwrap();
    assert_eq!(opened.opened, Some(true));
    let posted = send(
        &client,
        &post_data::Request::new(job.as_str()).body(records(START, 48)),
    )
    .await
    .unwrap();
    assert_eq!(posted.processed_record_count, Some(48 * 6));
    let flushed = send(
        &client,
        &flush_job::Request::new(job.as_str()).calc_interim(true),
    )
    .await
    .unwrap();
    assert_eq!(flushed.flushed, Some(true));
    let forecast = send(
        &client,
        &forecast::Request::new(job.as_str()).duration("2h"),
    )
    .await
    .unwrap();
    let forecast_id = forecast.forecast_id.expect("a forecast ID");
    let stats = send(&client, &get_job_stats::Request::new().job_id(job.as_str()))
        .await
        .unwrap();
    assert_eq!(stats.jobs[0].state, Some(JobState::Opened));
    let closed = send(&client, &close_job::Request::new(job.as_str()))
        .await
        .unwrap();
    assert_eq!(closed.closed, Some(true));

    let buckets = send(&client, &get_buckets::Request::new(job.as_str()))
        .await
        .unwrap();
    assert!(!buckets.buckets.is_empty(), "{buckets:?}");
    let anomalies = send(
        &client,
        &get_records::Request::new(job.as_str())
            .desc(true)
            .sort("record_score"),
    )
    .await
    .unwrap();
    assert_eq!(anomalies.count, Some(anomalies.records.len() as i64));
    send(&client, &get_influencers::Request::new(job.as_str()))
        .await
        .unwrap();
    let overall = send(
        &client,
        &get_overall_buckets::Request::new(job.as_str()).top_n(1),
    )
    .await
    .unwrap();
    assert!(!overall.overall_buckets.is_empty(), "{overall:?}");

    // A second run leaves a second model snapshot.
    let first = snapshot_ids(&client, &job).await;
    assert!(!first.is_empty());
    send(&client, &open_job::Request::new(job.as_str()))
        .await
        .unwrap();
    send(
        &client,
        &post_data::Request::new(job.as_str()).body(records(START + 48 * HOUR, 24)),
    )
    .await
    .unwrap();
    send(&client, &close_job::Request::new(job.as_str()))
        .await
        .unwrap();
    let snapshots = snapshot_ids(&client, &job).await;
    let (oldest, newest) = (snapshots[0].clone(), snapshots[snapshots.len() - 1].clone());
    let kept = send(
        &client,
        &update_model_snapshot::Request::new(job.as_str(), oldest.as_str()).body(
            update_model_snapshot::RequestBody::new()
                .description("the first run")
                .retain(true),
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        kept.model.and_then(|m| m.description).as_deref(),
        Some("the first run")
    );
    send(
        &client,
        &get_model_snapshot_upgrade_stats::Request::new(job.as_str(), "_all"),
    )
    .await
    .unwrap();
    // The snapshot is already the current version, so there's nothing to
    // upgrade: Elasticsearch read the request, and says so.
    match send(
        &client,
        &upgrade_job_snapshot::Request::new(job.as_str(), oldest.as_str())
            .wait_for_completion(true),
    )
    .await
    {
        Ok(_) => {}
        Err(e) => assert!(understood(&e), "{e}"),
    }
    let reverted = send(
        &client,
        &revert_model_snapshot::Request::new(job.as_str(), oldest.as_str())
            .delete_intervening_results(true),
    )
    .await
    .unwrap();
    assert_eq!(
        reverted.model.and_then(|m| m.snapshot_id).as_deref(),
        Some(oldest.as_str())
    );
    // The newest snapshot, if the job wrote two. With one, it's the active
    // snapshot, which Elasticsearch reads the request for and won't delete.
    let deleted = send(
        &client,
        &delete_model_snapshot::Request::new(job.as_str(), newest.as_str()),
    )
    .await;
    match deleted {
        Ok(_) => assert_ne!(newest, oldest, "deleted the active snapshot"),
        Err(e) if newest == oldest => assert!(understood(&e), "{e}"),
        Err(e) => panic!("{e}"),
    }

    // The forecast finished with the first run; delete it.
    let client_ref = &client;
    let (job_ref, forecast_ref) = (&job, &forecast_id);
    eventually("deleting the forecast", || async move {
        send(
            client_ref,
            &delete_forecast::Request::new(job_ref.as_str()).forecast_id(forecast_ref.as_str()),
        )
        .await
        .ok()
    })
    .await;

    send(
        &client,
        &reset_job::Request::new(job.as_str()).wait_for_completion(true),
    )
    .await
    .unwrap();
    send(
        &client,
        &delete_job::Request::new(job.as_str()).wait_for_completion(true),
    )
    .await
    .unwrap();
    let gone = send(&client, &get_jobs::Request::new().job_id(job.as_str())).await;
    assert!(
        matches!(gone, Err(Error::Elasticsearch(ref e)) if e.status == 404),
        "{gone:?}"
    );
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn categories() {
    let _ml = ML.lock().await;
    let client = client();
    let job = id("ml-categories");
    let created = send(
        &client,
        &put_job::Request::new(job.as_str()).body(put_job::RequestBody::new(
            AnalysisConfig::new(vec![
                Detector::new()
                    .function("count")
                    .by_field_name("mlcategory"),
            ])
            .bucket_span("1h")
            .categorization_field_name("message"),
            data_description(),
        )),
    )
    .await;
    if !licensed(&created) {
        return;
    }
    send(&client, &open_job::Request::new(job.as_str()))
        .await
        .unwrap();
    let messages = ["user logged in", "disk full on node", "user logged out"];
    let records: Vec<Value> = (0..120)
        .map(|n| {
            json!({
                "timestamp": START + n * HOUR / 10,
                "message": format!("{} {}", messages[n as usize % 3], n % 7),
            })
        })
        .collect();
    send(
        &client,
        &post_data::Request::new(job.as_str()).body(records),
    )
    .await
    .unwrap();
    send(&client, &close_job::Request::new(job.as_str()))
        .await
        .unwrap();
    let categories = send(&client, &get_categories::Request::new(job.as_str()))
        .await
        .unwrap();
    assert!(!categories.categories.is_empty(), "{categories:?}");
    send(&client, &delete_job::Request::new(job.as_str()).force(true))
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn datafeeds() {
    let _ml = ML.lock().await;
    let client = client();
    let job = id("ml-feed-job");
    let created = send(
        &client,
        &put_job::Request::new(job.as_str()).body(
            put_job::RequestBody::new(analysis(), data_description())
                .analysis_limits(oxsearch::ml::AnalysisLimits::new().model_memory_limit("16mb")),
        ),
    )
    .await;
    if !licensed(&created) {
        return;
    }
    let source = TestIndex::create(
        &client,
        "ml-feed",
        &serde_json::from_value(json!({"properties": {
            "timestamp": {"type": "date"},
            "value": {"type": "double"},
            "host": {"type": "keyword"},
        }}))
        .unwrap(),
    )
    .await;
    let documents = records(START, 24);
    let last = documents.len() - 1;
    for (n, document) in documents.into_iter().enumerate() {
        let request = index::Request::new(source.name()).body(document);
        let request = if n == last {
            request.refresh(Refresh::True)
        } else {
            request
        };
        send(&client, &request).await.unwrap();
    }

    let feed = format!("datafeed-{job}");
    let put = send(
        &client,
        &put_datafeed::Request::new(feed.as_str()).body(
            put_datafeed::RequestBody::new()
                .indices(source.name())
                .job_id(job.as_str())
                .scroll_size(100),
        ),
    )
    .await
    .unwrap();
    assert_eq!(put.indices, vec![source.name().to_owned()]);
    let feeds = send(
        &client,
        &get_datafeeds::Request::new().datafeed_id(feed.as_str()),
    )
    .await
    .unwrap();
    assert_eq!(feeds.datafeeds.len(), 1);
    let preview = send(
        &client,
        &preview_datafeed::Request::new().datafeed_id(feed.as_str()),
    )
    .await
    .unwrap();
    assert!(!preview.is_empty(), "{preview:?}");
    let updated = send(
        &client,
        &update_datafeed::Request::new(feed.as_str())
            .body(update_datafeed::RequestBody::new().scroll_size(500)),
    )
    .await
    .unwrap();
    assert_eq!(updated.scroll_size, Some(500));

    // A look back over the index, which stops when it reaches the end.
    send(&client, &open_job::Request::new(job.as_str()))
        .await
        .unwrap();
    let started = send(
        &client,
        &start_datafeed::Request::new(feed.as_str())
            .start(START)
            .end(START + 24 * HOUR),
    )
    .await
    .unwrap();
    assert_eq!(started.started, Some(true));
    let client_ref = &client;
    let feed_ref = &feed;
    eventually("the look back", || async move {
        let stats = send(
            client_ref,
            &get_datafeed_stats::Request::new().datafeed_id(feed_ref.as_str()),
        )
        .await
        .unwrap();
        (stats.datafeeds[0].state == Some(DatafeedState::Stopped)).then_some(())
    })
    .await;
    let stats = send(&client, &get_job_stats::Request::new().job_id(job.as_str()))
        .await
        .unwrap();
    assert_eq!(
        stats.jobs[0]
            .data_counts
            .as_ref()
            .and_then(|c| c.processed_record_count),
        Some(24 * 6)
    );
    // Stopping a stopped datafeed is no error.
    let stopped = send(
        &client,
        &stop_datafeed::Request::new(feed.as_str()).close_job(true),
    )
    .await
    .unwrap();
    assert_eq!(stopped.stopped, Some(true));
    send(&client, &close_job::Request::new(job.as_str()))
        .await
        .unwrap();
    send(&client, &delete_datafeed::Request::new(feed.as_str()))
        .await
        .unwrap();
    send(&client, &delete_job::Request::new(job.as_str()))
        .await
        .unwrap();
    source.delete(&client).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn calendars_and_filters() {
    let _ml = ML.lock().await;
    let client = client();
    let filter = id("ml-filter");
    let put = send(
        &client,
        &put_filter::Request::new(filter.as_str()).body(
            put_filter::RequestBody::new()
                .description("hosts to skip")
                .items(vec!["a".to_owned(), "b".to_owned()]),
        ),
    )
    .await;
    // Filters need no license; jobs and calendars do.
    assert!(put.is_ok(), "{put:?}");
    let filters = send(
        &client,
        &get_filters::Request::new().filter_id(filter.as_str()),
    )
    .await
    .unwrap();
    assert_eq!(filters.filters[0].items, ["a", "b"]);
    // An update right after the put can miss the items it put ("not
    // present in filter"), until the filter's index refreshes.
    let update = update_filter::Request::new(filter.as_str()).body(
        update_filter::RequestBody::new()
            .add_items(vec!["c".to_owned()])
            .remove_items(vec!["a".to_owned()]),
    );
    let client_ref = &client;
    let update_ref = &update;
    let updated = eventually("the filter's update", || async move {
        match send(client_ref, update_ref).await {
            Ok(updated) => Some(updated),
            Err(Error::Elasticsearch(e)) if e.to_string().contains("not present in filter") => None,
            Err(e) => panic!("{e}"),
        }
    })
    .await;
    assert_eq!(updated.items, ["b", "c"]);
    send(&client, &delete_filter::Request::new(filter.as_str()))
        .await
        .unwrap();

    let job = id("ml-calendar-job");
    let created = send(
        &client,
        &put_job::Request::new(job.as_str())
            .body(put_job::RequestBody::new(analysis(), data_description())),
    )
    .await;
    if !licensed(&created) {
        return;
    }
    let (holidays, maintenance) = (id("ml-holidays"), id("ml-maintenance"));
    let calendar = send(
        &client,
        &put_calendar::Request::new(holidays.as_str()).body(
            put_calendar::RequestBody::new()
                .job_ids(vec![job.clone()])
                .description("public holidays"),
        ),
    )
    .await
    .unwrap();
    assert_eq!(calendar.description.as_deref(), Some("public holidays"));
    send(&client, &put_calendar::Request::new(maintenance.as_str()))
        .await
        .unwrap();
    let joined = send(
        &client,
        &put_calendar_job::Request::new(maintenance.as_str(), job.as_str()),
    )
    .await
    .unwrap();
    assert!(joined.job_ids.is_some(), "{joined:?}");
    send(
        &client,
        &delete_calendar_job::Request::new(maintenance.as_str(), job.as_str()),
    )
    .await
    .unwrap();
    let calendars = send(
        &client,
        &get_calendars::Request::new().calendar_id(holidays.as_str()),
    )
    .await
    .unwrap();
    assert_eq!(calendars.calendars.len(), 1);

    let posted = send(
        &client,
        &post_calendar_events::Request::new(holidays.as_str()).body(
            post_calendar_events::RequestBody::new(vec![CalendarEvent::new(
                "new year",
                START + 24 * HOUR,
                START,
            )]),
        ),
    )
    .await
    .unwrap();
    assert_eq!(posted.events.len(), 1);
    let events = send(
        &client,
        &get_calendar_events::Request::new(holidays.as_str()),
    )
    .await
    .unwrap();
    let event = events.events[0].event_id.clone().expect("an event ID");
    send(
        &client,
        &delete_calendar_event::Request::new(holidays.as_str(), event.as_str()),
    )
    .await
    .unwrap();
    for calendar in [&holidays, &maintenance] {
        send(&client, &delete_calendar::Request::new(calendar.as_str()))
            .await
            .unwrap();
    }
    send(&client, &delete_job::Request::new(job.as_str()))
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn data_frame_analytics() {
    let _ml = ML.lock().await;
    let client = client();
    let analytics = id("ml-analytics");
    let source_name = TestIndex::named("ml-points").name().to_owned();
    let dest = TestIndex::named("ml-outliers");
    let source = || DataframeAnalyticsSource::new(source_name.as_str());
    let outliers = || {
        DataframeAnalysisContainer::OutlierDetection(
            DataframeAnalysisOutlierDetection::new().compute_feature_influence(true),
        )
    };
    let explained = send(
        &client,
        &explain_data_frame_analytics::Request::new().body(
            explain_data_frame_analytics::RequestBody::new()
                .source(source())
                .analysis(outliers()),
        ),
    )
    .await;
    // On Basic the license check comes first; on a trial, the source index
    // doesn't exist yet.
    if License::from_env() == License::Basic {
        assert!(matches!(explained, Err(Error::License(_))), "{explained:?}");
        return;
    }
    assert!(
        matches!(explained, Err(ref e) if understood(e)),
        "{explained:?}"
    );

    // Twenty points near (1, 1), and two far from them.
    let points = TestIndex::create(
        &client,
        "ml-points",
        &serde_json::from_value(json!({"properties": {
            "x": {"type": "double"},
            "y": {"type": "double"},
            "is_outlier": {"type": "integer"},
        }}))
        .unwrap(),
    )
    .await;
    let source_name = points.name().to_owned();
    let source = || DataframeAnalyticsSource::new(source_name.as_str());
    for n in 0..22 {
        let (x, y, outlier) = if n < 20 {
            (1.0 + (n % 5) as f64 * 0.1, 1.0 + (n % 4) as f64 * 0.1, 0)
        } else {
            (10.0 + n as f64, -10.0, 1)
        };
        let request = index::Request::new(points.name()).body(json!({
            "x": x, "y": y, "is_outlier": outlier,
        }));
        let request = if n == 21 {
            request.refresh(Refresh::True)
        } else {
            request
        };
        send(&client, &request).await.unwrap();
    }

    let explained = send(
        &client,
        &explain_data_frame_analytics::Request::new().body(
            explain_data_frame_analytics::RequestBody::new()
                .source(source())
                .analysis(outliers()),
        ),
    )
    .await
    .unwrap();
    assert!(!explained.field_selection.is_empty(), "{explained:?}");
    send(
        &client,
        &put_data_frame_analytics::Request::new(analytics.as_str()).body(
            put_data_frame_analytics::RequestBody::new(
                outliers(),
                DataframeAnalyticsDestination::new(dest.name()),
                source(),
            )
            .model_memory_limit("16mb"),
        ),
    )
    .await
    .unwrap();
    let read = send(
        &client,
        &get_data_frame_analytics::Request::new().id(analytics.as_str()),
    )
    .await
    .unwrap();
    assert_eq!(read.data_frame_analytics.len(), 1);
    let preview = send(
        &client,
        &preview_data_frame_analytics::Request::new().id(analytics.as_str()),
    )
    .await
    .unwrap();
    assert!(!preview.feature_values.is_empty(), "{preview:?}");
    let updated = send(
        &client,
        &update_data_frame_analytics::Request::new(analytics.as_str()).body(
            update_data_frame_analytics::RequestBody::new().description("outliers among points"),
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        updated.description.as_deref(),
        Some("outliers among points")
    );

    let started = send(
        &client,
        &start_data_frame_analytics::Request::new(analytics.as_str()),
    )
    .await
    .unwrap();
    assert_eq!(started.acknowledged, Some(true));
    let client_ref = &client;
    let analytics_ref = &analytics;
    eventually("the analysis", || async move {
        let stats = send(
            client_ref,
            &get_data_frame_analytics_stats::Request::new().id(analytics_ref.as_str()),
        )
        .await
        .unwrap();
        let state = stats.data_frame_analytics[0].state.clone();
        assert_ne!(state, Some(DataframeState::Failed), "{stats:?}");
        (state == Some(DataframeState::Stopped)).then_some(())
    })
    .await;
    let stopped = send(
        &client,
        &stop_data_frame_analytics::Request::new(analytics.as_str()),
    )
    .await
    .unwrap();
    assert_eq!(stopped.stopped, Some(true));

    let evaluated = send(
        &client,
        &evaluate_data_frame::Request::new().body(evaluate_data_frame::RequestBody::new(
            DataframeEvaluationContainer::OutlierDetection(
                DataframeEvaluationOutlierDetection::new("is_outlier", "ml.outlier_score"),
            ),
            dest.name(),
        )),
    )
    .await
    .unwrap();
    assert!(evaluated.outlier_detection.is_some(), "{evaluated:?}");

    send(
        &client,
        &delete_data_frame_analytics::Request::new(analytics.as_str()),
    )
    .await
    .unwrap();
    points.delete(&client).await;
    // The destination index is the analysis's; delete it by name.
    send(
        &client,
        &oxsearch::admin::indices::delete::Request::new(dest.name()),
    )
    .await
    .unwrap();
}

/// A regression tree on one feature: 1 below 0.5, 2 above.
fn tree() -> put_trained_model::RequestBody {
    serde_json::from_value(json!({
        "description": "a one-split tree",
        "input": {"field_names": ["x"]},
        "inference_config": {"regression": {}},
        "definition": {"trained_model": {"tree": {
            "feature_names": ["x"],
            "target_type": "regression",
            "tree_structure": [
                {"node_index": 0, "split_feature": 0, "threshold": 0.5,
                 "decision_type": "lte", "default_left": true,
                 "left_child": 1, "right_child": 2},
                {"node_index": 1, "leaf_value": 1.0},
                {"node_index": 2, "leaf_value": 2.0},
            ],
        }}},
    }))
    .unwrap()
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn trained_models() {
    let _ml = ML.lock().await;
    let client = client();
    let model = id("ml-tree");
    let put = send(
        &client,
        &put_trained_model::Request::new(model.as_str()).body(tree()),
    )
    .await;
    if !licensed(&put) {
        return;
    }
    let models = send(
        &client,
        &get_trained_models::Request::new().model_id(model.as_str()),
    )
    .await
    .unwrap();
    assert_eq!(models.trained_model_configs.len(), 1);
    let stats = send(
        &client,
        &get_trained_models_stats::Request::new().model_id(model.as_str()),
    )
    .await
    .unwrap();
    assert_eq!(stats.trained_model_stats.len(), 1);
    let inferred = send(
        &client,
        &infer_trained_model::Request::new(model.as_str()).body(
            infer_trained_model::RequestBody::new(vec![
                serde_json::from_value(json!({"x": 0.1})).unwrap(),
                serde_json::from_value(json!({"x": 0.9})).unwrap(),
            ]),
        ),
    )
    .await
    .unwrap();
    let predicted: Vec<Value> = inferred
        .inference_results
        .iter()
        .map(|r| serde_json::to_value(&r.predicted_value).unwrap())
        .collect();
    assert_eq!(predicted, [json!(1.0), json!(2.0)]);
    let alias = format!("{model}-alias");
    send(
        &client,
        &put_trained_model_alias::Request::new(alias.as_str(), model.as_str()),
    )
    .await
    .unwrap();
    send(
        &client,
        &delete_trained_model_alias::Request::new(alias.as_str(), model.as_str()),
    )
    .await
    .unwrap();
    send(&client, &delete_trained_model::Request::new(model.as_str()))
        .await
        .unwrap();

    // A PyTorch model's configuration, vocabulary and definition, uploaded
    // in parts. Elasticsearch only reads the definition when the model is
    // deployed, so a few bytes stand in for it here.
    let pytorch = id("ml-pytorch");
    send(
        &client,
        &put_trained_model::Request::new(pytorch.as_str()).body(
            serde_json::from_value::<put_trained_model::RequestBody>(json!({
                "model_type": "pytorch",
                "input": {"field_names": ["text_field"]},
                "inference_config": {"pass_through": {}},
            }))
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    send(
        &client,
        &put_trained_model_vocabulary::Request::new(pytorch.as_str()).body(
            put_trained_model_vocabulary::RequestBody::new(
                ["[PAD]", "[UNK]", "[CLS]", "[SEP]", "hello", "world"]
                    .map(str::to_owned)
                    .to_vec(),
            ),
        ),
    )
    .await
    .unwrap();
    // "not a model", in base64.
    send(
        &client,
        &put_trained_model_definition_part::Request::new(pytorch.as_str(), 0).body(
            put_trained_model_definition_part::RequestBody::new("bm90IGEgbW9kZWw=", 11, 1),
        ),
    )
    .await
    .unwrap();
    send(
        &client,
        &delete_trained_model::Request::new(pytorch.as_str()).force(true),
    )
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn deployments() {
    let _ml = ML.lock().await;
    let client = client();
    if License::from_env() != License::Trial {
        let started = send(
            &client,
            &start_trained_model_deployment::Request::new(".elser_model_2"),
        )
        .await;
        assert!(matches!(started, Err(Error::License(_))), "{started:?}");
        return;
    }
    // ELSER, through the default inference endpoint, which downloads and
    // deploys it on first use; the trained model under it is the one this
    // platform runs.
    let warm = inference::sparse_embedding::Request::new(".elser-2-elasticsearch")
        .body(inference::sparse_embedding::RequestBody::new("deploy me"));
    deployed(|| send(&client, &warm)).await;
    let stats = send(
        &client,
        &get_trained_models_stats::Request::new().model_id(".elser_model_2*"),
    )
    .await
    .unwrap();
    let model = stats
        .trained_model_stats
        .iter()
        .find(|s| s.deployment_stats.is_some())
        .and_then(|s| s.model_id.clone())
        .expect("a deployed ELSER model");

    // A deployment of our own of the same model, started, scaled, its cache
    // cleared, and stopped.
    let deployment = id("ml-deployment");
    let started = send(
        &client,
        &start_trained_model_deployment::Request::new(model.as_str())
            .deployment_id(deployment.as_str())
            .number_of_allocations(1)
            .threads_per_allocation(1)
            .wait_for(DeploymentAllocationState::Started)
            .timeout("5m"),
    )
    .await
    .unwrap();
    assert_eq!(
        started.assignment.and_then(|a| a.assignment_state),
        Some(DeploymentAssignmentState::Started)
    );
    let updated = send(
        &client,
        &update_trained_model_deployment::Request::new(deployment.as_str())
            .body(update_trained_model_deployment::RequestBody::new().number_of_allocations(1)),
    )
    .await
    .unwrap();
    assert!(updated.assignment.is_some(), "{updated:?}");
    let cleared = send(
        &client,
        &clear_trained_model_deployment_cache::Request::new(deployment.as_str()),
    )
    .await
    .unwrap();
    assert_eq!(cleared.cleared, Some(true));
    let stopped = send(
        &client,
        &stop_trained_model_deployment::Request::new(deployment.as_str()),
    )
    .await
    .unwrap();
    assert_eq!(stopped.stopped, Some(true));
}
