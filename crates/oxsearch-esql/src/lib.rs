//! ES|QL, generated from the Elasticsearch specification: queries, async
//! queries, and their results; with a typed query builder, [`Esql`], and
//! results read into your own types by column name
//! ([`Esql::fetch`], [`EsqlResult::rows`]).
mod builder;
mod generated;
mod rows;

pub use builder::{Column, Esql, Expr, Fuse, Order, Param, col};
pub use generated::*;
pub use rows::read_rows;
