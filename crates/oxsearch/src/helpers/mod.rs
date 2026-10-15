//! Helpers for reading and loading a lot of documents: [`Scan`] streams
//! every document a query matches over a point in time, and
//! [`BulkLoader`] loads documents with bulk requests.
//!
//! Both pass data through channels and streams rather than shared state:
//! the end of a stream is a channel closing, errors travel with the data
//! as `Result`s, and work is handled in the order it completes.

mod bulk;
mod scan;

pub use bulk::{Action, BulkLoader, BulkResults, BulkSender, Outcome, SendError, Summary};
pub use scan::Scan;
