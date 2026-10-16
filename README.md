# oxsearch

A typed Rust client for Elasticsearch, generated from the Elasticsearch specification, with typed AI search: retrievers, `semantic`, `knn` and `sparse_vector` queries, `semantic_text` mappings, the inference API, and a typed document layer.

**Status: 0.1, the first release.** The generated crates cover search, documents, mappings, analysis, inference and tasks, tested against the specification's own examples and against Elasticsearch 9 on the Basic license and a trial. Aggregations, ES|QL and the administration APIs come next; see [DESIGN.md](DESIGN.md) for the plan and [CHANGELOG.md](CHANGELOG.md) for what's in each release.

## Quick start

Semantic search over your own documents, with `features = ["derive", "transport-reqwest"]`. `semantic_text` embeds with the cluster's default ELSER model, which needs a license above Basic, such as a trial; on Basic, the [RAG example](crates/oxsearch/examples/rag.rs) uses your own embeddings and fuses on the client instead.

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

- **Generated types and endpoints** for every request and response in the first slice, with builders, `Raw` variants for what a newer Elasticsearch adds, and typed errors (license refusals and version conflicts have their own).
- **Aggregations with typed results** (`oxsearch::aggs`, the `aggs` feature): `Agg::terms("category", 10).sub("avg_price", Agg::avg("price"))`, sent with `typed_keys` and read back as `Aggregates`.
- **Query and retriever builders**: `Query::r#match`, `Query::semantic`, `Retriever::knn`, `Retriever::rrf`, …; and **fusion on the client** (`oxsearch::fusion`) when the license doesn't include the `rrf` and `linear` retrievers.
- **A typed document layer** (`oxsearch::document`): `#[derive(Document)]` maps a struct, `Index<T>` reads and writes it, and field handles offer only the queries that suit each field.
- **Helpers** (`oxsearch::helpers`): a point-in-time scan and a bulk loader with backpressure.
- **Transport layers**: several nodes with failover, retries, sniffing, credentials (basic, bearer, API key, Elastic Cloud) and gzip.
- **Telemetry** through `tracing` and `metrics`, following the OpenTelemetry conventions for Elasticsearch, for [oxtel](https://github.com/agentstatelabs/oxtel) or any other exporter.
- **A `rig` vector store** ([`oxsearch-rig`](crates/oxsearch-rig)).
- **Escape hatches** for what isn't covered yet: your own `Endpoint`, `Raw` variants and `Client::send_raw` ([example](crates/oxsearch/examples/escape_hatches.rs)).

[docs/coverage.md](docs/coverage.md) lists the endpoints generated and those the integration tests exercise.

## Versions

oxsearch's versions are its own, 0.N while its API settles. Each release is generated from one branch of the [Elasticsearch specification](https://github.com/elastic/elasticsearch-specification), one Elasticsearch minor, and tested against that minor:

| oxsearch | Specification branch (commit) | Tested against |
|---|---|---|
| 0.1 | 9.5 (`84bf510a`) | Elasticsearch 9.5.5 |

Elasticsearch 9 servers of other minors understand it too, for what both have; a newer server's additions read as `Raw` values until a release generated from its branch.

## Licenses of Elasticsearch features

Every API is generated whatever the license; a request for a feature the cluster's license doesn't include fails with `Error::License`. What the integration tests show on Elasticsearch 9.5:

| Feature | Basic | Above Basic (trial) |
|---|---|---|
| kNN query and retriever, `standard`, `pinned`, `rescorer`, `diversify` retrievers | ✓ | ✓ |
| `sparse_vector` with weights, `intervals`, everything else in the query DSL | ✓ | ✓ |
| Fusion on the client (`oxsearch::fusion`) | ✓ | ✓ |
| `rrf` and `linear` retrievers (server-side fusion) | | ✓ |
| `rule` and `text_similarity_reranker` retrievers | | ✓ |
| `semantic_text` with ELSER, and other machine learning models | | ✓ |

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

With the fixture up, the examples run against it:

```bash
cargo run -p oxsearch --example rag --features transport-reqwest
```

## License

Apache-2.0. See [LICENSE](LICENSE) and [CONTRIBUTING.md](CONTRIBUTING.md); every commit is signed off under the Developer Certificate of Origin.

oxsearch is not affiliated with or endorsed by Elastic. Elasticsearch is a trademark of Elasticsearch B.V., used here only to say what oxsearch works with.
