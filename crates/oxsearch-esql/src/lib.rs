//! ES|QL, generated from the Elasticsearch specification: queries, async
//! queries, and their results; with a typed query builder, [`Esql`], and
//! results read into your own types by column name
//! ([`Esql::fetch`], [`EsqlResult::rows`]), or with the `arrow` feature as
//! Arrow record batches (`Esql::fetch_arrow`).
#[cfg(feature = "arrow")]
mod arrow;
mod builder;
mod generated;
mod rows;

#[cfg(feature = "arrow")]
pub use arrow::read_arrow;
/// The Arrow arrays [`Esql::fetch_arrow`] returns, as the version it uses.
#[cfg(feature = "arrow")]
pub use arrow_array;
pub use builder::{Column, Esql, Expr, Fuse, Order, Param, col};
pub use generated::*;
pub use rows::read_rows;
