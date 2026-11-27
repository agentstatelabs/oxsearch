//! Rollups against Elasticsearch, on any license. Elasticsearch deprecates
//! them in favour of downsampling, and takes a new rollup job only in a
//! cluster that already uses rollups: the test shows a job refused without
//! one, then makes an index with a rollup index's metadata so the job is
//! taken: created, read, its capabilities, started, searched, stopped and
//! deleted.

use std::time::{Duration, Instant};

use oxsearch::aggs::{Agg, Aggs, Results};
use oxsearch::rollup::{
    DateHistogramGrouping, FieldMetric, Groupings, Metric, delete_job, get_jobs, get_rollup_caps,
    get_rollup_index_caps, put_job, rollup_search, start_job, stop_job,
};
use oxsearch::{Error, Method, Refresh};
use oxsearch_docs::index;
use oxsearch_integration::{TestIndex, client};
use serde_json::json;

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn jobs() {
    let client = client();
    let readings = TestIndex::named("rollup");
    client
        .send_raw(
            Method::Put,
            &format!("/{}", readings.name()),
            Some(&json!({"mappings": {"properties": {
                "@timestamp": {"type": "date"},
                "temp": {"type": "double"},
            }}})),
        )
        .await
        .unwrap();
    for (time, temp) in [
        ("00:00", 18.0),
        ("00:30", 20.0),
        ("01:00", 21.0),
        ("01:30", 25.0),
    ] {
        client
            .send(
                &index::Request::new(readings.name())
                    .refresh(Refresh::True)
                    .body(json!({"@timestamp": format!("2026-11-01T{time}:00Z"), "temp": temp})),
            )
            .await
            .unwrap();
    }

    // Every second, each hour's average and highest temperature.
    let job = format!("{}-hourly", readings.name());
    let rollups = format!("{}-hourly", readings.name());
    let put = put_job::Request::new(job.as_str()).body(
        put_job::RequestBody::new(
            "* * * * * ?",
            Groupings::new()
                .date_histogram(DateHistogramGrouping::new("@timestamp").fixed_interval("1h")),
            readings.name(),
            100,
            rollups.as_str(),
        )
        .metrics(vec![FieldMetric::new(
            "temp",
            vec![Metric::Avg, Metric::Max],
        )]),
    );

    // Refused while the cluster has no rollup index.
    match client.send(&put).await {
        Err(Error::Elasticsearch(e)) => {
            assert!(
                e.reason().is_some_and(|r| r.contains("rollup usage")),
                "{e}"
            );
        }
        other => panic!("a rollup job in a cluster without rollups: {other:?}"),
    }

    // Taken beside an index with a rollup index's metadata, which goes once
    // the job has made its own: the capabilities read every rollup index's,
    // and this one's are empty.
    let earlier = format!("{}-earlier", readings.name());
    client
        .send_raw(
            Method::Put,
            &format!("/{earlier}"),
            Some(
                &json!({"mappings": {"_meta": {"_rollup": {"earlier": {}}, "rollup-version": ""}}}),
            ),
        )
        .await
        .unwrap();
    client.send(&put).await.unwrap();
    client
        .send_raw(Method::Delete, &format!("/{earlier}"), None)
        .await
        .unwrap();

    let read = client
        .send(&get_jobs::Request::new().id(job.as_str()))
        .await
        .unwrap();
    let config = read.jobs[0]
        .config
        .as_ref()
        .expect("the job's configuration");
    assert_eq!(config.rollup_index.as_deref(), Some(rollups.as_str()));
    let caps = client
        .send(&get_rollup_caps::Request::new().id(readings.name()))
        .await
        .unwrap();
    assert!(caps.contains_key(readings.name()), "{caps:?}");
    let caps = client
        .send(&get_rollup_index_caps::Request::new(rollups.as_str()))
        .await
        .unwrap();
    assert!(caps.contains_key(rollups.as_str()), "{caps:?}");

    // Started, and searched once it has rolled up both hours.
    let started = client
        .send(&start_job::Request::new(job.as_str()))
        .await
        .unwrap();
    assert_eq!(started.started, Some(true));
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let read = client
            .send(&get_jobs::Request::new().id(job.as_str()))
            .await
            .unwrap();
        let stats = read.jobs[0].stats.as_ref();
        if stats.and_then(|s| s.rollups_indexed).unwrap_or_default() >= 2 {
            break;
        }
        assert!(Instant::now() < deadline, "nothing rolled up: {read:?}");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    client
        .send_raw(Method::Post, &format!("/{rollups}/_refresh"), None)
        .await
        .unwrap();
    let found = client
        .send(
            &rollup_search::Request::new(rollups.as_str())
                .typed_keys(true)
                .body(
                    rollup_search::RequestBody::new()
                        .size(0)
                        .aggregations(Aggs::new().add("highest", Agg::max("temp")).0),
                ),
        )
        .await
        .unwrap();
    let aggregations = found.aggregations.expect("the aggregations");
    assert_eq!(aggregations.value("highest"), Some(25.0));

    let stopped = client
        .send(
            &stop_job::Request::new(job.as_str())
                .wait_for_completion(true)
                .timeout("30s"),
        )
        .await
        .unwrap();
    assert_eq!(stopped.stopped, Some(true));
    let deleted = client
        .send(&delete_job::Request::new(job.as_str()))
        .await
        .unwrap();
    assert_eq!(deleted.acknowledged, Some(true));
    for index in [rollups.as_str(), readings.name()] {
        client
            .send_raw(Method::Delete, &format!("/{index}"), None)
            .await
            .unwrap();
    }
}
