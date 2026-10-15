//! Loads documents with bulk requests: batched, several in flight, with
//! backpressure and backoff.

use std::fmt;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_core::Stream;
use oxsearch_core::__private::encode as percent_encode;
use oxsearch_core::{Body, Client, Endpoint, Error, ErrorCause, Method, Refresh, Route, Transport};
use oxsearch_docs::bulk::{self, OperationType};
use serde::Serialize;
use serde_json::{Map, Value};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::{Instant, sleep, sleep_until};

/// Loads documents with bulk requests. Start it, send it actions, drop the
/// sender when done, and read each action's outcome:
///
/// ```ignore
/// let (sender, results) = BulkLoader::new(&client).index("articles").start();
/// let feed = tokio::spawn(async move {
///     for article in articles {
///         sender.send(Action::index(article)).await?;
///     }
///     Ok::<_, SendError>(())
/// });
/// let summary = results.summary().await?;
/// println!("{} loaded, {} failed", summary.succeeded, summary.failed.len());
/// ```
///
/// - **Backpressure.** Actions wait in bounded channels: `send` waits while
///   the loader is behind, and the loader waits while the results aren't
///   read.
/// - **Batches** are sent when they reach `max_actions` actions or
///   `max_bytes` bytes, or `flush_interval` after their first action, with
///   at most `concurrency` in flight.
/// - **Backoff.** A batch Elasticsearch refuses with 429, or the actions in
///   it refused with 429, are sent again after a growing delay, up to
///   `max_retries` times.
/// - **Results** come in the order batches complete, one per action. An
///   action Elasticsearch refused, such as a document that doesn't fit the
///   mapping, is an outcome with an error. A failure of a whole batch, such
///   as a transport error, is the last result: the loader stops, and
///   `send` fails from then on.
///
/// It runs on the Tokio runtime `start` is called from, and records
/// metrics through the `metrics` facade: `oxsearch.bulk.queued` (actions
/// waiting) and `oxsearch.bulk.in_flight` (batches) as gauges, and
/// `oxsearch.bulk.actions` (by `result`, `ok` or `failed`),
/// `oxsearch.bulk.bytes` and `oxsearch.bulk.requests` as counters, whose
/// rates are documents and bytes per second.
#[derive(Clone, Debug)]
pub struct BulkLoader<Tr> {
    client: Client<Tr>,
    index: Option<String>,
    refresh: Option<Refresh>,
    max_actions: usize,
    max_bytes: usize,
    flush_interval: Duration,
    concurrency: usize,
    max_retries: u32,
    initial_backoff: Duration,
    max_backoff: Duration,
}

impl<Tr: Transport + Clone + 'static> BulkLoader<Tr> {
    /// A loader with the defaults: batches of 1,000 actions or 5 MiB,
    /// flushed after a second, two in flight, five retries with backoff from
    /// 100 ms to 10 s.
    pub fn new(client: &Client<Tr>) -> BulkLoader<Tr> {
        BulkLoader {
            client: client.clone(),
            index: None,
            refresh: None,
            max_actions: 1000,
            max_bytes: 5 * 1024 * 1024,
            flush_interval: Duration::from_secs(1),
            concurrency: 2,
            max_retries: 5,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(10),
        }
    }

    /// The index for actions that don't name one.
    pub fn index(mut self, index: impl Into<String>) -> BulkLoader<Tr> {
        self.index = Some(index.into());
        self
    }

    /// Each batch refreshes the indices it wrote this way.
    pub fn refresh(mut self, refresh: Refresh) -> BulkLoader<Tr> {
        self.refresh = Some(refresh);
        self
    }

    /// The most actions in a batch.
    pub fn max_actions(mut self, max_actions: usize) -> BulkLoader<Tr> {
        self.max_actions = max_actions.max(1);
        self
    }

    /// The most bytes in a batch: a batch is sent once it reaches this.
    pub fn max_bytes(mut self, max_bytes: usize) -> BulkLoader<Tr> {
        self.max_bytes = max_bytes.max(1);
        self
    }

    /// How long a batch waits for more actions after its first.
    pub fn flush_interval(mut self, flush_interval: Duration) -> BulkLoader<Tr> {
        self.flush_interval = flush_interval;
        self
    }

    /// The most batches in flight at once.
    pub fn concurrency(mut self, concurrency: usize) -> BulkLoader<Tr> {
        self.concurrency = concurrency.max(1);
        self
    }

    /// How many times a batch, or its actions, refused with 429 are sent
    /// again, and the first and longest delays before they are.
    pub fn retries(
        mut self,
        max_retries: u32,
        initial_backoff: Duration,
        max_backoff: Duration,
    ) -> BulkLoader<Tr> {
        self.max_retries = max_retries;
        self.initial_backoff = initial_backoff;
        self.max_backoff = max_backoff.max(initial_backoff);
        self
    }

    /// Starts loading: the sender takes actions, and the results give each
    /// one's outcome. Call it from a Tokio runtime.
    pub fn start(self) -> (BulkSender, BulkResults) {
        let (actions, input) = mpsc::channel(self.max_actions);
        let (output, results) = mpsc::channel(self.max_actions);
        tokio::spawn(self.run(input, output));
        (BulkSender { actions }, BulkResults { results })
    }

    async fn run(
        self,
        mut input: mpsc::Receiver<Vec<u8>>,
        output: mpsc::Sender<Result<Outcome, Error>>,
    ) {
        let mut batch: Vec<Line> = Vec::new();
        let mut bytes = 0;
        let mut position = 0;
        let mut deadline: Option<Instant> = None;
        let mut open = true;
        let mut in_flight: JoinSet<Result<Vec<Outcome>, Error>> = JoinSet::new();
        loop {
            let full = batch.len() >= self.max_actions || bytes >= self.max_bytes;
            let due = deadline.is_some_and(|d| Instant::now() >= d);
            let room = in_flight.len() < self.concurrency;
            if !batch.is_empty() && (full || due || !open) && room {
                let lines = std::mem::take(&mut batch);
                metrics::counter!("oxsearch.bulk.bytes").increment(bytes as u64);
                metrics::counter!("oxsearch.bulk.requests").increment(1);
                bytes = 0;
                deadline = None;
                in_flight.spawn(self.clone().send(lines));
                continue;
            }
            metrics::gauge!("oxsearch.bulk.in_flight").set(in_flight.len() as f64);
            metrics::gauge!("oxsearch.bulk.queued").set((batch.len() + input.len()) as f64);
            if !open && batch.is_empty() && in_flight.is_empty() {
                return;
            }
            tokio::select! {
                action = input.recv(), if open && !full => match action {
                    Some(action) => {
                        if batch.is_empty() {
                            deadline = Some(Instant::now() + self.flush_interval);
                        }
                        bytes += action.len();
                        batch.push(Line { position, bytes: action });
                        position += 1;
                    }
                    None => open = false,
                },
                () = sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() && room => {}
                Some(done) = in_flight.join_next() => {
                    let outcomes = match done {
                        Ok(outcomes) => outcomes,
                        Err(join) => Err(Error::Transport(Box::new(join))),
                    };
                    match outcomes {
                        Ok(outcomes) => {
                            for outcome in outcomes {
                                let result = if outcome.is_ok() { "ok" } else { "failed" };
                                metrics::counter!("oxsearch.bulk.actions", "result" => result).increment(1);
                                // Nobody reads the results any more: stop.
                                if output.send(Ok(outcome)).await.is_err() {
                                    return;
                                }
                            }
                        }
                        Err(error) => {
                            // The last result. Returning drops the input,
                            // so senders fail, and aborts what's in flight.
                            let _ = output.send(Err(error)).await;
                            return;
                        }
                    }
                }
                else => return,
            }
        }
    }

    /// Sends one batch, again for what Elasticsearch refuses with 429.
    async fn send(self, mut lines: Vec<Line>) -> Result<Vec<Outcome>, Error> {
        let mut outcomes = Vec::with_capacity(lines.len());
        let mut backoff = self.initial_backoff;
        let mut attempt = 0;
        loop {
            let request = Batch {
                index: self.index.as_deref(),
                refresh: self.refresh.as_ref(),
                body: lines
                    .iter()
                    .flat_map(|line| line.bytes.iter().copied())
                    .collect(),
            };
            match self.client.send(&request).await {
                Ok(response) => {
                    if response.items.len() != lines.len() {
                        return Err(Error::Parameter(format!(
                            "Elasticsearch answered {} actions with {} items",
                            lines.len(),
                            response.items.len()
                        )));
                    }
                    let mut again = Vec::new();
                    for (line, item) in lines.into_iter().zip(response.items) {
                        let (action, item) = (item.key, item.value);
                        let status = item.status.and_then(|s| u16::try_from(s).ok()).unwrap_or(0);
                        if status == 429 && attempt < self.max_retries {
                            again.push(line);
                            continue;
                        }
                        outcomes.push(Outcome {
                            position: line.position,
                            action,
                            index: item.index,
                            id: item.id,
                            status,
                            result: item.result,
                            error: item.error,
                        });
                    }
                    if again.is_empty() {
                        return Ok(outcomes);
                    }
                    lines = again;
                }
                // Elasticsearch is overloaded: the whole batch goes again.
                Err(Error::Elasticsearch(e)) if e.status == 429 && attempt < self.max_retries => {}
                Err(e) => return Err(e),
            }
            attempt += 1;
            sleep(backoff).await;
            backoff = (backoff * 2).min(self.max_backoff);
        }
    }
}

/// An action as it goes on the wire, numbered in the order the loader
/// received it.
struct Line {
    position: u64,
    bytes: Vec<u8>,
}

/// A bulk request of actions already written as NDJSON.
struct Batch<'a> {
    index: Option<&'a str>,
    refresh: Option<&'a Refresh>,
    body: Vec<u8>,
}

impl Endpoint for Batch<'_> {
    type Response = bulk::Response;

    fn name(&self) -> &'static str {
        "bulk"
    }

    fn route(&self) -> Result<Route, Error> {
        Ok(match self.index {
            Some(index) => Route::new(Method::Post, format!("/{}/_bulk", percent_encode(index))),
            None => Route::new(Method::Post, "/_bulk"),
        })
    }

    fn query(&self) -> Result<Vec<(&'static str, String)>, Error> {
        Ok(self
            .refresh
            .map(|refresh| vec![("refresh", refresh.as_str().to_owned())])
            .unwrap_or_default())
    }

    fn body(&self) -> Result<Option<Body>, Error> {
        Ok(Some(Body::Ndjson(self.body.clone())))
    }
}

/// One action for a [`BulkLoader`].
#[derive(Clone, Debug)]
pub struct Action<T> {
    kind: &'static str,
    index: Option<String>,
    id: Option<String>,
    routing: Option<String>,
    document: Option<T>,
}

impl<T> Action<T> {
    /// Writes a document, replacing any with the same id.
    pub fn index(document: T) -> Action<T> {
        Action::new("index", Some(document))
    }

    /// Writes a document that mustn't exist yet: one with the same id is an
    /// outcome with an error.
    pub fn create(document: T) -> Action<T> {
        Action::new("create", Some(document))
    }

    /// Deletes a document.
    pub fn delete(id: impl Into<String>) -> Action<T> {
        Action::new("delete", None).id(id)
    }

    fn new(kind: &'static str, document: Option<T>) -> Action<T> {
        Action {
            kind,
            index: None,
            id: None,
            routing: None,
            document,
        }
    }

    /// The document's id.
    pub fn id(mut self, id: impl Into<String>) -> Action<T> {
        self.id = Some(id.into());
        self
    }

    /// The document's routing.
    pub fn routing(mut self, routing: impl Into<String>) -> Action<T> {
        self.routing = Some(routing.into());
        self
    }

    /// The index, instead of the loader's.
    pub fn in_index(mut self, index: impl Into<String>) -> Action<T> {
        self.index = Some(index.into());
        self
    }
}

#[cfg(feature = "mapping")]
impl<T: crate::document::Document> Action<T> {
    /// Writes a document, with the id and routing from its `#[id]` and
    /// `#[routing]` fields, and into its `#[index]` if it names one.
    pub fn put(document: T) -> Action<T> {
        let mut action = Action::index(document);
        let document = action
            .document
            .as_ref()
            .expect("an index action has a document");
        action.id = document.id();
        action.routing = document.routing();
        action.index = T::INDEX.map(str::to_owned);
        action
    }
}

impl<T: Serialize> Action<T> {
    /// The action and its document as NDJSON lines.
    fn encode(&self) -> Result<Vec<u8>, serde_json::Error> {
        let mut meta = Map::new();
        for (key, value) in [
            ("_index", &self.index),
            ("_id", &self.id),
            ("routing", &self.routing),
        ] {
            if let Some(value) = value {
                meta.insert(key.to_owned(), Value::String(value.clone()));
            }
        }
        let mut line = Map::new();
        line.insert(self.kind.to_owned(), Value::Object(meta));
        let mut bytes = serde_json::to_vec(&line)?;
        bytes.push(b'\n');
        if let Some(document) = &self.document {
            serde_json::to_writer(&mut bytes, document)?;
            bytes.push(b'\n');
        }
        Ok(bytes)
    }
}

/// Sends actions to a [`BulkLoader`]. Clone it to send from several tasks;
/// the loader finishes once every sender is dropped.
#[derive(Clone, Debug)]
pub struct BulkSender {
    actions: mpsc::Sender<Vec<u8>>,
}

impl BulkSender {
    /// Sends an action, waiting while the loader is behind.
    pub async fn send<T: Serialize>(&self, action: Action<T>) -> Result<(), SendError> {
        let bytes = action.encode().map_err(SendError::Encode)?;
        self.actions
            .send(bytes)
            .await
            .map_err(|_| SendError::Stopped)
    }
}

/// Why an action wasn't sent.
#[derive(Debug)]
#[non_exhaustive]
pub enum SendError {
    /// The document couldn't be written as JSON.
    Encode(serde_json::Error),
    /// The loader stopped: its last result says why.
    Stopped,
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendError::Encode(e) => write!(f, "writing the document: {e}"),
            SendError::Stopped => f.write_str("the bulk loader stopped; its last result says why"),
        }
    }
}

impl std::error::Error for SendError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SendError::Encode(e) => Some(e),
            SendError::Stopped => None,
        }
    }
}

/// What happened to one action.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Outcome {
    /// The action's place in the order the loader received actions, from 0.
    pub position: u64,
    /// What the action was.
    pub action: OperationType,
    /// The index it wrote.
    pub index: Option<String>,
    /// The document's id.
    pub id: Option<String>,
    /// The action's status code.
    pub status: u16,
    /// What it did: `created`, `updated`, `deleted`, `not_found` or `noop`.
    pub result: Option<String>,
    /// Why Elasticsearch refused it.
    pub error: Option<ErrorCause>,
}

impl Outcome {
    /// Whether the action succeeded.
    pub fn is_ok(&self) -> bool {
        self.error.is_none() && (200..300).contains(&self.status)
    }
}

/// The outcomes of a [`BulkLoader`]'s actions, in the order batches
/// complete; a stream, or read with [`BulkResults::next`].
#[derive(Debug)]
pub struct BulkResults {
    results: mpsc::Receiver<Result<Outcome, Error>>,
}

/// How a load went.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Summary {
    /// How many actions succeeded.
    pub succeeded: u64,
    /// The actions that didn't.
    pub failed: Vec<Outcome>,
}

impl BulkResults {
    /// The next outcome, or `None` once the loader is done.
    pub async fn next(&mut self) -> Option<Result<Outcome, Error>> {
        self.results.recv().await
    }

    /// Reads every outcome: how many succeeded and which failed, or the
    /// error that stopped the loader.
    pub async fn summary(mut self) -> Result<Summary, Error> {
        let mut summary = Summary::default();
        while let Some(outcome) = self.next().await {
            let outcome = outcome?;
            if outcome.is_ok() {
                summary.succeeded += 1;
            } else {
                summary.failed.push(outcome);
            }
        }
        Ok(summary)
    }
}

impl Stream for BulkResults {
    type Item = Result<Outcome, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.results.poll_recv(cx)
    }
}
