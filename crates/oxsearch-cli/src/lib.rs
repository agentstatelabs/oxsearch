//! The library behind the `oxsearch` command: export an index to NDJSON,
//! import NDJSON into an index, and transfer documents between indices or
//! clusters.
//!
//! An export writes one [`Record`] per line, `{"_index", "_id", "_routing",
//! "_source"}`, so an import puts each document back where it was, with its
//! id and routing. With `plain`, lines are the documents themselves, and an
//! import takes each one's id from a field.
//!
//! Reads go over a point in time, in slices read at once; writes go through
//! the bulk loader, several batches at a time with backoff. Either way the
//! documents pass through as the JSON they are, never parsed into a
//! structure and written again. A [`Report`] says what was read, skipped,
//! written and refused: nothing a document fails on is passed over.

mod connect;
mod error;
mod export;
mod import;
mod record;
mod report;
mod transfer;

pub use connect::{connect, redacted};
pub use error::Error;
pub use export::{ExportOptions, Output, export};
pub use import::{ImportOptions, Input, import};
pub use record::{Decision, Record, Sample, plain};
pub use report::{Failure, Report};
pub use transfer::{Stopped, TransferOptions, transfer};

/// How often a long run logs its progress.
const PROGRESS: std::time::Duration = std::time::Duration::from_secs(10);
