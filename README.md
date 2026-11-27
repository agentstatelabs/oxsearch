# oxsearch

A typed Rust client for Elasticsearch, generated from the Elasticsearch specification, with typed AI search: retrievers, `semantic`, `knn` and `sparse_vector` queries, `semantic_text` mappings, the inference API, and a typed document layer.

**Status: 0.2.** The generated crates cover search, documents, mappings, analysis, inference, tasks, aggregations, ES|QL, administration, ingest pipelines, enrich policies, query rules, synonyms, search applications, async search, EQL, SQL, snapshots, index lifecycle management and the license API. On `main` for the next release: machine learning, Watcher, snapshot lifecycle management, X-Pack information and usage, the cat APIs, security, transforms, rollups, and the cluster operations: cross-cluster replication, searchable snapshots, dangling indices, autoscaling, node shutdown, migration, feature states, ingest simulation and text structure; and connectors, Fleet, Logstash, graph, profiling, serverless projects and streams. That's every endpoint the specification offers for code generation, 597 of them, tested against its own examples where it has them, and the integration tests send 485 of them to Elasticsearch 9, on the Basic license and a trial. See [DESIGN.md](DESIGN.md) for the plan and [CHANGELOG.md](CHANGELOG.md) for what's in each release.

## Quick start

Semantic search over your own documents, with `features = ["derive"]`. The default features include the HTTP transport, on reqwest with rustls; with a transport of your own, set `default-features = false`. `semantic_text` embeds with the cluster's default ELSER model, which needs a license above Basic, such as a trial; on Basic, the [RAG example](crates/oxsearch/examples/rag.rs) uses your own embeddings and fuses on the client instead.

```rust
use oxsearch::document::{Document, Indexes};
use oxsearch::transport_reqwest::ReqwestTransport;
use oxsearch::{Client, Refresh};
use serde::{Deserialize, Serialize};

#[derive(Document, Serialize, Deserialize, Debug)]
#[index("quickstart-passages")]
struct Passage {
    #[semantic_text]
    text: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = Client::new(ReqwestTransport::new("http://localhost:9200")?);
    let passages = client.index::<Passage>().refresh(Refresh::WaitFor);
    let _ = passages.ensure_mapping().await?;
    passages
        .put(&Passage {
            text: "HNSW graphs find nearest neighbours fast.".into(),
        })
        .await?;
    passages
        .put(&Passage {
            text: "Replicas keep copies of shards on other nodes.".into(),
        })
        .await?;
    for hit in passages
        .search(Passage::TEXT.semantic("how do vector indexes work?"))
        .await?
    {
        println!("{:.2} {:?}", hit.score.unwrap_or_default(), hit.source);
    }
    Ok(())
}
```

([`examples/quickstart.rs`](crates/oxsearch/examples/quickstart.rs); a test keeps the two the same.)

## What's in it

- **Generated types and endpoints** for every request and response in the areas generated so far, with builders, `Raw` variants for what a newer Elasticsearch adds, and typed errors (license refusals and version conflicts have their own).
- **Aggregations with typed results** (`oxsearch::aggs`, the `aggs` feature): `Agg::terms("category", 10).sub("avg_price", Agg::avg("price"))`, sent with `typed_keys` and read back as `Aggregates`.
- **ES|QL** (`oxsearch::esql`, the `esql` feature): a typed builder, `Esql::from_index("books").where_(col("year").gte(2020)).sort([col("year").desc()])`, with values sent as parameters, `FORK` and `FUSE`; and rows read into your own structs by column name with `.fetch(&client)`, or as Arrow record batches with `.fetch_arrow(&client)` (the `esql-arrow` feature).
- **Ingest pipelines** (`oxsearch::ingest`, the `ingest` feature): pipelines built from typed processors (`SetProcessor::new("indexed_by").value(…).into()`), simulated, or run as documents are indexed, including the inference processor that embeds text at index time.
- **Search relevance**: query rules that pin documents for matching queries (`oxsearch::query_rules`, the `query-rules` feature) and synonym sets that analyzers read and update without reindexing (`oxsearch::synonyms`, the `synonyms` feature).
- **More ways to query**: async search, EQL and SQL (`oxsearch::async_search`, `oxsearch::eql`, `oxsearch::sql`), search applications (`oxsearch::search_application`) and enrich policies (`oxsearch::enrich`), each behind the feature of its name.
- **Operations**: snapshots (`oxsearch::snapshot`, the `snapshot` feature), snapshot lifecycle policies that take them on a schedule (`oxsearch::slm`, the `slm` feature) and index lifecycle policies (`oxsearch::ilm`, the `ilm` feature).
- **Machine learning** (`oxsearch::ml`, the `ml` feature): anomaly detection jobs with their datafeeds, results, forecasts and model snapshots; calendars and filters; data frame analytics; and trained models, imported, inferred with and deployed.
- **Alerting** (`oxsearch::watcher`, the `watcher` feature): watches that run an input, such as a search, on a schedule, test a condition and act, put, run, acknowledged, activated and deactivated, and the Watcher service itself.
- **Security** (`oxsearch::security`, the `security` feature): native users, roles and role mappings, application privileges, API keys and cross-cluster API keys, tokens, service accounts and user profiles; and single sign-on through SAML, OpenID Connect and PKI.
- **Transforms** (`oxsearch::transform`, the `transform` feature): pivot and latest transforms that summarize one index into another, previewed, run once or continuously, scheduled now, stopped, reset and upgraded; and **rollups** (`oxsearch::rollup`, the `rollup` feature), which Elasticsearch deprecates in favour of downsampling and takes new jobs for only in a cluster that already has rollup indices.
- **Cluster operations**: cross-cluster replication (`oxsearch::ccr`), searchable snapshots (`oxsearch::searchable_snapshots`), dangling indices (`oxsearch::dangling_indices`), node shutdown and autoscaling (`oxsearch::shutdown`, `oxsearch::autoscaling`), deprecations and system feature upgrades (`oxsearch::migration`), and feature states (`oxsearch::features`), each behind the feature of its name with hyphens (`searchable-snapshots`); and, for data on its way in, ingest simulation against an index's pipelines and mapping (`oxsearch::simulate`) and the structure of log text (`oxsearch::text_structure`).
- **Elastic's other products**: content connectors and their sync jobs (`oxsearch::connector`), Fleet's checkpoints and searches (`oxsearch::fleet`), Logstash's centrally managed pipelines (`oxsearch::logstash`), graph exploration (`oxsearch::graph`), Universal Profiling (`oxsearch::profiling`), named streams (`oxsearch::streams`) and, on Elastic Cloud Serverless only, projects' tags and routing (`oxsearch::project`), each behind the feature of its name.
- **What the cluster offers** (`oxsearch::xpack`, the `xpack` feature): its build, its license and which features it makes available, and how much each is used.
- **The cat APIs** (`oxsearch::cat`, the `cat` feature): the compact tables of indices, nodes, shards, allocation, tasks, snapshots and the rest, read as typed rows from their JSON form.
- **Administration** (`oxsearch::admin`, the `admin` feature): indices, their mappings, settings, aliases, templates and data streams; the cluster's health, settings and state; and nodes' information and statistics.
- **Query and retriever builders**: `Query::r#match`, `Query::semantic`, `Retriever::knn`, `Retriever::rrf`, …; and **fusion on the client** (`oxsearch::fusion`) when the license doesn't include the `rrf` and `linear` retrievers.
- **A typed document layer** (`oxsearch::document`, the `document` feature, which `derive` implies): `#[derive(Document)]` maps a struct, `Index<T>` reads and writes it, changes part of it with a patch or a script, and sends bulk writes; `Versioned<T>` makes a write conditional on what was read; and field handles offer only the queries that suit each field.
- **Helpers** (`oxsearch::helpers`): a point-in-time scan and a bulk loader with backpressure.
- **A command line** (`oxsearch-cli`, `cargo install oxsearch-cli`): `oxsearch export`, `import` and `transfer` move documents to and from NDJSON files and between clusters, with their ids and routing, in slices, samples and limits.
- **Transport layers**: several nodes with failover, retries, sniffing, credentials (basic, bearer, API key, Elastic Cloud) and gzip.
- **Telemetry** through `tracing` and `metrics`, following the OpenTelemetry conventions for Elasticsearch, for [oxtel](https://github.com/agentstatelabs/oxtel) or any other exporter.
- **A `rig` vector store** ([`oxsearch-rig`](crates/oxsearch-rig)).
- **Escape hatches** for what isn't covered yet: your own `Endpoint`, `Raw` variants and `Client::send_raw` ([example](crates/oxsearch/examples/escape_hatches.rs)).

[docs/coverage.md](docs/coverage.md) lists the endpoints generated and those the integration tests exercise.

## Versions

oxsearch's versions are its own, 0.N while its API settles. Each release is generated from one branch of the [Elasticsearch specification](https://github.com/elastic/elasticsearch-specification), one Elasticsearch minor, and tested against that minor:

| oxsearch | Specification branch (commit) | Tested against |
|---|---|---|
| Next | 9.5 (`84bf510a`) | Elasticsearch 9.5.7 |
| 0.2 | 9.5 (`84bf510a`) | Elasticsearch 9.5.5 |
| 0.1 | 9.5 (`84bf510a`) | Elasticsearch 9.5.5 |

Elasticsearch 9 servers of other minors understand it too, for what both have; a newer server's additions read as `Raw` values until a release generated from its branch. Each endpoint's documentation says since which version it's there, and whether Elastic Cloud Serverless has it.

## Licenses of Elasticsearch features

Every API is generated whatever the license; a request for a feature the cluster's license doesn't include fails with `Error::License`. What the integration tests show on Elasticsearch 9.5:

| Feature | Basic | Above Basic (trial) |
|---|---|---|
| kNN query and retriever, `standard`, `pinned`, `rescorer`, `diversify` retrievers | ✓ | ✓ |
| `sparse_vector` with weights, `intervals`, everything else in the query DSL | ✓ | ✓ |
| Fusion on the client (`oxsearch::fusion`) | ✓ | ✓ |
| `rrf` and `linear` retrievers (server-side fusion) | | ✓ |
| `rule` and `text_similarity_reranker` retrievers | | ✓ |
| `semantic_text` with ELSER, and other machine learning models, including the ingest inference processor | | ✓ |
| Machine learning: anomaly detection, data frame analytics, trained models and their deployments | | ✓ |
| Machine learning: validating a detector, and filters | ✓ | ✓ |
| Watcher | | ✓ |
| Security: users, roles, role mappings, application privileges, API keys, service accounts, user profiles | ✓ | ✓ |
| Security: tokens; single sign-on through SAML, OpenID Connect and PKI | | ✓ |
| ES\|QL, aggregations, async search, EQL, SQL, ingest pipelines, enrich, synonyms, snapshots, snapshot lifecycle policies, ILM, transforms, rollups, administration | ✓ | ✓ |
| ES\|QL `FUSE` | | ✓ |
| Cross-cluster replication, searchable snapshots, autoscaling | | ✓ |
| Node shutdown, dangling indices, migration, feature states, ingest simulation, text structure | ✓ | ✓ |
| Query rules (Enterprise) and search applications (Platinum) | | ✓ |

Elastic's subscription matrix is the authority, and it has moved between releases.

## Development

The workspace needs Rust 1.95 or newer.

```bash
cargo test --workspace
```

The generator reads the specification's `schema.json` from the commit pinned in [spec/pin.toml](spec/pin.toml), one Elasticsearch minor at a time. This downloads it, checks its SHA-256 and reads it:

```bash
cargo run -p xtask -- spec check
```

The generated code in each crate's `src/generated` comes from that pin, and is never edited by hand. This writes it again, with round-trip tests from the specification's examples in each crate's `tests/examples.rs`:

```bash
cargo run -p xtask -- generate
```

The integration tests run against Elasticsearch in Docker, on the Basic license or a trial license:

```bash
scripts/es-fixture.sh up trial
OXSEARCH_TEST_LICENSE=trial cargo test -p oxsearch-integration -- --ignored
scripts/es-fixture.sh down
```

The security tests need security on, and skip otherwise; the fixture turns it on, with a password for the `elastic` user, like this:

```bash
export OXSEARCH_TEST_SECURITY=true OXSEARCH_TEST_PASSWORD=$(openssl rand -hex 16)
scripts/es-fixture.sh up trial
OXSEARCH_TEST_LICENSE=trial cargo test -p oxsearch-integration --test security -- --ignored
```

With the fixture up, the examples run against it:

```bash
cargo run -p oxsearch --example rag --features admin
```

## License

Apache-2.0. See [LICENSE](LICENSE) and [CONTRIBUTING.md](CONTRIBUTING.md); every commit is signed off under the Developer Certificate of Origin.

oxsearch is not affiliated with or endorsed by Elastic. Elasticsearch is a trademark of Elasticsearch B.V., used here only to say what oxsearch works with.
