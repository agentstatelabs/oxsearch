//! A typed Rust client for Elasticsearch, generated from the Elasticsearch
//! specification.
//!
//! This crate re-exports the others behind features. The defaults are the
//! AI-search set: `search`, `docs`, `mapping` and `inference`. The other API
//! areas are `aggs`, `analysis`, `esql` and `admin`; `derive` adds the typed
//! document layer's derive macros, and `transport-reqwest` the default HTTP
//! transport.
//!
//! Nothing is generated yet. The generator that fills these crates is being
//! built; see DESIGN.md in the repository.

// oxsearch-core's traits and shared types will be re-exported at the root
// once there are any.

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
#[cfg(feature = "transport-reqwest")]
pub use oxsearch_transport_reqwest as transport_reqwest;
