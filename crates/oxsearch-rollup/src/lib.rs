//! Rollups, generated from the Elasticsearch specification: rollup jobs that
//! summarize time series into a rollup index, their capabilities, and search
//! across rollup and live indices. Elasticsearch deprecates rollups in
//! favour of downsampling, and a cluster can only create rollup jobs while
//! it already has a rollup index.
mod generated;
pub use generated::*;
