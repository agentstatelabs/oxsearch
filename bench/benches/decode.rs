//! Serde throughput for large responses: a search response of 10, 1,000 and
//! 10,000 hits, read into oxsearch's generated response type, into a
//! hand-written struct of just the hits (what the official client leaves
//! you to write), and into a `serde_json::Value` (what it gives you
//! without one). No network: the bytes are in memory.

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use oxsearch::search::search;
use oxsearch_bench::{Doc, MinimalSearch, search_response};
use serde_json::Value;

fn decode(c: &mut Criterion) {
    for hits in [10u64, 1_000, 10_000] {
        let bytes = search_response(hits);
        let mut group = c.benchmark_group(format!("read a search response of {hits} hits"));
        group.throughput(Throughput::Bytes(bytes.len() as u64));
        if hits == 10_000 {
            group.sample_size(20);
        }
        group.bench_function("oxsearch, typed response", |b| {
            b.iter(|| {
                let found: search::Response<Doc> = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(found.into_results().len() as u64, hits);
            })
        });
        group.bench_function("hand-written struct", |b| {
            b.iter(|| {
                let found: MinimalSearch<Doc> = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(found.hits.hits.len() as u64, hits);
            })
        });
        group.bench_function("serde_json::Value", |b| {
            b.iter(|| {
                let found: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(found["hits"]["hits"].as_array().unwrap().len() as u64, hits);
            })
        });
        group.finish();
    }
}

criterion_group!(benches, decode);
criterion_main!(benches);
