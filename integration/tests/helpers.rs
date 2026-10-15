//! The bulk loader and the point-in-time scan against Elasticsearch: load a
//! few thousand documents, then read every one back, whole and in slices.

use std::collections::BTreeSet;
use std::time::Duration;

use oxsearch::helpers::{Action, BulkLoader, Scan};
use oxsearch::{Method, Query};
use oxsearch_integration::{TestIndex, client};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Serialize, Deserialize)]
struct Row {
    n: u32,
    even: bool,
}

const ROWS: u32 = 3000;

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn load_then_scan_everything() {
    let client = client();
    let test = TestIndex::named("helpers");
    client
        .send_raw(
            Method::Put,
            &format!("/{}", test.name()),
            Some(&json!({"mappings": {"properties": {"n": {"type": "integer"}, "even": {"type": "boolean"}}}})),
        )
        .await
        .unwrap();

    let (sender, results) = BulkLoader::new(&client)
        .index(test.name())
        .max_actions(500)
        .concurrency(3)
        .flush_interval(Duration::from_millis(200))
        .start();
    let feed = tokio::spawn(async move {
        for n in 0..ROWS {
            let row = Row {
                n,
                even: n % 2 == 0,
            };
            sender.send(Action::index(row).id(n.to_string())).await?;
        }
        Ok::<_, oxsearch::helpers::SendError>(())
    });
    let summary = results.summary().await.unwrap();
    feed.await.unwrap().unwrap();
    assert_eq!(summary.succeeded, u64::from(ROWS));
    assert!(summary.failed.is_empty(), "{:?}", summary.failed.first());
    client
        .send_raw(Method::Post, &format!("/{}/_refresh", test.name()), None)
        .await
        .unwrap();

    for slices in [1, 3] {
        let mut stream = std::pin::pin!(
            Scan::new(test.name())
                .page_size(400)
                .slices(slices)
                .stream::<Row, _>(&client)
        );
        let mut seen = BTreeSet::new();
        while let Some(hit) = futures_util::TryStreamExt::try_next(&mut stream)
            .await
            .unwrap()
        {
            let row = hit.source.expect("a hit has its source");
            assert!(seen.insert(row.n), "row {} read twice", row.n);
        }
        assert_eq!(seen.len(), ROWS as usize, "{slices} slice(s)");
    }

    // Only what a query matches.
    let mut stream = std::pin::pin!(
        Scan::new(test.name())
            .query(Query::term("even", true))
            .stream::<Row, _>(&client)
    );
    let mut count = 0;
    while let Some(hit) = futures_util::TryStreamExt::try_next(&mut stream)
        .await
        .unwrap()
    {
        assert!(hit.source.unwrap().even);
        count += 1;
    }
    assert_eq!(count, ROWS / 2);
    test.delete(&client).await;
}
