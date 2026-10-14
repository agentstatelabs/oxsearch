# oxsearch: a typed Rust client for Elasticsearch, generated from its specification

_Written 2026-10-14. Nothing is built yet; Phase 0 is a generator spike with a gate. The numbers below come from a quick analysis of the specification on that date and are approximate; the generator will compute them for real._

## 1. Why

Rust has no typed, production-ready client for Elasticsearch, and none of its AI-search features are typed anywhere in Rust.

- **The official `elasticsearch` crate is untyped and still alpha.** Every release is `-alpha.1`, and none has shipped since 9.1.0-alpha.1 (2025-08-08). Request and response bodies are raw JSON or the caller's own structs. Its transport sends once, with no retries or failover. It has no bulk helper, no point-in-time paging helper, and no tracing.
- **The community DSL (`elasticsearch-dsl`) covers queries well but stops there.** It has 59 query types, but only 19 aggregations and untyped aggregation results. It has no retrievers, RRF, `semantic`, `sparse_vector` or `intervals`. It is never tested against a real cluster, and it silently drops empty queries, so an empty `terms` filter matches every document.
- **Rust's AI frameworks don't integrate with Elasticsearch.** `rig` has vector-store integrations for Qdrant, LanceDB, Milvus, MongoDB, Postgres, Neo4j, SurrealDB and S3 Vectors; Swiftide for Qdrant, LanceDB, pgvector and Redis. Neither has Elasticsearch.

Elasticsearch publishes a machine-readable specification of its whole API: `elasticsearch-specification`, Apache-2.0, updated daily, with 610 endpoints and 3,943 types. It already defines everything a typed client needs, including all nine retrievers, the `semantic`, `sparse_vector`, `intervals` and `knn` queries, the `semantic_text`, `dense_vector` and `sparse_vector` mappings, and the inference API. oxsearch generates from it.

## 2. Principles

1. **Generated, not hand-written.** Every request, response and type comes from the specification, pinned per Elasticsearch minor version and regenerated, never edited by hand. Hand-written code only adds convenience on top.
2. **Depend only on what you use.** The code is split into crates by how the types actually depend on each other, so an app that only searches doesn't compile aggregations or machine learning.
3. **Never block a developer.** Every layer has an escape hatch: user-defined endpoints, raw variants in every enum, container and tagged union, a raw call, and middleware.
4. **Typed where it pays, open where it doesn't.** The user's documents and aggregation results are type parameters, defaulting to raw JSON.
5. **Features only add.** Generated structs are `#[non_exhaustive]` and built through builders, so enabling a feature anywhere in a build can't break code elsewhere.
6. **Proven against real Elasticsearch.** Integration tests run against Elasticsearch in Docker, on both the free Basic license and a trial license. An API area counts as supported only once it has tests.
7. **Telemetry through the facades.** Library crates emit `tracing` spans and `metrics` using the OpenTelemetry semantic conventions for Elasticsearch and GenAI, and depend on no telemetry runtime. Any subscriber works; oxtel is the one the examples use.
8. **Open and contributable.** Apache-2.0, DCO sign-off, all code written by us or generated from the Apache-2.0 specification.

## 3. What it covers

**Phase 1, the first release: AI search.**
- Search requests and responses with all nine retrievers (standard, knn, rrf, linear, text_similarity_reranker, rule, rescorer, pinned, diversify) and the `knn`, `semantic`, `sparse_vector`, `intervals` and full query-DSL queries.
- Mappings, including `semantic_text`, `dense_vector` (with index and quantization options) and `sparse_vector`.
- The inference API: create an endpoint, embeddings, sparse embeddings, rerank, completion.
- Document APIs: index, get, multi-get, update, delete, bulk.
- A point-in-time stream and a bulk-loading helper.
- Client-side fusion (RRF and linear) for clusters without a license for server-side fusion.
- The typed document layer: `#[derive(Document)]` and `Index<T>`.
- A `rig` vector-store integration.

**Phase 2, breadth:** aggregations with typed results, ES|QL with a builder and rows mapped into structs, the remaining API areas, the import/export CLI, and a version matrix.

## 4. Architecture

```
 your app ── oxsearch (front crate, feature flags)
               │
   convenience │  oxsearch-derive · Index<T> · retriever/query builders · PIT stream · bulk helper · client-side fusion
   ────────────┼──────────────────────────────────────────────────────────────────────────────────────────────
   generated   │  oxsearch-search · -docs · -inference · -mapping · -aggs · -esql · -admin · …one crate per API area
               │  oxsearch-analysis
               │  oxsearch-core (common types, query DSL, retrievers, kNN, Endpoint trait, Transport trait, errors)
   ────────────┼──────────────────────────────────────────────────────────────────────────────────────────────
   transport   │  middleware: auth · retries/failover · sniffing · compression · telemetry · user layers
               │  oxsearch-transport-reqwest (default)
```

### 4.1 Crates

| Crate | Contents | Approx. types |
|---|---|---|
| `oxsearch-core` | transport and `Endpoint` traits, errors, common types, query DSL, retrievers, kNN | 400 |
| `oxsearch-analysis` | analyzers, tokenizers, filters | 185 |
| `oxsearch-mapping` | field mappings, including `semantic_text`, `dense_vector`, `sparse_vector` | 100 |
| `oxsearch-aggs` | aggregation requests and typed results | 280 |
| `oxsearch-search` | search, msearch, point-in-time, scroll, explain | 70, plus endpoints |
| `oxsearch-inference` | inference endpoints, one feature flag per provider | 340, mostly per-provider settings |
| `oxsearch-docs` | index, get, multi-get, update, delete, bulk | – |
| `oxsearch-esql`, `oxsearch-admin` (cluster + indices + nodes), `oxsearch-ingest`, `oxsearch-security`, `oxsearch-ml`, … | one crate per API area | the rest of about 2,900 API types |
| `oxsearch-transport-reqwest` | the default HTTP transport | – |
| `oxsearch-derive` | derive macros for the typed document layer | – |
| `oxsearch-rig` | `rig` vector-store integration | – |
| `oxsearch` | front crate that re-exports the rest behind feature flags | – |

The default features are the AI-search set: search, docs, mapping, inference. `features = ["aggs", "esql"]` adds more. Libraries depend on the specific crates they need.

**Why crates and not only feature flags:** Cargo compiles crates in parallel and caches each one separately; feature flags inside one huge crate get neither benefit.

### 4.2 The generator

- **Input:** the specification's `schema.json`, pinned per Elasticsearch minor version (`spec/pin.toml` holds the commit on the minor's branch and the file's SHA-256) and verified by checksum on every run.
- **Our own model of `schema.json`, not the specification's `clients_schema` crate** (decided in Phase 0, 2026-10-14). The generator reads the JSON with a serde model written for it (`xtask/src/spec/model.rs`, about 500 lines), because:
  - `clients_schema` isn't published to crates.io (`publish = false`). It could only be a git dependency on the specification repository, which is about 1.4 GB and which Cargo clones whole into every cold CI cache.
  - It accepts fields it doesn't know. Our model denies unknown fields, so a specification whose format has changed fails to load, naming the new field, rather than being read in part. A pinned, checksummed input needs that check.
  - It brings dependencies the generator doesn't need (clap, a derive_more beta, arcstr), and its API serves the specification repository's own tools, with no compatibility promise.
  - Our model reads the whole 9.5 specification (3,904 types, 603 endpoints), and only the generator depends on it.
- **Placement by type graph.** Elasticsearch's namespaces can't map to crates: 15 of them (2,437 types) depend on each other in a loop, which Cargo forbids. About 30 type references cause the loop. The generator (`xtask/src/placement.rs`):
  - starts each type in its namespace's crate (cluster, indices and nodes share `admin`, which keeps their stats loop inside one crate);
  - orders the foundation crates (core, analysis, mapping, tasks, inference, search, aggs, docs), each depending only on crates below it; every other crate is an API area above them, and the areas must form no loop;
  - moves types to the lowest crate that can hold them: a reference to a crate the referrer may not depend on moves the type into the referrer's crate, or into the core when neither crate may depend on the other, repeated until nothing moves (36 types on the 9.5 specification, mostly search types the core's retrievers and kNN use, such as inner hits, highlighting and rescoring);
  - breaks a small number of references with seams (three: a script's source may be a whole search body, the search request's and response's aggregations, and one Watcher-only reference; 7 references in all);
  - fails CI if the placement changes unexpectedly, and publishes the placement as a report (`spec/placement.md`).
- **Serde for the hard shapes.** The specification describes each shape, and `spec/shapes.md` counts them (on 9.5: in all / in the first slice's crates). The generator maps them to serde as follows (the plan for Phase 0; not built yet):
  - **Structs** (2,017 / 497) are `#[non_exhaustive]`, with optional fields as `Option`s left out when empty, a constructor taking the required fields, and a setter for each optional one. Inherited properties are copied in. Generated structs accept unknown fields, so responses from a newer Elasticsearch still parse.
  - **Containers with one variant keyed by its name** (59 / 23: queries, retrievers, aggregations, processors) are enums with a generated visitor that reads the key and deserializes the variant directly, without buffering. An unknown key becomes `Raw`. A container with properties beside the variant (11 / 6, such as an aggregation's `aggregations` and `meta`) is a struct holding that enum.
  - **Objects keyed by a field name** (56 / 26, `{"term": {"user.id": {...}}}`) and **structs with one property keyed by a field name** (10 / 8, such as `geo_distance`) hold a field-and-value pair, written as a one-key object.
  - **Shorthand forms** (25 / 20, `{"term": {"user.id": "kimchy"}}`) are recognized by their JSON kind on input, and written short when only the shorthand property is set, so the specification's examples round-trip exactly.
  - **Internally tagged unions** (14 / 2: analyzers, token filters, mapping properties) are read through `serde_json::Value`, so a missing tag can take its default (3, such as `object` for a mapping property) and an unknown tag becomes `Raw`.
  - **Externally tagged unions** (3 / 1: aggregation results and suggestions) are requested with `typed_keys` and read from keys such as `sterms#by_tag`.
  - **Untagged unions with an untyped fallback** (3 / 3: `range`, `distance_feature` and decay functions) are always read as the untyped variant, which keeps every field; the typed variants are for building requests.
  - **Other unions** (373 / 121): `T` or `T[]` (178) becomes a one-or-many type that writes back the form it read. Members told apart by JSON kind (168) are dispatched on it, without buffering. Members that share a primitive kind (14, such as an enum or a string) try the narrower one first. Only 8 unions have two or more object members (7 in the first slice, such as `GeoLocation` and `RRFRetrieverEntry`); the generator tells them apart by a property only one member has, and msearch's header and body lines by position.
  - **Enums** (390) all get a `Raw(String)` variant, not only the 27 the specification marks open, so a value added in a later Elasticsearch minor doesn't break reading a response (decided with the user, 2026-10-14).
  - **Recursive types** are boxed where a loop of references closes (`spec/placement.md` counts them per crate).
- **Round-trip tests from the specification's own request and response examples**, so every generated type is exercised.
- **Availability becomes features and docs.** Beta and experimental endpoints sit behind `beta` and `experimental` features (24 and 73 endpoints on the analysis date). Stack and serverless availability appear in the docs. Endpoints the specification excludes from code generation are skipped.
- **Docs come from the specification's descriptions and links.**

### 4.3 Seams

- **Type parameters for user data.** `SearchResponse<Doc, Aggs = RawAggs>`. `_source` goes into the user's struct. Aggregation results are raw JSON by default, `oxsearch-aggs` types, or the user's own struct.
- **Conversions on the request side.** The search request takes `impl Into<AggsBody>`. `oxsearch-aggs` provides the conversion, and `json!` works too, so `oxsearch-search` never depends on `oxsearch-aggs`. A script's search-body source gets the same treatment.

### 4.4 Transport and middleware

- `Transport` is a trait in `oxsearch-core`; generated crates depend only on it. `oxsearch-transport-reqwest` is the default implementation.
- Middleware layers, each opt-in:
  - authentication (basic, bearer, API key, Elastic Cloud ID) and credential refresh;
  - retries with backoff and failover across nodes, with dead-node marking and resurrection;
  - node sniffing;
  - request compression;
  - telemetry.
  Users can add their own layers.
- Errors are typed: transport failures, Elasticsearch error responses with their cause chain, and license errors (a feature the cluster's license doesn't allow) as their own variant.

### 4.5 Escape hatches

1. **An `Endpoint` trait.** Generated endpoints implement it; users implement it for anything not yet covered and get the same transport, middleware and telemetry.
2. **Raw variants in every enum, container and tagged union**, such as `Query::Raw(json!(...))`, `Retriever::Raw(...)` and `SortOrder::Raw("...")`, so new Elasticsearch features work before a regeneration, and values a newer Elasticsearch returns still parse.
3. **A raw call**, `client.send(method, path, body)`.
4. **Middleware**, as above.

### 4.6 The typed document layer

Rust has ORMs (sqlx, Diesel, SeaORM) but nothing like JPA, and JPA's model doesn't fit Elasticsearch: there are no transactions, and writes become visible only after a refresh. The model here is MongoDB's Rust driver, `Collection<T>`, with Elasticsearch's own strengths added. Roughly (API not final):

```rust
#[derive(Document, Serialize, Deserialize)]
#[index("articles")]
struct Article {
    #[id] id: String,
    #[semantic_text(inference_id = "my-embeddings")] body: String,
    #[keyword] tags: Vec<String>,
    published: DateTime<Utc>,
}

let articles = client.index::<Article>();
articles.ensure_mapping().await?;          // mapping comes from the struct; reports drift from the live index
articles.put(&article).await?;
let hits = articles                         // Vec<Hit<Article>>
    .search(Retriever::rrf([
        Article::BODY.semantic("how do I tune HNSW?"),
        Article::TAGS.term("vector"),       // field names checked at compile time
    ]))
    .await?;
```

- **The mapping is defined by the struct.** Field attributes choose mapped types, and Rust types give defaults. `ensure_mapping()` creates the index or reports drift. A changed field type is reported, never applied silently, because Elasticsearch can't change a mapped type in place.
- **Typed field handles** offer only the queries that suit the field's mapped type: `semantic()` on `semantic_text`, `term()` on `keyword`, `range()` on dates and numbers.
- **`Versioned<T>`** carries the sequence number and primary term for optimistic concurrency; a stale write fails with a typed conflict error.
- **ES|QL rows map into structs** by column name (Phase 2).
- **Left out:** SQL emulation, relations between indices (ES|QL's `LOOKUP JOIN` does that on the server), lazy loading, change tracking, transactions, a unit of work.

### 4.7 Helpers

- **Point-in-time stream.** An async stream of `Hit<T>` over point-in-time and `search_after`.
  - It splits into slices for parallel reads.
  - It renews a short keep-alive with each page, ends at the first empty page (no exact total needed), and closes the point-in-time when it finishes.
- **Bulk-loading helper.**
  - Bounded channels give backpressure.
  - It has a concurrency limit and flushes by document count, bytes and interval.
  - It backs off on 429 responses.
  - It returns results item by item, and the first fatal error cancels the pipeline.
- **Client-side fusion.** RRF and linear fusion over a multi-search, for clusters whose license doesn't include server-side fusion. It uses the same retriever builders as the server-side path.

Rules for every pipeline: no locks shared between producer and consumer, end of stream is a channel closing rather than a flag, errors travel with the data as `Result`s, and work is handled in completion order.

### 4.8 Telemetry

- One client span per request, named and attributed per the OpenTelemetry database conventions for Elasticsearch (`db.system.name`, operation name, sanitized query text off by default, server address and port, node name), plus a request-duration metric.
- Inference calls (embeddings, rerank, completion) also get GenAI attributes.
- Retries and failover appear as span events, so a slow request shows why it was slow.
- Bulk and point-in-time helpers report queue depth, requests in flight, and documents and bytes per second.
- Library crates use `tracing` and `metrics` only. Attribute names come from `oxtel-semconv`'s generated constants, a light dependency with no runtime.

## 5. Licenses

Elasticsearch's free Basic license includes kNN and the standard, knn, pinned and rescorer retrievers. Server-side RRF, linear, rule and rerank retrievers, and Elastic's managed inference, need a paid license or a trial. (Elastic's subscription matrix should be checked against each release; the tiers have moved before.)

- Every API is generated regardless of license. Docs note features that need a paid license.
- License failures come back as a typed error.
- Client-side fusion gives Basic clusters hybrid search.
- CI runs the integration tests twice: once on Basic, once on a trial license.

## 6. Proof

- Round-trip tests from the specification's examples for every generated type.
- Integration tests per API area against Elasticsearch in Docker, on Basic and trial licenses.
- A published coverage report: endpoints generated, endpoints with integration tests, by area.
- A compile-time budget for the default feature set, measured with `cargo build --timings` in CI.
- Benchmarks against the official crate for request overhead and bulk throughput (Phase 2).
- The reference demo (section 7) running on a real cluster.

## 7. Reference demo

A Rust retrieval service built on oxsearch, behind an existing Java RAG application.
- The Java service gets a `SearchBackend` interface with two implementations: the Elasticsearch Java client, and an HTTP client for the Rust service. Config switches between them, and the UI stays the same.
- The Rust service implements the same five operations with the same responses: search, get by id, retrieve for chat, retrieve for a listing, and location suggestions.
- On Basic it uses embeddings from a local model (Ollama), kNN plus BM25, and client-side RRF. With a trial license, the same requests switch to server-side RRF and linear retrievers and `semantic_text`.
- One trace runs from the Java service through the Rust service to Elasticsearch: the Java agent and oxtel export OTLP to a collector, and the trace appears in Kibana.
- The service lives in `demo/` in this repo. The Java application stays in its own repository, and the cluster deployment in the cluster's GitOps repository.

## 8. Plan

| Phase | Scope | Gate |
|---|---|---|
| 0 | Workspace and CI; specification ingestion; placement by type graph; serde for the hard shapes; the AI-search slice generated with a minimal transport; integration tests on Basic and trial; compile-time budget | The generator handles the hard shapes cleanly, the slice passes against Elasticsearch 9.x, and the compile time is acceptable |
| 1 | Transport middleware; telemetry; escape hatches; retriever and query builders; client-side fusion; typed document layer; point-in-time stream and bulk helper; `rig` integration; docs and examples; 0.1 on crates.io | The reference demo runs on 0.1 |
| 2 | Aggregations; ES|QL; remaining API areas; the rest of the document layer; import/export CLI; version matrix; benchmarks | Outside adopters |

## 9. Governance

- Apache-2.0, with DCO sign-off on every commit.
- oxsearch is not affiliated with or endorsed by Elastic. Elasticsearch is a trademark of Elasticsearch B.V.; it is used only to say what oxsearch works with, and never in crate names.

## 10. Non-goals

- OpenSearch. Its API has diverged; oxsearch targets Elasticsearch only.
- An ORM, SQL emulation, or SQL-to-DSL translation.
- A blocking API in Phase 1.
- Replacing Elasticsearch's server-side features (reindex, ES|QL joins) with client-side ones, except fusion for Basic clusters.

## 11. Open questions

1. **Version numbers.** Follow Elasticsearch's (9.x) like the official clients, or 0.N with the specification branch recorded?
2. ~~**The specification model.** Reuse `clients_schema` from the specification repository, or read `schema.json` with our own model?~~ Decided in Phase 0: our own model (§4.2).
3. **The oldest supported Elasticsearch.** 9.0, or 8.19 through compatibility headers?
4. **Serverless.** Generate a serverless profile from `schema-serverless.json`, or only document availability?
5. **MSRV**, and whether `oxsearch-transport-reqwest` is on by default.
