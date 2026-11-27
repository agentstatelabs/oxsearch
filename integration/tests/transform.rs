//! Transforms against Elasticsearch, on any license: a pivot that sums each
//! tag's sales into another index, previewed, run once, updated and deleted
//! with its destination; a continuous transform that keeps each tag's latest
//! sale, started, scheduled now, stopped and reset; the nodes' statistics;
//! the upgrade check; and upgrade mode turned on and off.

use std::time::{Duration, Instant};

use oxsearch::Method;
use oxsearch::Refresh;
use oxsearch::aggs::{Agg, Aggs, TermsAggregation};
use oxsearch::search::count;
use oxsearch::shapes::IndexMap;
use oxsearch::transform::{
    Destination, Latest, Pivot, PivotGroupByContainer, Source, SyncContainer, TimeSync,
    delete_transform, get_node_stats, get_transform, get_transform_stats, preview_transform,
    put_transform, reset_transform, schedule_now_transform, set_upgrade_mode, start_transform,
    stop_transform, update_transform, upgrade_transforms,
};
use oxsearch_docs::index;
use oxsearch_integration::{TestClient, TestIndex, client};
use serde_json::json;

/// Waits until the transform has finished a checkpoint and is in `state`,
/// and returns how many documents it processed.
async fn checkpointed(client: &TestClient, id: &str, state: &str) -> i64 {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let stats = client
            .send(&get_transform_stats::Request::new(id))
            .await
            .unwrap();
        let transform = &stats.transforms[0];
        let checkpoint = transform
            .checkpointing
            .as_ref()
            .and_then(|c| c.last.as_ref())
            .and_then(|l| l.checkpoint)
            .unwrap_or_default();
        if checkpoint >= 1 && transform.state.as_deref() == Some(state) {
            return transform
                .stats
                .as_ref()
                .and_then(|s| s.documents_processed)
                .unwrap_or_default();
        }
        assert!(
            Instant::now() < deadline,
            "{id} didn't checkpoint: {transform:?}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Sums each tag's sales.
fn pivot() -> Pivot {
    let mut group_by = IndexMap::new();
    group_by.insert(
        "tag".to_owned(),
        PivotGroupByContainer::from(TermsAggregation::new().field("tag")),
    );
    Pivot::new()
        .group_by(group_by)
        .aggregations(Aggs::new().add("total", Agg::sum("price")).0)
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn transforms() {
    let client = client();
    let sales = TestIndex::named("transform");
    client
        .send_raw(
            Method::Put,
            &format!("/{}", sales.name()),
            Some(&json!({"mappings": {"properties": {
                "tag": {"type": "keyword"},
                "price": {"type": "double"},
                "@timestamp": {"type": "date"},
            }}})),
        )
        .await
        .unwrap();
    for (n, (tag, price)) in [("a", 1.0), ("a", 2.0), ("b", 4.0), ("b", 8.0), ("b", 16.0)]
        .into_iter()
        .enumerate()
    {
        client
            .send(
                &index::Request::new(sales.name())
                    .refresh(Refresh::True)
                    .body(json!({
                        "tag": tag,
                        "price": price,
                        "@timestamp": format!("2026-11-01T00:00:0{n}Z"),
                    })),
            )
            .await
            .unwrap();
    }

    // The pivot, previewed: a row a tag.
    let preview = client
        .send(
            &preview_transform::Request::new().body(
                preview_transform::RequestBody::new()
                    .source(Source::new(sales.name()))
                    .pivot(pivot()),
            ),
        )
        .await
        .unwrap();
    assert_eq!(preview.preview.len(), 2, "{preview:?}");
    assert!(
        preview
            .preview
            .contains(&json!({"tag": "b", "total": 28.0})),
        "{preview:?}"
    );

    // Created, read, updated, and run once into its destination.
    let id = format!("{}-pivot", sales.name());
    let totals = format!("{}-totals", sales.name());
    client
        .send(
            &put_transform::Request::new(id.as_str()).body(
                put_transform::RequestBody::new(
                    Destination::new().index(totals.as_str()),
                    Source::new(sales.name()),
                )
                .pivot(pivot())
                .description("each tag's sales"),
            ),
        )
        .await
        .unwrap();
    let read = client
        .send(&get_transform::Request::new().transform_id(id.as_str()))
        .await
        .unwrap();
    assert_eq!(read.transforms[0].id.as_deref(), Some(id.as_str()));
    let updated = client
        .send(
            &update_transform::Request::new(id.as_str())
                .body(update_transform::RequestBody::new().description("each tag's sales, summed")),
        )
        .await
        .unwrap();
    assert_eq!(
        updated.description.as_deref(),
        Some("each tag's sales, summed")
    );
    client
        .send(&start_transform::Request::new(id.as_str()))
        .await
        .unwrap();
    assert_eq!(checkpointed(&client, &id, "stopped").await, 5);
    let rows = client
        .send(&count::Request::new().index(totals.as_str()))
        .await
        .unwrap();
    assert_eq!(rows.count, Some(2));
    client
        .send(&delete_transform::Request::new(id.as_str()).delete_dest_index(true))
        .await
        .unwrap();

    // Each tag's latest sale, kept up to date: started, scheduled now,
    // stopped and reset.
    let id = format!("{}-latest", sales.name());
    let latest = format!("{}-latest", sales.name());
    client
        .send(
            &put_transform::Request::new(id.as_str()).body(
                put_transform::RequestBody::new(
                    Destination::new().index(latest.as_str()),
                    Source::new(sales.name()),
                )
                .latest(Latest::new("@timestamp", vec!["tag".to_owned()]))
                .sync(SyncContainer::from(TimeSync::new("@timestamp").delay("1s")))
                .frequency("1s"),
            ),
        )
        .await
        .unwrap();
    client
        .send(&start_transform::Request::new(id.as_str()))
        .await
        .unwrap();
    client
        .send(&schedule_now_transform::Request::new(id.as_str()))
        .await
        .unwrap();
    checkpointed(&client, &id, "started").await;
    let rows = client
        .send(&count::Request::new().index(latest.as_str()))
        .await
        .unwrap();
    assert_eq!(rows.count, Some(2));
    client
        .send(
            &stop_transform::Request::new(id.as_str())
                .wait_for_completion(true)
                .timeout("30s"),
        )
        .await
        .unwrap();
    client
        .send(&reset_transform::Request::new(id.as_str()))
        .await
        .unwrap();
    client
        .send(&delete_transform::Request::new(id.as_str()))
        .await
        .unwrap();

    // The nodes' statistics and the upgrade check.
    let nodes = client.send(&get_node_stats::Request::new()).await.unwrap();
    assert!(nodes.total.is_some(), "{nodes:?}");
    let upgrade = client
        .send(&upgrade_transforms::Request::new().dry_run(true))
        .await
        .unwrap();
    assert!(upgrade.needs_update.is_some(), "{upgrade:?}");

    // Upgrade mode, which pauses every transform, on and off.
    for enabled in [true, false] {
        client
            .send(&set_upgrade_mode::Request::new().enabled(enabled))
            .await
            .unwrap();
    }

    for index in [latest.as_str(), sales.name()] {
        client
            .send_raw(
                Method::Delete,
                &format!("/{index}?ignore_unavailable=true"),
                None,
            )
            .await
            .unwrap();
    }
}
