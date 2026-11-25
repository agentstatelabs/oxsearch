//! Fleet, generated from the Elasticsearch specification: an index's global
//! checkpoints, searches that wait until a checkpoint is visible, and
//! Fleet's secrets. Elasticsearch meant the secrets for Fleet Server to
//! call, not for direct use.
mod generated;
pub use generated::*;
