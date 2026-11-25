//! Projects, generated from the Elasticsearch specification: a project's
//! tags and those of the projects linked to it, and the routing expressions
//! that cross-project search uses. Only Elastic Cloud Serverless has these:
//! a self-managed cluster refuses them.
mod generated;
pub use generated::*;
