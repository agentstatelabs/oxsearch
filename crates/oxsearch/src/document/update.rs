//! Changes to part of a document: some of its fields, or a script.

use std::fmt;

use oxsearch_core::{Result as Outcome, Script};
use serde::Serialize;
use serde_json::{Map, Value};

use super::{Field, Version};

/// New values for some of a document's fields; the others stay as they are.
///
/// ```ignore
/// articles
///     .update("1", Patch::new().set(Article::VIEWS, 41).set(Article::DRAFT, false))
///     .await?;
/// ```
///
/// A value replaces the field's, except that an object's fields are merged
/// into the ones it has. A patch can't remove a field; a script can.
#[derive(Debug, Default)]
pub struct Patch {
    fields: Map<String, Value>,
    /// The first value that couldn't be written as JSON.
    error: Option<serde_json::Error>,
}

impl Patch {
    /// A patch that changes nothing yet.
    pub fn new() -> Patch {
        Patch::default()
    }

    /// Gives a field a new value. An object's field is set inside it:
    /// `set(Article::AUTHOR.field(Author::NAME), "Ada")`.
    pub fn set<K>(mut self, field: Field<K>, value: impl Serialize) -> Patch {
        match serde_json::to_value(value) {
            Ok(value) => insert(&mut self.fields, field.name(), value),
            Err(error) => {
                self.error.get_or_insert(error);
            }
        }
        self
    }

    /// Gives fields new values from a struct or a map that holds some of
    /// the document's.
    pub fn merge(mut self, fields: impl Serialize) -> Patch {
        match serde_json::to_value(fields) {
            Ok(Value::Object(fields)) => self.fields.extend(fields),
            Ok(_) => {
                self.error
                    .get_or_insert(serde::ser::Error::custom("a patch is an object of fields"));
            }
            Err(error) => {
                self.error.get_or_insert(error);
            }
        }
        self
    }
}

/// Sets a value at a dotted path, making the objects on the way: a partial
/// document's `author.name` would be a field of that name.
fn insert(fields: &mut Map<String, Value>, path: &str, value: Value) {
    let Some((object, rest)) = path.split_once('.') else {
        fields.insert(path.to_owned(), value);
        return;
    };
    let inner = fields
        .entry(object)
        .or_insert_with(|| Value::Object(Map::new()));
    if !inner.is_object() {
        *inner = Value::Object(Map::new());
    }
    if let Value::Object(inner) = inner {
        insert(inner, rest, value);
    }
}

/// How an update changes the document.
enum Kind {
    Patch(Patch),
    Script(Box<Script>),
}

/// A change to a document, from a [`Patch`] or a [`Script`], with what to do
/// when the document isn't there or changes meanwhile.
///
/// ```ignore
/// let visit = Script::inline("ctx._source.views += params.by").param("by", 1);
/// articles
///     .update("1", Update::from(visit).upsert(first_visit).retry_on_conflict(3))
///     .await?;
/// ```
pub struct Update<T> {
    kind: Kind,
    pub(super) upsert: Option<T>,
    pub(super) retry_on_conflict: Option<u32>,
    pub(super) version: Option<Version>,
}

impl<T> Update<T> {
    fn new(kind: Kind) -> Update<T> {
        Update {
            kind,
            upsert: None,
            retry_on_conflict: None,
            version: None,
        }
    }

    /// Writes this document if there's none to change.
    pub fn upsert(mut self, document: T) -> Update<T> {
        self.upsert = Some(document);
        self
    }

    /// Tries again this many times when the document changes between the
    /// update's read and its write.
    pub fn retry_on_conflict(mut self, times: u32) -> Update<T> {
        self.retry_on_conflict = Some(times);
        self
    }

    /// Changes the document only if this is its version, and otherwise
    /// fails with [`Error::Conflict`](oxsearch_core::Error::Conflict).
    pub fn if_version(mut self, version: Version) -> Update<T> {
        self.version = Some(version);
        self
    }
}

impl<T: Serialize> Update<T> {
    /// The update's body.
    pub(super) fn body(&self) -> Result<Value, serde_json::Error> {
        let mut body = Map::new();
        match &self.kind {
            Kind::Patch(patch) => {
                if let Some(error) = &patch.error {
                    return Err(serde::ser::Error::custom(error));
                }
                body.insert("doc".to_owned(), Value::Object(patch.fields.clone()));
            }
            Kind::Script(script) => {
                body.insert("script".to_owned(), serde_json::to_value(script)?);
            }
        }
        if let Some(document) = &self.upsert {
            body.insert("upsert".to_owned(), serde_json::to_value(document)?);
        }
        Ok(Value::Object(body))
    }
}

impl<T> From<Patch> for Update<T> {
    fn from(patch: Patch) -> Update<T> {
        Update::new(Kind::Patch(patch))
    }
}

impl<T> From<Script> for Update<T> {
    fn from(script: Script) -> Update<T> {
        Update::new(Kind::Script(Box::new(script)))
    }
}

impl<T> fmt::Debug for Update<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("Update");
        match &self.kind {
            Kind::Patch(patch) => debug.field("patch", patch),
            Kind::Script(script) => debug.field("script", script),
        };
        debug
            .field("upsert", &self.upsert.is_some())
            .field("retry_on_conflict", &self.retry_on_conflict)
            .field("version", &self.version)
            .finish()
    }
}

/// What an update, or one action of a bulk write, did.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Applied {
    /// The document's id.
    pub id: String,
    /// The document's version afterwards.
    pub version: Version,
    /// What changed.
    pub change: Change,
}

/// How a write changed a document.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Change {
    /// The document is new.
    Created,
    /// The document was replaced or changed.
    Updated,
    /// The document is gone.
    Deleted,
    /// The document already was as the update would make it, or the script
    /// left it alone.
    Unchanged,
    /// There was no such document to change or delete.
    NotFound,
    /// Something this version doesn't know, as Elasticsearch names it.
    Other(String),
}

impl From<Outcome> for Change {
    fn from(outcome: Outcome) -> Change {
        match outcome {
            Outcome::Created => Change::Created,
            Outcome::Updated => Change::Updated,
            Outcome::Deleted => Change::Deleted,
            Outcome::NoOp => Change::Unchanged,
            Outcome::NotFound => Change::NotFound,
            other => Change::Other(other.as_str().to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::document::kind;

    const VIEWS: Field<kind::Number> = Field::new("views");
    const NAME: Field<kind::Keyword> = Field::new("author.name");
    const CITY: Field<kind::Keyword> = Field::new("author.address.city");

    #[test]
    fn patches_nest_dotted_fields() {
        let patch = Patch::new()
            .set(VIEWS, 41)
            .set(NAME, "Ada")
            .set(CITY, "London")
            .merge(json!({"draft": false}));
        assert_eq!(
            Update::<Value>::from(patch).body().unwrap(),
            json!({"doc": {
                "views": 41,
                "author": {"name": "Ada", "address": {"city": "London"}},
                "draft": false
            }})
        );
    }

    #[test]
    fn scripts_and_upserts() {
        let update = Update::from(Script::inline("ctx._source.views++"))
            .upsert(json!({"views": 1}))
            .body()
            .unwrap();
        assert_eq!(update["upsert"], json!({"views": 1}));
        assert!(update.get("doc").is_none());
    }

    #[test]
    fn values_that_arent_json_fail_when_sent() {
        let patch = Patch::new().merge(5).set(VIEWS, 1);
        let error = Update::<Value>::from(patch).body().unwrap_err();
        assert!(error.to_string().contains("object of fields"), "{error}");
    }
}
