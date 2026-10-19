//! Index lifecycle management, generated from the Elasticsearch
//! specification: policies whose phases roll indices over, shrink,
//! force-merge and delete them as they age, and the indices' progress
//! through them.
mod generated;
pub use generated::*;
