//! Several writes to an index in one request.

use std::fmt;

use oxsearch_core::{Error, Transport};
use oxsearch_docs::bulk::ResponseItem;
use serde::Serialize;
use serde_json::{Map, Value, json};

use super::update::{Applied, Change, Update};
use super::{Document, Index, Version, Versioned};
use crate::batch::Batch;

/// Several writes to an index, sent in one request, from [`Index::bulk`].
/// Each succeeds or fails on its own.
///
/// ```ignore
/// let report = articles
///     .bulk()
///     .put(&new)
///     .put_versioned(&edited)
///     .update("7", Patch::new().set(Article::DRAFT, false))
///     .delete("9")
///     .send()
///     .await?;
/// for (position, error) in report.failures() {
///     eprintln!("write {position}: {error}");
/// }
/// ```
///
/// Elasticsearch writes the same document's actions in order. A lot of
/// documents load faster, with batches, backoff and backpressure, through
/// `helpers::BulkLoader`.
#[must_use = "a bulk write does nothing until it's sent"]
pub struct Bulk<'a, T, Tr> {
    index: &'a Index<T, Tr>,
    body: Vec<u8>,
    actions: usize,
    /// The first document that couldn't be written as JSON.
    error: Option<serde_json::Error>,
}

impl<T, Tr> fmt::Debug for Bulk<'_, T, Tr> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bulk")
            .field("index", self.index)
            .field("actions", &self.actions)
            .finish_non_exhaustive()
    }
}

impl<'a, T: Document, Tr: Transport> Bulk<'a, T, Tr> {
    pub(super) fn new(index: &'a Index<T, Tr>) -> Bulk<'a, T, Tr> {
        Bulk {
            index,
            body: Vec::new(),
            actions: 0,
            error: None,
        }
    }

    /// How many writes there are.
    pub fn len(&self) -> usize {
        self.actions
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.actions == 0
    }

    /// Writes a document, with the id from its `#[id]` field, replacing any
    /// with the same id.
    pub fn put(self, document: &T) -> Self {
        let meta = self.document_meta(document);
        self.add("index", meta, Some(document))
    }

    /// Writes documents, as [`Bulk::put`] does each.
    pub fn put_all<'d>(self, documents: impl IntoIterator<Item = &'d T>) -> Self
    where
        T: 'd,
    {
        documents.into_iter().fold(self, Bulk::put)
    }

    /// Writes a document that mustn't exist yet: if one has its id, the
    /// write fails with [`Error::Conflict`].
    pub fn create(self, document: &T) -> Self {
        let meta = self.document_meta(document);
        self.add("create", meta, Some(document))
    }

    /// Writes a document back if it hasn't changed since it was read: if it
    /// has, the write fails with [`Error::Conflict`].
    pub fn put_versioned(self, document: &Versioned<T>) -> Self {
        let mut meta = self.document_meta(&document.document);
        meta.insert("_id".to_owned(), json!(document.id));
        if document.document.routing().is_none()
            && let Some(routing) = self.index.routing_of(document)
        {
            meta.insert("routing".to_owned(), json!(routing));
        }
        condition(&mut meta, document.version);
        self.add("index", meta, Some(&document.document))
    }

    /// Changes part of a document, as [`Index::update`] does.
    pub fn update(mut self, id: impl Into<String>, update: impl Into<Update<T>>) -> Self {
        let update = update.into();
        let routing = self.index.routing_for(update.upsert.as_ref());
        let mut meta = meta(Some(id.into()), routing);
        if let Some(times) = update.retry_on_conflict {
            meta.insert("retry_on_conflict".to_owned(), json!(times));
        }
        if let Some(version) = update.version {
            condition(&mut meta, version);
        }
        match update.body() {
            Ok(body) => self.add("update", meta, Some(&body)),
            Err(error) => {
                self.error.get_or_insert(error);
                self
            }
        }
    }

    /// Deletes a document.
    pub fn delete(self, id: impl Into<String>) -> Self {
        let meta = meta(Some(id.into()), self.index.routing_for(None));
        self.add::<Value>("delete", meta, None)
    }

    /// Deletes a document if it hasn't changed since it was read: if it
    /// has, the write fails with [`Error::Conflict`].
    pub fn delete_versioned(self, document: &Versioned<T>) -> Self {
        let mut meta = meta(Some(document.id.clone()), self.index.routing_of(document));
        condition(&mut meta, document.version);
        self.add::<Value>("delete", meta, None)
    }

    fn document_meta(&self, document: &T) -> Map<String, Value> {
        meta(document.id(), self.index.routing_for(Some(document)))
    }

    fn add<D: Serialize>(mut self, kind: &str, meta: Map<String, Value>, line: Option<&D>) -> Self {
        let start = self.body.len();
        match encode(&mut self.body, kind, meta, line) {
            Ok(()) => self.actions += 1,
            Err(error) => {
                // Not half an action.
                self.body.truncate(start);
                self.error.get_or_insert(error);
            }
        }
        self
    }

    /// Sends the writes, and says what each did, in order. With no writes,
    /// sends nothing. A document that couldn't be written as JSON fails
    /// before anything is sent.
    pub async fn send(self) -> Result<BulkReport, Error> {
        if let Some(error) = self.error {
            return Err(Error::Encode(error));
        }
        if self.actions == 0 {
            return Ok(BulkReport::default());
        }
        let request = Batch {
            index: Some(self.index.name()),
            refresh: self.index.refresh.as_ref(),
            body: self.body,
        };
        let response = self.index.client().send(&request).await?;
        if response.items.len() != self.actions {
            return Err(Error::Parameter(format!(
                "Elasticsearch answered {} actions with {} items",
                self.actions,
                response.items.len()
            )));
        }
        Ok(BulkReport {
            items: response
                .items
                .into_iter()
                .map(|item| applied(item.value))
                .collect(),
        })
    }
}

fn meta(id: Option<String>, routing: Option<String>) -> Map<String, Value> {
    let mut meta = Map::new();
    for (key, value) in [("_id", id), ("routing", routing)] {
        if let Some(value) = value {
            meta.insert(key.to_owned(), Value::String(value));
        }
    }
    meta
}

fn condition(meta: &mut Map<String, Value>, version: Version) {
    meta.insert("if_seq_no".to_owned(), json!(version.seq_no));
    meta.insert("if_primary_term".to_owned(), json!(version.primary_term));
}

/// Writes an action and its line as NDJSON.
fn encode<D: Serialize>(
    body: &mut Vec<u8>,
    kind: &str,
    meta: Map<String, Value>,
    line: Option<&D>,
) -> Result<(), serde_json::Error> {
    let mut action = Map::new();
    action.insert(kind.to_owned(), Value::Object(meta));
    serde_json::to_writer(&mut *body, &action)?;
    body.push(b'\n');
    if let Some(line) = line {
        serde_json::to_writer(&mut *body, line)?;
        body.push(b'\n');
    }
    Ok(())
}

/// What one action did.
fn applied(item: ResponseItem) -> Result<Applied, Error> {
    let status = item.status.and_then(|s| u16::try_from(s).ok()).unwrap_or(0);
    let change = match (&item.error, item.result) {
        // An update of a document that isn't there.
        (Some(error), _) if error.r#type.as_deref() == Some("document_missing_exception") => {
            Change::NotFound
        }
        (Some(error), _) => {
            let body = json!({"error": error, "status": status});
            return Err(Error::from_response(status, body.to_string().as_bytes()));
        }
        (None, Some(result)) => oxsearch_core::Result::from_known(&result)
            .map_or_else(|| Change::Other(result), Change::from),
        (None, None) => Change::Other(String::new()),
    };
    Ok(Applied {
        id: item.id.unwrap_or_default(),
        version: Version {
            seq_no: item.seq_no.unwrap_or_default(),
            primary_term: item.primary_term.unwrap_or_default(),
        },
        change,
    })
}

/// What each write of a [`Bulk`] did.
#[derive(Debug, Default)]
#[non_exhaustive]
#[must_use = "a write in a bulk can fail on its own; check `failures`"]
pub struct BulkReport {
    /// What each write did, or why it failed, in the order they were added.
    pub items: Vec<Result<Applied, Error>>,
}

impl BulkReport {
    /// Whether every write succeeded.
    pub fn is_ok(&self) -> bool {
        self.items.iter().all(Result::is_ok)
    }

    /// The writes that failed, each with its place in the order they were
    /// added, from 0.
    pub fn failures(&self) -> impl Iterator<Item = (usize, &Error)> {
        self.items
            .iter()
            .enumerate()
            .filter_map(|(position, item)| item.as_ref().err().map(|error| (position, error)))
    }
}
