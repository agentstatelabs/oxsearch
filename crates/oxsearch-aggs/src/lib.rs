//! Aggregation requests and typed aggregation results, generated from the
//! Elasticsearch specification.
//!
//! [`Agg`] builds the common aggregations and [`Aggs`] names them for a
//! search request; [`Aggregates`] reads the results, typed, from a response
//! to a request sent with `typed_keys`, and [`Results`] picks them out by
//! name:
//!
//! ```ignore
//! use oxsearch::aggs::{Agg, Aggregates, Aggs, Results};
//!
//! let request = search::Request::new().index("products").typed_keys(true).body(
//!     search::RequestBody::new().size(0).aggregations(
//!         Aggs::new().add("by_category", Agg::terms("category", 10).sub("avg_price", Agg::avg("price"))),
//!     ),
//! );
//! let response: search::Response<Product, Aggregates> = client.send_as(&request).await?;
//! let results = response.aggregations.as_ref().unwrap();
//! for bucket in results.string_terms("by_category") {
//!     println!("{:?}: {:?}", bucket.key, bucket.aggregations.value("avg_price"));
//! }
//! ```

mod builders;
mod generated;
mod results;

pub use builders::{Agg, Aggs};
pub use generated::*;
pub use results::{Aggregates, Results, buckets};
