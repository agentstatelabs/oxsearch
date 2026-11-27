//! Documents copied from one index or cluster to another, and the end of a
//! run.

use std::fmt;
use std::time::Instant;

use futures_util::StreamExt;
use oxsearch::helpers::{Action, BulkLoader, BulkResults, Scan};
use oxsearch::{Client, Transport};
use serde_json::Value;
use serde_json::value::RawValue;
use tokio::task::JoinHandle;

use crate::export::query;
use crate::record::{Decision, Sample};
use crate::report::Report;
use crate::{Error, PROGRESS};

/// What a transfer reads and where it writes.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct TransferOptions {
    /// The index, alias or pattern to read.
    pub index: String,
    /// The index to write. Without one, each document goes to an index of
    /// the name it was read from.
    pub to_index: Option<String>,
    /// Only the documents this query matches, as JSON.
    pub query: Option<Value>,
    /// How many slices of the point in time to read at once.
    pub slices: u32,
    /// How many documents each page holds.
    pub page_size: i32,
    /// How long the point in time stays open between pages.
    pub keep_alive: String,
    /// The most documents in a bulk request.
    pub batch_size: usize,
    /// How many bulk requests are in flight at once.
    pub concurrency: usize,
    /// Keeps one document in this many.
    pub every: u64,
    /// Keeps at most this many.
    pub limit: Option<u64>,
}

impl TransferOptions {
    /// Every document of `index`, to indices of the same names.
    pub fn new(index: impl Into<String>) -> TransferOptions {
        TransferOptions {
            index: index.into(),
            to_index: None,
            query: None,
            slices: 1,
            page_size: 1000,
            keep_alive: "1m".to_owned(),
            batch_size: 1000,
            concurrency: 2,
            every: 1,
            limit: None,
        }
    }
}

/// Why a run stopped, and what it had done by then.
#[derive(Debug)]
pub struct Stopped {
    /// Why.
    pub error: Error,
    /// What it had done: documents written before the stop stay written.
    pub report: Report,
}

impl fmt::Display for Stopped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.error)
    }
}

impl std::error::Error for Stopped {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// Copies documents from one cluster's index to another's, or within one
/// cluster, with their ids and routing: read over a point in time, written
/// through the bulk loader.
pub async fn transfer<A, B>(
    from: &Client<A>,
    to: &Client<B>,
    options: &TransferOptions,
) -> Result<Report, Stopped>
where
    A: Transport + Clone + 'static,
    B: Transport + Clone + 'static,
{
    let query = query(options.query.as_ref()).map_err(|error| Stopped {
        error,
        report: Report::default(),
    })?;
    let (sender, results) = BulkLoader::new(to)
        .max_actions(options.batch_size)
        .concurrency(options.concurrency)
        .start();
    let mut scan = Scan::new(options.index.as_str())
        .slices(options.slices)
        .page_size(options.page_size)
        .keep_alive(options.keep_alive.as_str());
    if let Some(query) = query {
        scan = scan.query(query);
    }
    let hits = scan.stream::<Box<RawValue>, A>(from);
    let (every, limit, to_index) = (options.every, options.limit, options.to_index.clone());
    let feed = tokio::spawn(async move {
        let mut hits = std::pin::pin!(hits);
        let mut sample = Sample::new(every, limit);
        let mut fed = Fed::default();
        let mut logged = Instant::now();
        while let Some(hit) = hits.next().await {
            let hit = match hit {
                Ok(hit) => hit,
                Err(error) => return fed.stopped(error.into()),
            };
            match sample.decide() {
                Decision::Stop => break,
                Decision::Skip => {
                    fed.read += 1;
                    fed.skipped += 1;
                    continue;
                }
                Decision::Keep => fed.read += 1,
            }
            let index = to_index.clone().or(hit.index).unwrap_or_default();
            let Some(source) = hit.source else {
                return fed.stopped(Error::Usage(format!(
                    "{index}/{} has no _source to copy: the index doesn't keep it",
                    hit.id.unwrap_or_default()
                )));
            };
            let mut action = Action::index(source).in_index(index);
            if let Some(id) = hit.id {
                action = action.id(id);
            }
            if let Some(routing) = hit.routing {
                action = action.routing(routing);
            }
            // The loader stopped: its last result says why.
            if sender.send(action).await.is_err() {
                break;
            }
            if logged.elapsed() >= PROGRESS {
                tracing::info!(read = fed.read, "transferring");
                logged = Instant::now();
            }
        }
        fed
    });
    collect(results, feed, "transfer").await
}

/// What the side that reads documents did: how many it read and passed
/// over, and why it stopped early, if it did.
#[derive(Debug, Default)]
pub(crate) struct Fed {
    pub read: u64,
    pub skipped: u64,
    pub error: Option<Error>,
}

impl Fed {
    pub(crate) fn stopped(mut self, error: Error) -> Fed {
        self.error = Some(error);
        self
    }
}

/// Reads the loader's results while `feed` sends it documents, and ends
/// the run: the first error, the feed's or the loader's, stops it.
pub(crate) async fn collect(
    mut results: BulkResults,
    feed: JoinHandle<Fed>,
    operation: &'static str,
) -> Result<Report, Stopped> {
    let mut report = Report::default();
    let mut fatal = None;
    let mut logged = Instant::now();
    while let Some(outcome) = results.next().await {
        match outcome {
            Ok(outcome) => report.outcome(outcome, operation),
            Err(error) => fatal = Some(Error::from(error)),
        }
        if logged.elapsed() >= PROGRESS {
            tracing::info!(
                written = report.written,
                refused = report.failed,
                "{operation}ing"
            );
            logged = Instant::now();
        }
    }
    let fed = feed.await.unwrap_or_else(|e| {
        Fed::default().stopped(Error::Usage(format!("the reader stopped: {e}")))
    });
    report.read = fed.read;
    report.skipped = fed.skipped;
    // The loader's error first: the reader stops because of it.
    fatal = fatal.or(fed.error);
    match fatal {
        Some(error) => Err(Stopped { error, report }),
        None => Ok(report),
    }
}
