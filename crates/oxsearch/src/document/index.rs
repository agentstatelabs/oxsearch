//! An index of one document type.

use std::fmt;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use oxsearch_core::__private::encode;
use oxsearch_core::{Client, Error, Method, Refresh, RetrieverContainer, Transport};
use oxsearch_docs::{delete, get, index, mget};
use oxsearch_search::fusion::{Fused, Fusion};
use oxsearch_search::search::{self, Hit};
use serde_json::{Map, Value, json};

use super::Document;

/// Indexes of document types, from a client: `client.index::<Article>()`.
pub trait Indexes<Tr> {
    /// The index the document type's `#[index("…")]` names. A type without
    /// one doesn't compile here; use [`Indexes::index_named`].
    fn index<T: Document>(&self) -> Index<T, Tr>;

    /// An index of the document type, by name.
    fn index_named<T: Document>(&self, name: impl Into<String>) -> Index<T, Tr>;
}

impl<Tr: Transport + Clone> Indexes<Tr> for Client<Tr> {
    fn index<T: Document>(&self) -> Index<T, Tr> {
        const {
            assert!(
                T::INDEX.is_some(),
                "the document type has no #[index(\"…\")]; use index_named"
            )
        };
        Index::new(self.clone(), T::INDEX.unwrap_or_default())
    }

    fn index_named<T: Document>(&self, name: impl Into<String>) -> Index<T, Tr> {
        Index::new(self.clone(), name)
    }
}

/// An index, or an alias, holding documents of type `T`.
pub struct Index<T, Tr> {
    client: Client<Tr>,
    name: String,
    refresh: Option<Refresh>,
    document: PhantomData<fn() -> T>,
}

impl<T, Tr: Clone> Clone for Index<T, Tr> {
    fn clone(&self) -> Index<T, Tr> {
        Index {
            client: self.client.clone(),
            name: self.name.clone(),
            refresh: self.refresh.clone(),
            document: PhantomData,
        }
    }
}

impl<T, Tr> fmt::Debug for Index<T, Tr> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Index")
            .field("name", &self.name)
            .field("refresh", &self.refresh)
            .finish_non_exhaustive()
    }
}

/// A document's version, for writes conditional on it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Version {
    /// The sequence number of the document's last write.
    pub seq_no: i64,
    /// The primary term of the document's last write.
    pub primary_term: i64,
}

/// A document with its id and version. It reads as the document: change
/// it, then write it back with [`Index::put_versioned`], which fails with
/// [`Error::Conflict`] if the document changed in the meantime.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Versioned<T> {
    /// The document's id.
    pub id: String,
    /// The version read.
    pub version: Version,
    /// The document.
    pub document: T,
}

impl<T> Deref for Versioned<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.document
    }
}

impl<T> DerefMut for Versioned<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.document
    }
}

/// What a write did.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Written {
    /// The document's id, Elasticsearch's when the document had none.
    pub id: String,
    /// The document's new version.
    pub version: Version,
    /// Whether the write created the document, rather than replaced it.
    pub created: bool,
}

/// How a live index's mapping compares with a document type's.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
#[must_use = "a mapping can drift from the document type; check `drift`"]
pub struct MappingReport {
    /// Whether the index was created, with the document type's mapping.
    pub created: bool,
    /// Fields the live mapping didn't have, which were added to it.
    pub added: Vec<String>,
    /// Fields mapped differently from the document type. Elasticsearch
    /// can't change a field's type in place, so these are left alone: a
    /// change needs a new index and a reindex.
    pub drift: Vec<Drift>,
}

impl MappingReport {
    /// Whether the live mapping now has every field as the document type
    /// maps it.
    pub fn is_current(&self) -> bool {
        self.drift.is_empty()
    }
}

/// A field mapped differently from the document type.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Drift {
    /// The field, with its path through any objects.
    pub field: String,
    /// The document type's mapping of it.
    pub expected: Value,
    /// The live index's.
    pub actual: Value,
}

/// The parameters that make a field what it is, beside its type: a change
/// to one of these is drift too.
const IDENTITY: &[&str] = &["dims", "similarity", "element_type", "inference_id"];

/// Mapped types Elasticsearch leaves out of the `_source` it returns unless
/// the index says otherwise.
const VECTORS: &[&str] = &["dense_vector", "sparse_vector", "rank_vectors"];

impl<T: Document, Tr: Transport> Index<T, Tr> {
    /// An index of documents of type `T`, by name.
    pub fn new(client: Client<Tr>, name: impl Into<String>) -> Index<T, Tr> {
        Index {
            client,
            name: name.into(),
            refresh: None,
            document: PhantomData,
        }
    }

    /// The index's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The client.
    pub fn client(&self) -> &Client<Tr> {
        &self.client
    }

    /// Writes refresh the index this way, to make them visible to searches
    /// sooner: `Refresh::WaitFor` waits for the next refresh. By default
    /// they don't.
    pub fn refresh(mut self, refresh: Refresh) -> Index<T, Tr> {
        self.refresh = Some(refresh);
        self
    }

    fn path(&self) -> String {
        format!("/{}", encode(&self.name))
    }

    /// Makes the index's mapping the document type's: creates the index if
    /// it doesn't exist, adds the fields it lacks, and reports fields mapped
    /// differently, which it never changes.
    ///
    /// A new index for a type with vector fields keeps vectors in the
    /// `_source` it returns (`index.mapping.exclude_source_vectors: false`),
    /// so documents read back whole.
    pub async fn ensure_mapping(&self) -> Result<MappingReport, Error> {
        let expected = serde_json::to_value(T::properties()).map_err(Error::Encode)?;
        let expected = expected.as_object().cloned().unwrap_or_default();
        let path = self.path();
        let live = match self
            .client
            .send_raw(Method::Get, &format!("{path}/_mapping"), None)
            .await
        {
            Ok(live) => live,
            Err(Error::Elasticsearch(e)) if e.error_type() == Some("index_not_found_exception") => {
                let mut body = json!({ "mappings": { "properties": expected } });
                if has_vectors(&expected) {
                    body["settings"] = json!({ "index.mapping.exclude_source_vectors": false });
                }
                self.client
                    .send_raw(Method::Put, &path, Some(&body))
                    .await?;
                return Ok(MappingReport {
                    created: true,
                    ..MappingReport::default()
                });
            }
            Err(e) => return Err(e),
        };
        let mut report = MappingReport::default();
        // The mapping is keyed by the index, which an alias names otherwise:
        // the first index's is compared, and every index an alias names gets
        // the missing fields at once.
        let properties = live
            .as_object()
            .and_then(|indices| indices.values().next())
            .and_then(|mapping| mapping.pointer("/mappings/properties"))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let missing = compare("", &expected, &properties, &mut report);
        if !missing.is_empty() {
            self.client
                .send_raw(
                    Method::Put,
                    &format!("{path}/_mapping"),
                    Some(&json!({ "properties": missing })),
                )
                .await?;
        }
        Ok(report)
    }

    /// Writes a document, with the id from its `#[id]` field, replacing any
    /// with the same id.
    pub async fn put(&self, document: &T) -> Result<Written, Error> {
        let mut request = index::Request::new(self.name.as_str()).body(document);
        if let Some(id) = document.id() {
            request = request.id(id);
        }
        self.write(request, document).await
    }

    /// Writes a document back if it hasn't changed since it was read, and
    /// fails with [`Error::Conflict`] if it has.
    pub async fn put_versioned(&self, document: &Versioned<T>) -> Result<Written, Error> {
        let request = index::Request::new(self.name.as_str())
            .id(document.id.as_str())
            .if_seq_no(document.version.seq_no)
            .if_primary_term(document.version.primary_term)
            .body(&document.document);
        self.write(request, &document.document).await
    }

    async fn write(&self, mut request: index::Request<&T>, document: &T) -> Result<Written, Error> {
        if let Some(routing) = document.routing() {
            request = request.routing(routing);
        }
        if let Some(refresh) = &self.refresh {
            request = request.refresh(refresh.clone());
        }
        let response = self.client.send(&request).await?;
        let response = response.0;
        Ok(Written {
            id: response.id.unwrap_or_default(),
            version: Version {
                seq_no: response.seq_no.unwrap_or_default(),
                primary_term: response.primary_term.unwrap_or_default(),
            },
            created: matches!(response.result, Some(oxsearch_core::Result::Created)),
        })
    }

    /// A document by id, or `None` if there's none.
    pub async fn get(&self, id: &str) -> Result<Option<Versioned<T>>, Error> {
        let request = get::Request::new(id, self.name.as_str());
        match self
            .client
            .send_as::<_, get::Response<Value>>(&request)
            .await
        {
            Ok(response) => versioned(response.0),
            Err(Error::Elasticsearch(e))
                if e.status == 404 && e.body.get("found") == Some(&Value::Bool(false)) =>
            {
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    /// Documents by id, in the same order, with `None` for each that
    /// doesn't exist.
    pub async fn get_many<I: Into<String>>(
        &self,
        ids: impl IntoIterator<Item = I>,
    ) -> Result<Vec<Option<Versioned<T>>>, Error> {
        let ids: Vec<String> = ids.into_iter().map(Into::into).collect();
        let request = mget::Request::new()
            .index(self.name.as_str())
            .body(mget::RequestBody::new().ids(ids));
        let response: mget::Response<Value> = self.client.send_as(&request).await?;
        response
            .docs
            .into_iter()
            .map(|item| match item {
                mget::ResponseItem::Result(result) => versioned(result),
                mget::ResponseItem::Failure(failure) => {
                    let body = json!({ "error": failure.error });
                    Err(Error::from_response(500, body.to_string().as_bytes()))
                }
                // Responses gain no other kind of item.
                _ => Ok(None),
            })
            .collect()
    }

    /// Deletes a document by id; `false` if there was none.
    pub async fn delete(&self, id: &str) -> Result<bool, Error> {
        let mut request = delete::Request::new(id, self.name.as_str());
        if let Some(refresh) = &self.refresh {
            request = request.refresh(refresh.clone());
        }
        match self.client.send(&request).await {
            Ok(_) => Ok(true),
            Err(Error::Elasticsearch(e))
                if e.status == 404 && e.body.get("result") == Some(&json!("not_found")) =>
            {
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }

    /// The best ten documents a retriever finds. A query is a retriever
    /// too: `Retriever::standard(Article::TITLE.r#match("hnsw"))`.
    pub async fn search(
        &self,
        retriever: impl Into<RetrieverContainer>,
    ) -> Result<Vec<Hit<T>>, Error> {
        let body = search::RequestBody::new().retriever(retriever);
        Ok(self.search_request(body).await?.into_results())
    }

    /// Any search of the index, with documents read as `T`.
    pub async fn search_request(
        &self,
        body: search::RequestBody,
    ) -> Result<search::Response<T>, Error> {
        let request = search::Request::new().index(self.name.as_str()).body(body);
        self.client.send_as(&request).await
    }

    /// Fuses retrievers' results, on the server if its license allows,
    /// else on the client.
    pub async fn fuse(&self, fusion: &Fusion) -> Result<Fused<T>, Error> {
        fusion.search(&self.client, self.name.as_str()).await
    }
}

/// A document read with its version, or `None` if it wasn't found.
fn versioned<T: Document>(result: get::GetResult<Value>) -> Result<Option<Versioned<T>>, Error> {
    if result.found == Some(false) {
        return Ok(None);
    }
    let source = result.source.unwrap_or(Value::Null);
    let document = serde_json::from_value(source.clone()).map_err(|e| {
        let body = source.to_string();
        Error::Decode {
            status: 200,
            source: e,
            body: body.chars().take(512).collect(),
        }
    })?;
    Ok(Some(Versioned {
        id: result.id.unwrap_or_default(),
        version: Version {
            seq_no: result.seq_no.unwrap_or_default(),
            primary_term: result.primary_term.unwrap_or_default(),
        },
        document,
    }))
}

fn has_vectors(properties: &Map<String, Value>) -> bool {
    properties.values().any(|property| {
        property
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| VECTORS.contains(&kind))
            || property
                .get("properties")
                .and_then(Value::as_object)
                .is_some_and(has_vectors)
    })
}

/// A property's type: an object's may be left out.
fn kind(property: &Value) -> &str {
    property
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("object")
}

/// Compares the document type's properties with the live mapping's at one
/// level, recording drift and the fields the live mapping lacks, and
/// returns the properties to add there.
fn compare(
    path: &str,
    expected: &Map<String, Value>,
    live: &Map<String, Value>,
    report: &mut MappingReport,
) -> Map<String, Value> {
    let mut add = Map::new();
    for (name, wanted) in expected {
        let field = if path.is_empty() {
            name.clone()
        } else {
            format!("{path}.{name}")
        };
        let Some(actual) = live.get(name) else {
            report.added.push(field);
            add.insert(name.clone(), wanted.clone());
            continue;
        };
        let differs = kind(wanted) != kind(actual)
            || IDENTITY
                .iter()
                .any(|key| wanted.get(key).is_some_and(|v| actual.get(key) != Some(v)));
        if differs {
            report.drift.push(Drift {
                field,
                expected: wanted.clone(),
                actual: actual.clone(),
            });
            continue;
        }
        if let Some(fields) = wanted.get("properties").and_then(Value::as_object) {
            let empty = Map::new();
            let live_fields = actual
                .get("properties")
                .and_then(Value::as_object)
                .unwrap_or(&empty);
            let missing = compare(&field, fields, live_fields, report);
            if !missing.is_empty() {
                add.insert(name.clone(), json!({ "properties": missing }));
            }
        }
    }
    add
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn drift_and_missing_fields() {
        let expected = map(json!({
            "title": {"type": "text"},
            "tags": {"type": "keyword"},
            "embedding": {"type": "dense_vector", "dims": 3, "similarity": "cosine"},
            "author": {"type": "object", "properties": {
                "name": {"type": "keyword"},
                "email": {"type": "keyword"}
            }}
        }));
        let live = map(json!({
            "title": {"type": "text"},
            "tags": {"type": "text"},
            "embedding": {"type": "dense_vector", "dims": 4, "similarity": "cosine", "index": true},
            "author": {"properties": {"name": {"type": "keyword"}}},
            "extra": {"type": "long"}
        }));
        let mut report = MappingReport::default();
        let add = compare("", &expected, &live, &mut report);
        assert_eq!(report.added, ["author.email"]);
        let drifted: Vec<&str> = report.drift.iter().map(|d| d.field.as_str()).collect();
        assert_eq!(drifted, ["embedding", "tags"]);
        assert!(!report.is_current());
        assert_eq!(
            Value::Object(add),
            json!({"author": {"properties": {"email": {"type": "keyword"}}}})
        );
        assert!(has_vectors(&expected));
        assert!(!has_vectors(&map(json!({"author": expected["author"]}))));
    }
}
