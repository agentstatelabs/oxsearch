//! An index's documents, written to NDJSON.

use std::fs::OpenOptions;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use flate2::Compression;
use flate2::write::GzEncoder;
use futures_util::StreamExt;
use oxsearch::helpers::Scan;
use oxsearch::oxsearch_core::query_dsl::QueryContainer;
use oxsearch::search::search::Hit;
use oxsearch::{Client, Transport};
use serde_json::Value;
use serde_json::value::RawValue;
use tokio::sync::mpsc;

use crate::record::{Decision, Record, Sample};
use crate::report::Report;
use crate::transfer::Stopped;
use crate::{Error, PROGRESS};

/// What an export reads.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ExportOptions {
    /// The index, alias or pattern to read.
    pub index: String,
    /// Only the documents this query matches, as JSON: `{"term": {…}}`.
    pub query: Option<Value>,
    /// How many slices of the point in time to read at once.
    pub slices: u32,
    /// How many documents each page holds.
    pub page_size: i32,
    /// How long the point in time stays open between pages.
    pub keep_alive: String,
    /// Keeps one document in this many.
    pub every: u64,
    /// Keeps at most this many.
    pub limit: Option<u64>,
    /// Writes the documents themselves, not records with their ids.
    pub plain: bool,
}

impl ExportOptions {
    /// Every document of `index`, in records, read in one slice.
    pub fn new(index: impl Into<String>) -> ExportOptions {
        ExportOptions {
            index: index.into(),
            query: None,
            slices: 1,
            page_size: 1000,
            keep_alive: "1m".to_owned(),
            every: 1,
            limit: None,
            plain: false,
        }
    }
}

/// Where an export writes.
#[derive(Clone, Debug)]
pub struct Output {
    path: Option<PathBuf>,
    gzip: bool,
}

impl Output {
    /// Standard output.
    pub fn stdout() -> Output {
        Output {
            path: None,
            gzip: false,
        }
    }

    /// A new file, which mustn't exist yet: an export never overwrites one.
    /// A name ending in `.gz` is compressed.
    pub fn file(path: impl Into<PathBuf>) -> Output {
        let path = path.into();
        let gzip = path.extension().is_some_and(|e| e == "gz");
        Output {
            path: Some(path),
            gzip,
        }
    }

    /// Compresses what's written with gzip, or not.
    pub fn gzip(mut self, gzip: bool) -> Output {
        self.gzip = gzip;
        self
    }

    fn name(&self) -> PathBuf {
        self.path.clone().unwrap_or_else(|| PathBuf::from("-"))
    }

    fn open(&self) -> Result<Sink, Error> {
        let out: Box<dyn Write + Send> = match &self.path {
            None => Box::new(io::stdout()),
            Some(path) => Box::new(create_new(path)?),
        };
        let out = BufWriter::with_capacity(1 << 20, out);
        Ok(if self.gzip {
            Sink::Gzip(GzEncoder::new(out, Compression::default()))
        } else {
            Sink::Plain(out)
        })
    }
}

fn create_new(path: &Path) -> Result<std::fs::File, Error> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| Error::Io {
            path: path.to_owned(),
            source: if source.kind() == io::ErrorKind::AlreadyExists {
                io::Error::new(
                    source.kind(),
                    "exists already; an export never overwrites a file",
                )
            } else {
                source
            },
        })
}

/// The file or stream lines go to, from one thread: lines are never
/// interleaved.
enum Sink {
    Plain(BufWriter<Box<dyn Write + Send>>),
    Gzip(GzEncoder<BufWriter<Box<dyn Write + Send>>>),
}

impl Sink {
    fn drain(mut self, mut lines: mpsc::Receiver<Vec<u8>>) -> io::Result<()> {
        while let Some(line) = lines.blocking_recv() {
            match &mut self {
                Sink::Plain(out) => out.write_all(&line)?,
                Sink::Gzip(out) => out.write_all(&line)?,
            }
        }
        match self {
            Sink::Plain(mut out) => out.flush(),
            Sink::Gzip(out) => out.finish()?.flush(),
        }
    }
}

/// A hit as its line.
pub(crate) fn line(hit: Hit<Box<RawValue>>, plain: bool) -> Result<Vec<u8>, Error> {
    let Some(source) = hit.source else {
        return Err(Error::Usage(format!(
            "{}/{} has no _source to copy: the index doesn't keep it",
            hit.index.unwrap_or_default(),
            hit.id.unwrap_or_default()
        )));
    };
    let mut line = if plain {
        source.get().as_bytes().to_vec()
    } else {
        let record = Record {
            index: hit.index,
            id: hit.id,
            routing: hit.routing,
            source,
        };
        serde_json::to_vec(&record).map_err(|e| Error::from(oxsearch::Error::Encode(e)))?
    };
    line.push(b'\n');
    Ok(line)
}

/// The query, read from its JSON.
pub(crate) fn query(query: Option<&Value>) -> Result<Option<QueryContainer>, Error> {
    query
        .map(|q| serde_json::from_value(q.clone()))
        .transpose()
        .map_err(|e| Error::Usage(format!("the query isn't one: {e}")))
}

/// Writes the documents of an index to NDJSON, and says how many.
///
/// The point in time is read in slices at once, and each page is refused
/// if a shard couldn't answer it. With a limit, the read stops early and
/// leaves the point in time to expire after its keep-alive.
pub async fn export<Tr: Transport + Clone + 'static>(
    client: &Client<Tr>,
    options: &ExportOptions,
    output: &Output,
) -> Result<Report, Stopped> {
    let mut report = Report::default();
    let stop = |error: Error, report: &Report| Stopped {
        error,
        report: report.clone(),
    };
    let query = query(options.query.as_ref()).map_err(|e| stop(e, &report))?;
    let sink = output.open().map_err(|e| stop(e, &report))?;
    let (lines, received) = mpsc::channel::<Vec<u8>>(1024);
    let writer = std::thread::spawn(move || sink.drain(received));

    let mut scan = Scan::new(options.index.as_str())
        .slices(options.slices)
        .page_size(options.page_size)
        .keep_alive(options.keep_alive.as_str());
    if let Some(query) = query {
        scan = scan.query(query);
    }
    let hits = scan.stream::<Box<RawValue>, Tr>(client);
    let mut hits = std::pin::pin!(hits);
    let mut sample = Sample::new(options.every, options.limit);
    let mut failure = None;
    let mut logged = Instant::now();
    while let Some(hit) = hits.next().await {
        let hit = match hit {
            Ok(hit) => hit,
            Err(e) => {
                failure = Some(Error::from(e));
                break;
            }
        };
        match sample.decide() {
            Decision::Stop => break,
            Decision::Skip => {
                report.read += 1;
                report.skipped += 1;
                continue;
            }
            Decision::Keep => report.read += 1,
        }
        let line = match line(hit, options.plain) {
            Ok(line) => line,
            Err(e) => {
                failure = Some(e);
                break;
            }
        };
        // The writer stopped: its error says why.
        if lines.send(line).await.is_err() {
            break;
        }
        report.written += 1;
        metrics::counter!("oxsearch.cli.documents", "operation" => "export", "result" => "written")
            .increment(1);
        if logged.elapsed() >= PROGRESS {
            tracing::info!(read = report.read, written = report.written, "exporting");
            logged = Instant::now();
        }
    }
    drop(lines);
    let written = writer
        .join()
        .unwrap_or_else(|_| Err(io::Error::other("the writer panicked")));
    if let Err(source) = written {
        let error = Error::Io {
            path: output.name(),
            source,
        };
        return Err(stop(error, &report));
    }
    match failure {
        Some(error) => Err(stop(error, &report)),
        None => Ok(report),
    }
}

#[cfg(test)]
mod tests {
    use oxsearch::search::search;

    use super::*;

    #[test]
    fn hits_keep_their_source_as_read() {
        let response = r#"{"took": 1, "timed_out": false, "pit_id": "p",
            "hits": {"hits": [{"_index": "notes", "_id": "1", "_routing": "ada",
                "_score": null, "_source": {"price": 1.10, "tags": ["a"]}, "sort": [3]}]}}"#;
        let response: search::Response<Box<RawValue>> = serde_json::from_str(response).unwrap();
        let hit = response.into_results().remove(0);
        let line = String::from_utf8(super::line(hit.clone(), false).unwrap()).unwrap();
        assert_eq!(
            line,
            "{\"_index\":\"notes\",\"_id\":\"1\",\"_routing\":\"ada\",\"_source\":{\"price\": 1.10, \"tags\": [\"a\"]}}\n"
        );
        let plain = String::from_utf8(super::line(hit, true).unwrap()).unwrap();
        assert_eq!(plain, "{\"price\": 1.10, \"tags\": [\"a\"]}\n");
    }

    #[test]
    fn an_export_never_overwrites_a_file() {
        let path = std::env::temp_dir().join(format!("oxsearch-cli-{}.ndjson", std::process::id()));
        std::fs::write(&path, b"kept").unwrap();
        let error = Output::file(&path).open().err().expect("a refusal");
        assert!(error.to_string().contains("never overwrites"), "{error}");
        assert_eq!(std::fs::read(&path).unwrap(), b"kept");
        std::fs::remove_file(&path).unwrap();
        assert!(Output::file("x.ndjson.gz").gzip);
    }
}
