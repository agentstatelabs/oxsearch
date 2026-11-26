//! Bulk throughput: each client loads the same documents into Elasticsearch
//! with the same batch size and the same number of requests in flight,
//! timed from the first document to the last batch's answer. Each run
//! starts from a new index, and afterwards counts it, so a client that
//! dropped documents can't look fast.
//!
//! ```text
//! OXSEARCH_BENCH_URL=http://127.0.0.1:9200 cargo run --release --bin bulk
//! ```
//!
//! `OXSEARCH_BENCH_DOCS` (200,000), `OXSEARCH_BENCH_BATCH` (1,000),
//! `OXSEARCH_BENCH_CONCURRENCY` (4) and `OXSEARCH_BENCH_RUNS` (3) change the
//! defaults. The clients take turns, and swap who goes first each run.

use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};

use elasticsearch::http::transport::Transport;
use elasticsearch::{BulkOperation, BulkParts, Elasticsearch};
use oxsearch::Method;
use oxsearch::helpers::{Action, BulkLoader};
use oxsearch::transport_reqwest::ReqwestTransport;
use oxsearch_bench::{Doc, doc};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

const INDEX: &str = "oxsearch-bench-bulk";

fn setting(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.replace('_', "").parse().ok())
        .unwrap_or(default)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Contender {
    Oxsearch,
    Official,
}

struct Load {
    docs: usize,
    batch: usize,
    concurrency: usize,
}

type Ours = oxsearch::Client<ReqwestTransport>;

async fn fresh_index(ours: &Ours) -> Result<(), Box<dyn Error + Send + Sync>> {
    let _ = ours
        .send_raw(
            Method::Delete,
            &format!("/{INDEX}?ignore_unavailable=true"),
            None,
        )
        .await;
    ours.send_raw(
        Method::Put,
        &format!("/{INDEX}"),
        Some(&json!({"settings": {
            "number_of_shards": 1,
            "number_of_replicas": 0,
            "refresh_interval": "-1",
        }})),
    )
    .await?;
    Ok(())
}

async fn count(ours: &Ours) -> Result<u64, Box<dyn Error + Send + Sync>> {
    ours.send_raw(Method::Post, &format!("/{INDEX}/_refresh"), None)
        .await?;
    let counted = ours
        .send_raw(Method::Get, &format!("/{INDEX}/_count"), None)
        .await?;
    Ok(counted["count"].as_u64().unwrap_or_default())
}

async fn with_oxsearch(ours: &Ours, load: &Load) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (sender, results) = BulkLoader::new(ours)
        .index(INDEX)
        .max_actions(load.batch)
        .max_bytes(usize::MAX)
        .flush_interval(Duration::from_secs(60))
        .concurrency(load.concurrency)
        .start();
    let docs = load.docs as u64;
    let feed = tokio::spawn(async move {
        for n in 0..docs {
            sender
                .send(Action::index(doc(n)).id(n.to_string()))
                .await
                .map_err(|e| e.to_string())?;
        }
        Ok::<_, String>(())
    });
    let summary = results.summary().await?;
    feed.await??;
    if !summary.failed.is_empty() {
        return Err(format!("oxsearch: {} documents refused", summary.failed.len()).into());
    }
    Ok(())
}

async fn with_official(
    theirs: &Elasticsearch,
    load: &Load,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let in_flight = Arc::new(Semaphore::new(load.concurrency));
    let mut batches = JoinSet::new();
    for start in (0..load.docs).step_by(load.batch) {
        let end = (start + load.batch).min(load.docs);
        let operations: Vec<BulkOperation<Doc>> = (start..end)
            .map(|n| BulkOperation::index(doc(n as u64)).id(n.to_string()).into())
            .collect();
        let permit = in_flight.clone().acquire_owned().await?;
        let theirs = theirs.clone();
        batches.spawn(async move {
            let _permit = permit;
            let response = theirs
                .bulk(BulkParts::Index(INDEX))
                .body(operations)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            let answer: Value = response.json().await.map_err(|e| e.to_string())?;
            if answer["errors"] != json!(false) {
                return Err(format!(
                    "official: a batch had errors: {}",
                    answer["items"][0]
                ));
            }
            Ok::<_, String>(())
        });
    }
    while let Some(batch) = batches.join_next().await {
        batch??;
    }
    Ok(())
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let url =
        std::env::var("OXSEARCH_BENCH_URL").unwrap_or_else(|_| "http://127.0.0.1:9200".to_owned());
    let load = Load {
        docs: setting("OXSEARCH_BENCH_DOCS", 200_000),
        batch: setting("OXSEARCH_BENCH_BATCH", 1_000),
        concurrency: setting("OXSEARCH_BENCH_CONCURRENCY", 4),
    };
    let runs = setting("OXSEARCH_BENCH_RUNS", 3).max(1);
    let ours = oxsearch::Client::new(ReqwestTransport::new(&url)?);
    let theirs = Elasticsearch::new(Transport::single_node(&url)?);
    let bytes: usize = (0..load.docs as u64)
        .map(|n| serde_json::to_vec(&doc(n)).map_or(0, |d| d.len()))
        .sum();
    let version = ours.send_raw(Method::Get, "/", None).await?["version"]["number"].clone();

    let mut seconds = [Vec::new(), Vec::new()];
    for run in 0..runs {
        let order = if run % 2 == 0 {
            [Contender::Oxsearch, Contender::Official]
        } else {
            [Contender::Official, Contender::Oxsearch]
        };
        for contender in order {
            fresh_index(&ours).await?;
            let started = Instant::now();
            match contender {
                Contender::Oxsearch => with_oxsearch(&ours, &load).await?,
                Contender::Official => with_official(&theirs, &load).await?,
            }
            let took = started.elapsed().as_secs_f64();
            let counted = count(&ours).await?;
            if counted != load.docs as u64 {
                return Err(
                    format!("{contender:?} left {counted} of {} documents", load.docs).into(),
                );
            }
            eprintln!("run {}: {contender:?} {took:.2} s", run + 1);
            seconds[contender as usize].push(took);
        }
    }
    let _ = ours
        .send_raw(Method::Delete, &format!("/{INDEX}"), None)
        .await;

    println!(
        "Elasticsearch {version}: {} documents ({:.1} MB of JSON), batches of {}, {} in flight, median of {runs} runs.\n",
        load.docs,
        bytes as f64 / 1e6,
        load.batch,
        load.concurrency
    );
    println!("| Client | Documents/s | MB/s | Seconds |\n|---|---:|---:|---:|");
    for (contender, name) in [
        (Contender::Oxsearch, "oxsearch `BulkLoader`"),
        (Contender::Official, "official `elasticsearch`"),
    ] {
        let took = median(seconds[contender as usize].clone());
        println!(
            "| {name} | {:.0} | {:.1} | {took:.2} |",
            load.docs as f64 / took,
            bytes as f64 / 1e6 / took
        );
    }
    Ok(())
}
