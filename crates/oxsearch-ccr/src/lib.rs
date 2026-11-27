//! Cross-cluster replication, generated from the Elasticsearch
//! specification: follower indices that copy an index on a remote cluster as
//! it changes, paused, resumed and turned into ordinary indices; auto-follow
//! patterns that follow new indices by name; and their statistics.
mod generated;
pub use generated::*;
