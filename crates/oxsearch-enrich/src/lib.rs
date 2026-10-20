//! Enrich policies, generated from the Elasticsearch specification: a
//! source index looked up by a field, so that the enrich processor adds its
//! data to documents as they are ingested.
mod generated;
pub use generated::*;
