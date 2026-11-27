# Benchmarks: oxsearch against the official elasticsearch crate

_Measured 2026-11-26, against `elasticsearch` 9.1.0-alpha.1, the latest release of Elastic's Rust client._

## What's compared

- **Request overhead:** a search for 10 hits and the indexing of one document, through each client, over HTTP to a local server that answers at once with the same canned response. What's measured is each client's own work: building the request, the HTTP exchange through reqwest, and reading the response.
- **Serde throughput:** search responses of 10, 1,000 and 10,000 hits, about 470 bytes of `_source` each, read from memory.
- **Bulk throughput:** both clients load the same 200,000 documents into Elasticsearch.

Each benchmark reads a response the ways you'd read it with each client:
- **oxsearch's typed response** is its generated `search::Response<Doc>`: every field Elasticsearch sends, typed, including the total, the scores and the shard counts.
- **A hand-written struct** holds only the hits' ids and sources. The official client hands you the body and leaves the types to you, so this is the least work you can do with it. oxsearch reads into the same struct through `send_as`.
- **`serde_json::Value`** is what you read without types.

## Environment

- **Request overhead and serde throughput:** Apple M5 Max, 36 GiB RAM, macOS 27.2.1, rustc 1.99.1, release builds, criterion 0.8. It's a laptop with other work running, so absolute numbers move a little between runs; the comparisons held across the runs made.
- **Bulk throughput:** a GitHub-hosted `ubuntu-latest` runner (Ubuntu 26.04.1, image 20261122.9.1, rustc 1.100.0), with Elasticsearch 9.5.7 in Docker on the same machine (`scripts/es-fixture.sh`, one node, Basic license), from the `Benchmarks` workflow at commit `a4b69ca`. The same run repeated the criterion benchmarks: [On the CI runner](#on-the-ci-runner) below.

The clients use their own HTTP stacks: reqwest 0.13 with rustls for oxsearch, and reqwest 0.12 with rustls for the official client, both over hyper 1, HTTP/1.1 with keep-alive.

## Making it fair

- **Same responses.** In the overhead benchmark, one server answers every request with the same canned body and the `x-elastic-product` header, which the official client checks for. It reads each request whole before answering.
- **Same work.** Every iteration asserts the number of hits or the document's id, so neither client can skip reading the response.
- **Typed against hand-written.** The overhead benchmark reads oxsearch's complete typed response, and also reads the same hand-written struct through oxsearch. That separates the cost of the request path from the cost of reading everything.
- **Bulk loads are counted.** Each run starts from a new index (one shard, no replicas, no refresh while loading). After each run the index is refreshed and counted, and the run fails unless every document is there. Both clients send batches of 1,000 documents with 4 requests in flight. Each loads once, untimed, while the cluster warms up; then they take turns over 5 runs, swapping who goes first.

## Request overhead

Median from criterion, with its 95% confidence interval.

| Request | oxsearch, typed response | oxsearch, hand-written struct | official, hand-written struct | official, `Value` |
|---|---|---|---|---|
| Search, 10 hits | 46.5 µs (46.3–46.7) | **41.2 µs** (41.0–41.3) | **40.7 µs** (40.5–40.8) | 47.0 µs (46.8–47.3) |
| Index one document | 34.6 µs (34.5–34.8) | | **34.0 µs** (33.8–34.2) | |

- **The request path is close to level.** Reading the same struct, oxsearch is within about 1% of the official client on a search here, and about 4% on the CI runner; on a write, about 2% on both.
- **Reading the complete typed response costs about 5 µs** per 10-hit search over a hand-written struct. That's what reading every field costs, with no struct to write. `Value` costs about the same and gives you no types.
- Measured alone, reading a 10-hit response takes 6.6 µs into the typed response and 4.7 µs into the hand-written struct, a 1.9 µs difference. Inside a request the difference is larger, and isn't broken down further here.

## Serde throughput

| Response | oxsearch, typed response | hand-written struct | `serde_json::Value` |
|---|---|---|---|
| 10 hits (about 5 KB) | 6.6 µs | **4.7 µs** | 8.5 µs |
| 1,000 hits (about 540 KB) | 978 MiB/s (527 µs) | **1.08 GiB/s** (465 µs) | 621 MiB/s (829 µs) |
| 10,000 hits (about 5.4 MB) | 877 MiB/s (5.91 ms) | **1.08 GiB/s** (4.67 ms) | 602 MiB/s (8.60 ms) |

- **The typed response reads 1.5–1.6× faster than `Value`**, and 12–20% slower than a struct of only the hits.
- The hand-written struct reads less: it skips the score, the index and everything outside the hits. With oxsearch you can still read into it, through `send_as`, when every microsecond counts.

## Bulk throughput

200,000 documents (95 MB of JSON), batches of 1,000, 4 requests in flight, median of 5 runs after a warm-up.

| Client | Documents/s | MB/s | Median | Range over the runs |
|---|---:|---:|---:|---|
| oxsearch `BulkLoader` | 115,393 | 54.9 | 1.73 s | 1.68–2.01 s |
| official `elasticsearch` | 115,872 | 55.1 | 1.73 s | 1.67–1.80 s |

- **Level.** The medians are the same to the hundredth of a second, and the runs overlap. That suggests neither client is the bottleneck at these batch sizes and with this many requests in flight; both wait on the same single-node cluster.
- **Warm up first.** In an earlier run without the warm-up, the first load (oxsearch's) took 5.9 s, against 2.6–3.6 s for the loads after it. The warm-up keeps the cluster's first load out of the numbers.
- `BulkLoader` also does what the official client leaves to you: batching by count and size, backpressure, backoff on 429s, and an outcome per document. Here it does that without costing throughput.

## On the CI runner

The same workflow ran the criterion benchmarks on the runner. Its numbers are slower than the laptop's and noisier, but the order is the same.

| Benchmark | oxsearch, typed | oxsearch, hand-written | official, hand-written | `Value` |
|---|---|---|---|---|
| Search, 10 hits | 59.9 µs | 54.0 µs | **52.0 µs** | 64.6 µs (official), 66.4 µs (oxsearch's `send_raw`) |
| Index one document | 39.4 µs | | **38.6 µs** | |
| Read 1,000 hits | 527 MiB/s | **618 MiB/s** | | 339 MiB/s |
| Read 10,000 hits | 522 MiB/s | **630 MiB/s** | | 318 MiB/s |

## Running them

```bash
cd bench
cargo bench --bench overhead --bench decode
scripts/es-fixture.sh up basic   # from the repository's root
OXSEARCH_BENCH_URL=http://127.0.0.1:9200 cargo run --release --bin bulk
```

`OXSEARCH_BENCH_DOCS`, `OXSEARCH_BENCH_BATCH`, `OXSEARCH_BENCH_CONCURRENCY` and `OXSEARCH_BENCH_RUNS` change the bulk load. The `Benchmarks` workflow runs all three on pull requests that touch `bench/`, and by hand, and writes the numbers to its summary.
