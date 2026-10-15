//! A typed Rust client for Elasticsearch, generated from the Elasticsearch
//! specification.
//!
//! This crate re-exports the others behind features. The defaults are the
//! AI-search set: `search`, `docs`, `mapping` and `inference`. The other API
//! areas are `aggs`, `analysis`, `esql`, `admin` and `tasks`; `derive` adds
//! the typed document layer's derive macros, and `transport-reqwest` the
//! default HTTP transport.
//!
//! The [`Client`] sends the generated requests over a [`Transport`], such as
//! the one in `transport-reqwest`. [`Query`] and [`Retriever`] build the
//! common queries and retrievers in one call each, and `fusion` fuses
//! retrievers' results on the client when the cluster's license doesn't
//! include the `rrf` and `linear` retrievers. Retries, failover, authentication and
//! telemetry are still to come; see DESIGN.md in the repository.

mod generated;
pub use generated::*;

/// The crate under every area. Its common types are re-exported here at
/// the root, and its other types in their area's module; through it are
/// the types of areas not generated yet, such as `indices`.
pub use oxsearch_core;
pub use oxsearch_core::{
    AggsBody, Body, Client, ElasticsearchError, Endpoint, Error, HttpRequest, HttpResponse,
    LicenseError, Method, Query, RawAggs, Retriever, Route, Transport, TransportError, shapes,
};

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
