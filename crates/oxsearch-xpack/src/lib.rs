//! X-Pack information and usage, generated from the Elasticsearch
//! specification: the build, the license and which features are available
//! and enabled, and statistics on how much each is used; and [`monitoring`]'s
//! bulk request, which Elasticsearch meant for the monitoring features to
//! call, not for direct use. Its actions name a `_type`, which a bulk
//! request's no longer do, so they're written as JSON.
mod generated;
pub use generated::*;
