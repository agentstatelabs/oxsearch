//! The core of oxsearch: the [`Client`], the [`Endpoint`] and [`Transport`]
//! traits it sends calls through, the [`Error`]s, and the types every API
//! area shares (common types, the query DSL, retrievers and kNN), generated
//! from the Elasticsearch specification, with the [`shapes`] they're built
//! from.

// The generated code names this crate by its name, as other crates do.
extern crate self as oxsearch_core;

mod generated;
pub use generated::*;

mod builders;
mod client;
mod endpoint;
mod error;
mod seams;
pub mod shapes;

pub use builders::{Query, Retriever};
pub use client::Client;
pub use endpoint::{Body, Endpoint, HttpRequest, HttpResponse, Method, Route, Transport};
pub use error::{ElasticsearchError, Error, LicenseError, TransportError};
pub use seams::{AggsBody, RawAggs};

#[doc(hidden)]
pub mod __private;
