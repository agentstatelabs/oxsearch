//! A typed Rust client for Elasticsearch, generated from the Elasticsearch
//! specification.
//!
//! The README's quick start (`examples/quickstart.rs`) does semantic search
//! over your own documents in a dozen lines; `examples/rag.rs` is the
//! retrieval half of a RAG application on any license.
//!
//! This crate re-exports the others behind features. The defaults are the
//! AI-search set: `search`, `docs`, `mapping` and `inference`; `aggs`
//! (aggregations with typed results), `esql` (ES|QL, with a typed builder
//! and rows read into your own types; `esql-arrow` adds Arrow output),
//! `admin` (cluster, index and node
//! administration), `analysis` and `tasks` are the other areas generated so
//! far. `document` adds the typed document layer and `derive` its derive
//! macro, `helpers` the point-in-time scan and the bulk loader, and
//! `transport-reqwest` the default HTTP transport.
//!
//! The [`Client`] sends the generated requests over a [`Transport`], such as
//! the one in `transport-reqwest`. [`Query`] and [`Retriever`] build the
//! common queries and retrievers in one call each, `fusion` fuses
//! retrievers' results on the client when the cluster's license doesn't
//! include the `rrf` and `linear` retrievers, and `document` reads and
//! writes your own document types.
//!
//! For what isn't covered yet, implement [`Endpoint`] for your own request
//! (`Route::segments` and `Body::json` help), write a query, retriever or
//! value as a `Raw` variant, or send any request with `Client::send_raw`;
//! `examples/escape_hatches.rs` shows each.
//!
//! The transport's layers are opt-in: `transport_reqwest`'s builder adds
//! several nodes with failover, retries, sniffing, credentials and
//! compression, [`auth`] signs requests with credentials that refresh, and
//! a layer of your own is a [`Transport`] that wraps another. [`telemetry`]
//! records a span per request and its duration through the `tracing` and
//! `metrics` facades, for oxtel or any other exporter.
//!
//! Every API is generated whatever the cluster's license. A request for a
//! feature the license doesn't include, such as the `rrf` retriever or
//! ELSER on Basic, fails with [`Error::License`].
//!
//! This release is generated from the Elasticsearch specification's 9.5
//! branch and tested against Elasticsearch 9.5; the README's version table
//! lists each release's branch.
//!
//! oxsearch is not affiliated with or endorsed by Elastic. Elasticsearch is
//! a trademark of Elasticsearch B.V.

// The derive macros' code names this crate `::oxsearch`, in its own tests too.
extern crate self as oxsearch;

mod generated;
pub use generated::*;

#[cfg(feature = "document")]
pub mod document;
#[cfg(feature = "helpers")]
pub mod helpers;

/// The crate under every area. Its common types are re-exported here at
/// the root, and its other types in their area's module; through it are
/// the types of areas not generated yet, such as `indices`.
pub use oxsearch_core;
pub use oxsearch_core::{
    AggsBody, Body, Client, ElasticsearchError, Endpoint, Error, HttpRequest, HttpResponse,
    LicenseError, Method, Query, RawAggs, Retriever, Route, RowError, Transport, TransportError,
    shapes,
};
pub use oxsearch_core::{auth, telemetry};

#[cfg(feature = "admin")]
pub use oxsearch_admin as admin;
#[cfg(feature = "aggs")]
pub use oxsearch_aggs as aggs;
#[cfg(feature = "analysis")]
pub use oxsearch_analysis as analysis;
#[cfg(feature = "derive")]
pub use oxsearch_derive as derive;
#[cfg(feature = "docs")]
pub use oxsearch_docs as docs;
#[cfg(feature = "esql")]
pub use oxsearch_esql as esql;
#[cfg(feature = "inference")]
pub use oxsearch_inference as inference;
#[cfg(feature = "mapping")]
pub use oxsearch_mapping as mapping;
#[cfg(feature = "search")]
pub use oxsearch_search as search;
#[cfg(feature = "search")]
pub use oxsearch_search::fusion;
#[cfg(feature = "tasks")]
pub use oxsearch_tasks as tasks;
#[cfg(feature = "transport-reqwest")]
pub use oxsearch_transport_reqwest as transport_reqwest;
