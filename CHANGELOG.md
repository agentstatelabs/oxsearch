# Changelog

All notable changes to oxsearch are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions are 0.N while
the API settles, and each release names the branch of the Elasticsearch
specification it's generated from.

## [Unreleased]

### Added
- **Aggregations with typed results** (`oxsearch-aggs`, the `aggs` feature).
  - The aggregation types are generated from the specification: every
    request container and every result.
  - `Agg` builds the common bucket, metric and pipeline aggregations,
    `.sub()` nests them, and `Aggs` names them for a search request's body.
  - `Aggregates` reads a response's results, typed, from a request sent with
    `typed_keys`. `Results` picks them out by name (`value`, `stats`,
    `terms`, `histogram`, `date_histogram`, `ranges`, `filters`,
    `percentiles`, `top_hits`, …), in a bucket's sub-aggregations too.
- **Administration APIs** (`oxsearch-admin`, the `admin` feature), generated
  from the specification: indices (create, get, settings, mappings, aliases,
  templates, data streams, rollover, blocks, clone, shrink, split, stats,
  …), the cluster (health, settings, state, stats, component templates,
  allocation) and nodes (info, stats, usage, hot threads).
- Calls that answer in plain text, such as `nodes.hot_threads`, read as a
  `String` (`Endpoint::text_response`).
- **ES|QL** (`oxsearch-esql`, the `esql` feature): the query and async query
  APIs, generated from the specification, with:
  - `Esql`, a typed query builder: `from_index`, `row`, `where_`, `eval`,
    `keep`, `drop`, `rename`, `sort`, `limit`, `stats`, `mv_expand`, `fork`
    and `fuse`, with `then` for any other command. Values go into the
    request's `params`, never into the query, and column names are quoted
    where they need it. `col("year").gte(2020)` and the other conditions,
    `Expr` for the rest.
  - Rows read into your own types by column name (`Esql::fetch`,
    `EsqlResult::rows`): dotted columns fill nested structs, and a single
    value reads into a `Vec` field. A row that doesn't fit fails with the
    new `Error::Row`, naming the row and the column.
  - `Index<T>::esql`, a query `FROM` a typed document index, whose rows
    read back as the documents.
  - Arrow output, with the `esql-arrow` feature: `Esql::fetch_arrow` runs a
    query with `format=arrow` and reads the result as Arrow record batches
    (`arrow-array` 60), without JSON in between.
- `Client::send_bytes`, which returns a response's body as it came, for
  formats other than JSON.
- **Ingest pipelines** (`oxsearch-ingest`, the `ingest` feature), generated
  from the specification: pipelines and their 45 processors (each converts
  into a `ProcessorContainer`), simulation, the grok patterns, and the
  GeoIP and IP location databases.
- **Query rules** (`oxsearch-query-rules`, the `query-rules` feature):
  rulesets whose rules pin or exclude documents for queries that match
  their criteria, which the `rule` retriever applies; they need a license
  above Basic.
- **Synonym sets** (`oxsearch-synonyms`, the `synonyms` feature): synonym
  rules stored in Elasticsearch, which analyzers read and which take
  effect without reindexing.
- **The license API** (`oxsearch-license`, the `license` feature).
- **Snapshots** (`oxsearch-snapshot`, the `snapshot` feature): repositories,
  and snapshots taken, cloned, restored, analyzed and deleted.
- **Index lifecycle management** (`oxsearch-ilm`, the `ilm` feature):
  policies and their phases, the indices they manage, and ILM itself
  stopped and started.

### Changed
- The typed document layer is the `document` feature, which `derive`
  implies, and creates and maps indices with the `admin` crate's typed
  calls rather than raw ones. It adds `Index::delete_index`.
- Mapping, analysis and common types that the admin responses read
  (mappings, index and node statistics, …) now take a required property
  Elasticsearch leaves out as `None`, as other response types do: such
  fields are `Option`s.
- Generated names no longer end in `2` where two properties would share
  one.
  - When a type has both `_x` and `x`, `x` keeps the plain name and `_x`
    its underscore. The node APIs' map of nodes is `nodes`, beside the
    `_nodes` header.
  - Two fields from 0.1 now mean the other property, with the same type,
    so code using them still compiles: `CompletionSuggestOption::score` is
    the suggestion's `score` (the hit's is `_score`, which was `score`),
    and `PercolateQuery::name` is the percolator's `name` (the query's own
    is `_name`, which was `name`).
  - A query parameter that repeats a path parameter isn't generated twice:
    `index2` (`msearch`, `indices.analyze`, `indices.clear_cache`),
    `scroll_id2` (`scroll`) and `context2` (`put_script`) are gone. Set the
    path parameter instead.

### Fixed
- A union tagged by a boolean, such as an ILM explanation's `"managed":
  true`, reads and writes its tag as a boolean.

## [0.1.0] - 2026-10-15

The first release: a typed Rust client for Elasticsearch 9, generated from
the Elasticsearch specification's 9.5 branch (commit `84bf510a`) and tested
against Elasticsearch 9.5.5 on the Basic license and a trial. It's made for
AI search: retrievers, `semantic`, `knn` and `sparse_vector` queries,
`semantic_text` mappings and the inference API, with a typed document layer
on top. Rust 1.95 or newer.

### Added
- **Generated crates** for search, documents, mappings, analysis, inference
  and tasks, and the common types under them in `oxsearch-core`, re-exported
  by the `oxsearch` front crate behind features (search, docs, mapping and
  inference by default).
  - Every type the specification defines for them, with constructors taking
    the required properties and setters for the rest. Enums, containers and
    tagged unions have `Raw` variants, so values a newer Elasticsearch writes
    still read. Responses tolerate required properties Elasticsearch leaves
    out.
  - Beta and experimental endpoints are behind the `beta` and
    `experimental` features.
  - Round-trip tests from the specification's own request and response
    examples.
- **A client and transports.** `Client` sends any generated request. `send_as`
  reads responses into your own document types.
  - Errors are typed: a license refusal (`Error::License`, naming the
    feature) and a version conflict (`Error::Conflict`) have their own
    variants, and `ElasticsearchError::causes` walks the cause chain.
  - `oxsearch-transport-reqwest` has opt-in layers: several nodes with
    failover and resting failed nodes, retries with backoff, sniffing, Elastic
    Cloud IDs, credentials (basic, bearer, API key) and gzip.
  - `oxsearch_core::auth::Authenticated` signs requests with credentials that
    refresh. A layer of your own is a `Transport` that wraps another.
- **Query and retriever builders**: `Query` and `Retriever` build the common
  queries and every retriever in one call each. A query works wherever a
  retriever does.
- **Fusion on the client** (`oxsearch::fusion`): reciprocal rank fusion and
  linear combinations over one multi-search, matching the server's scores.
  It's used when the cluster's license doesn't include the `rrf` and
  `linear` retrievers.
- **A typed document layer** (`oxsearch::document`, the `derive` feature).
  `#[derive(Document)]` maps a struct, and `Index<T>` puts, gets, deletes,
  searches and fuses it. `ensure_mapping` creates the index, adds missing
  fields and reports drift. Field handles offer the queries each field's type
  suits, and `Versioned<T>` gives optimistic concurrency.
- **Helpers** (`oxsearch::helpers`, the `helpers` feature): a point-in-time
  scan with slices, and a bulk loader with backpressure, batching,
  concurrency, backoff on 429 and an outcome per action.
- **Telemetry** through `tracing` and `metrics`, per the OpenTelemetry
  conventions for Elasticsearch: a client span per request with sanitized
  query text, `db.client.operation.duration`, GenAI attributes on inference
  calls, retries as span events, and metrics for the helpers.
- **`oxsearch-rig`**: a [rig](https://docs.rs/rig-core) 0.44 vector store on
  Elasticsearch. It searches by kNN or `semantic_text`, optionally hybrid,
  with rig's filters as Elasticsearch queries.
- **Escape hatches**: your own `Endpoint` (`Route::segments`, `Body::json`),
  `Raw` variants, and `Client::send_raw`.
- [docs/coverage.md](docs/coverage.md) lists which generated endpoints the
  integration tests exercise.

[Unreleased]: https://github.com/agentstatelabs/oxsearch/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/agentstatelabs/oxsearch/releases/tag/v0.1.0
