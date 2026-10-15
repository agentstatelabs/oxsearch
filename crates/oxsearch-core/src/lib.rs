//! The core of oxsearch: the types every API area shares (common types, the
//! query DSL, retrievers and kNN), generated from the Elasticsearch
//! specification, and the [`shapes`] they're built from. The `Transport` and
//! `Endpoint` traits and the errors will join them.

// The generated code names this crate by its name, as other crates do.
extern crate self as oxsearch_core;

mod generated;
pub use generated::*;

pub mod shapes;

#[doc(hidden)]
pub mod __private;
