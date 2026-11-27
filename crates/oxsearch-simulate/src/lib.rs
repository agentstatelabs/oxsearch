//! Ingest simulation, generated from the Elasticsearch specification:
//! documents run through the pipelines an index would use, with substitute
//! pipelines, templates and mappings if given, and checked against its
//! mapping, without indexing them.
mod generated;
pub use generated::*;
