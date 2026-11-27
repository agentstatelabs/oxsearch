//! Node shutdown, generated from the Elasticsearch specification: telling
//! the cluster a node will restart, leave or be replaced, so that it moves
//! the node's shards and tasks first, and how far that has got.
//! Elasticsearch meant these for Elastic Cloud, ECE and ECK to call, not for
//! direct use.
mod generated;
pub use generated::*;
