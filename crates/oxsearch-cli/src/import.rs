//! NDJSON, written into an index.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::PathBuf;
use std::time::Instant;

use flate2::read::MultiGzDecoder;
use oxsearch::helpers::{Action, BulkLoader, BulkSender};
use oxsearch::{Client, Transport};
use serde_json::value::RawValue;

use crate::Error;
use crate::record::{Decision, Record, Sample, plain};
use crate::report::Report;
use crate::transfer::{Fed, Stopped, collect};

/// How an import reads and writes.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ImportOptions {
    /// The index to write. Without one, each record goes back to its own.
    pub index: Option<String>,
    /// The lines are documents themselves, not records.
    pub plain: bool,
    /// For plain documents, the field that holds each one's id. Without
    /// one, Elasticsearch gives each an id.
    pub id_field: Option<String>,
    /// The most documents in a bulk request.
    pub batch_size: usize,
    /// How many bulk requests are in flight at once.
    pub concurrency: usize,
    /// Keeps one document in this many.
    pub every: u64,
    /// Keeps at most this many.
    pub limit: Option<u64>,
}

impl ImportOptions {
    /// Records, each back into its own index, two batches of 1,000 at a
    /// time.
    pub fn new() -> ImportOptions {
        ImportOptions {
            index: None,
            plain: false,
            id_field: None,
            batch_size: 1000,
            concurrency: 2,
            every: 1,
            limit: None,
        }
    }

    fn check(&self) -> Result<(), Error> {
        if self.plain && self.index.is_none() {
            return Err(Error::Usage(
                "plain documents need an index to go into".to_owned(),
            ));
        }
        if self.id_field.is_some() && !self.plain {
            return Err(Error::Usage(
                "an id field is for plain documents; records carry their ids".to_owned(),
            ));
        }
        Ok(())
    }
}

impl Default for ImportOptions {
    fn default() -> ImportOptions {
        ImportOptions::new()
    }
}

/// Where an import reads, compressed with gzip or not: it can tell.
#[derive(Clone, Debug)]
pub struct Input {
    path: Option<PathBuf>,
}

impl Input {
    /// Standard input.
    pub fn stdin() -> Input {
        Input { path: None }
    }

    /// A file.
    pub fn file(path: impl Into<PathBuf>) -> Input {
        Input {
            path: Some(path.into()),
        }
    }

    fn open(&self) -> Result<Box<dyn BufRead + Send>, Error> {
        let name = || self.path.clone().unwrap_or_else(|| PathBuf::from("-"));
        let raw: Box<dyn Read + Send> = match &self.path {
            None => Box::new(io::stdin()),
            Some(path) => Box::new(File::open(path).map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?),
        };
        let mut raw = BufReader::with_capacity(1 << 20, raw);
        let start = raw.fill_buf().map_err(|source| Error::Io {
            path: name(),
            source,
        })?;
        Ok(if start.starts_with(&[0x1f, 0x8b]) {
            Box::new(BufReader::with_capacity(1 << 20, MultiGzDecoder::new(raw)))
        } else {
            Box::new(raw)
        })
    }
}

/// Writes NDJSON into an index through the bulk loader, and says what
/// was written and what Elasticsearch refused.
///
/// A line that isn't a record, or with `plain` a JSON object, stops the
/// import there: what was sent before it is written.
pub async fn import<Tr: Transport + Clone + 'static>(
    client: &Client<Tr>,
    options: &ImportOptions,
    input: &Input,
) -> Result<Report, Stopped> {
    let stop = |error| Stopped {
        error,
        report: Report::default(),
    };
    options.check().map_err(stop)?;
    let reader = input.open().map_err(stop)?;
    let mut loader = BulkLoader::new(client)
        .max_actions(options.batch_size)
        .concurrency(options.concurrency);
    if let Some(index) = &options.index {
        loader = loader.index(index.as_str());
    }
    let (sender, results) = loader.start();
    let handle = tokio::runtime::Handle::current();
    let options = options.clone();
    let lines = tokio::task::spawn_blocking(move || read(reader, &options, &sender, &handle));
    collect(results, lines, "import").await
}

/// Reads the lines into actions for the loader: how many it read and
/// passed over.
fn read(
    mut reader: Box<dyn BufRead + Send>,
    options: &ImportOptions,
    sender: &BulkSender,
    handle: &tokio::runtime::Handle,
) -> Fed {
    let mut sample = Sample::new(options.every, options.limit);
    let mut text = String::new();
    let mut number = 0;
    let mut fed = Fed::default();
    let mut logged = Instant::now();
    loop {
        text.clear();
        let bytes = match reader.read_line(&mut text) {
            Ok(bytes) => bytes,
            Err(e) => {
                let error = Error::Line {
                    line: number + 1,
                    message: e.to_string(),
                };
                return fed.stopped(error);
            }
        };
        if bytes == 0 {
            return fed;
        }
        number += 1;
        let line = text.trim_end_matches(['\n', '\r']);
        if line.trim().is_empty() {
            continue;
        }
        // Every line is read, kept or not: a malformed one fails the import.
        let action = match action(line, number, options) {
            Ok(action) => action,
            Err(error) => return fed.stopped(error),
        };
        match sample.decide() {
            Decision::Stop => return fed,
            Decision::Skip => {
                fed.read += 1;
                fed.skipped += 1;
                continue;
            }
            Decision::Keep => fed.read += 1,
        }
        // The loader stopped: its last result says why.
        if handle.block_on(sender.send(action)).is_err() {
            return fed;
        }
        if logged.elapsed() >= crate::PROGRESS {
            tracing::info!(line = number, read = fed.read, "importing");
            logged = Instant::now();
        }
    }
}

fn action(
    line: &str,
    number: u64,
    options: &ImportOptions,
) -> Result<Action<Box<RawValue>>, Error> {
    if options.plain {
        let (source, id) = plain(line, number, options.id_field.as_deref())?;
        let mut action = Action::index(source);
        if let Some(id) = id {
            action = action.id(id);
        }
        return Ok(action);
    }
    let record = Record::parse(line, number)?;
    let mut action = Action::index(record.source);
    if let Some(id) = record.id {
        action = action.id(id);
    }
    if let Some(routing) = record.routing {
        action = action.routing(routing);
    }
    if options.index.is_none() {
        let index = record.index.ok_or_else(|| Error::Line {
            line: number,
            message: "no `_index`, and no index to import into".to_owned(),
        })?;
        action = action.in_index(index);
    }
    Ok(action)
}
