//! A [`rig`](https://docs.rs/rig-core) vector store on Elasticsearch,
//! through oxsearch.
//!
//! ```ignore
//! use oxsearch_rig::ElasticsearchIndex;
//!
//! // Nearest neighbours of the query, embedded by a rig embedding model.
//! let index = ElasticsearchIndex::knn(client.clone(), "passages", "embedding", model);
//! // Or a semantic_text field, which Elasticsearch embeds itself.
//! let index = ElasticsearchIndex::semantic(client.clone(), "passages", "text");
//!
//! let agent = openai.agent("gpt-5").dynamic_context(3, index).build();
//! ```
//!
//! Either can be made hybrid with [`ElasticsearchIndex::hybrid`]: the
//! vector or semantic search fused by reciprocal rank with a keyword match,
//! on the server when the cluster's license includes the `rrf` retriever,
//! else on the client.
//!
//! Filters ([`Filter`]) become Elasticsearch queries: `eq` a `term` (or
//! `terms` for a list), `gt` and `lt` a `range`, `and` and `or` a `bool`.
//! A request's threshold is a minimum `_score`, in Elasticsearch's scale:
//! `(1 + cosine) / 2` for kNN on a cosine field.

use oxsearch_core::query_dsl::{
    BoolQuery, DateRangeQuery, NumberRangeQuery, QueryContainer, TermsQuery, TermsQueryField,
};
use oxsearch_core::{Client, FieldValue, Query, Refresh, Retriever, RetrieverContainer, Transport};
use oxsearch_docs::bulk;
use oxsearch_search::fusion::Fusion;
use oxsearch_search::search::{self, Hit};
use rig_core::DynModel;
use rig_core::embeddings::{Embed, Embedding};
use rig_core::operation::Embedding as EmbeddingOp;
use rig_core::vector_store::request::{SearchFilter, VectorSearchRequest};
use rig_core::vector_store::{
    InsertDocuments, VectorSearchIdResult, VectorSearchResult, VectorStoreError, VectorStoreIndex,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Elasticsearch returns at most this many hits.
const MAX_RESULTS: u64 = 10_000;

/// How the query finds documents.
#[derive(Clone)]
enum Search {
    /// The `k` nearest neighbours of the query, embedded by `model`.
    Knn {
        field: String,
        model: DynModel<EmbeddingOp>,
    },
    /// A `semantic` query on a `semantic_text` field.
    Semantic { field: String },
}

/// A rig vector store index on an Elasticsearch index.
#[derive(Clone)]
pub struct ElasticsearchIndex<Tr> {
    client: Client<Tr>,
    index: String,
    search: Search,
    /// A text field to match the query against too, fused by RRF.
    hybrid: Option<String>,
    num_candidates: Option<u64>,
    refresh: Option<Refresh>,
}

impl<Tr> std::fmt::Debug for ElasticsearchIndex<Tr> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let search = match &self.search {
            Search::Knn { field, .. } => format!("knn on {field}"),
            Search::Semantic { field } => format!("semantic on {field}"),
        };
        f.debug_struct("ElasticsearchIndex")
            .field("index", &self.index)
            .field("search", &search)
            .field("hybrid", &self.hybrid)
            .finish_non_exhaustive()
    }
}

impl<Tr: Transport + Clone + 'static> ElasticsearchIndex<Tr> {
    /// Nearest neighbours in the `dense_vector` field `field`, of the query
    /// embedded by `model`, which must be the model that embedded the
    /// documents.
    pub fn knn(
        client: Client<Tr>,
        index: impl Into<String>,
        field: impl Into<String>,
        model: impl Into<DynModel<EmbeddingOp>>,
    ) -> ElasticsearchIndex<Tr> {
        ElasticsearchIndex::new(
            client,
            index,
            Search::Knn {
                field: field.into(),
                model: model.into(),
            },
        )
    }

    /// A `semantic` query on the `semantic_text` field `field`:
    /// Elasticsearch embeds the query with the field's inference endpoint,
    /// so no model is needed here.
    pub fn semantic(
        client: Client<Tr>,
        index: impl Into<String>,
        field: impl Into<String>,
    ) -> ElasticsearchIndex<Tr> {
        ElasticsearchIndex::new(
            client,
            index,
            Search::Semantic {
                field: field.into(),
            },
        )
    }

    fn new(client: Client<Tr>, index: impl Into<String>, search: Search) -> ElasticsearchIndex<Tr> {
        ElasticsearchIndex {
            client,
            index: index.into(),
            search,
            hybrid: None,
            num_candidates: None,
            refresh: None,
        }
    }

    /// Fuses the search with a `match` of the query on the text field
    /// `text_field`, by reciprocal rank: on the server if the cluster's
    /// license includes it, else on the client.
    pub fn hybrid(mut self, text_field: impl Into<String>) -> ElasticsearchIndex<Tr> {
        self.hybrid = Some(text_field.into());
        self
    }

    /// For kNN, the candidates each shard considers: 100, or the number of
    /// results asked for if more, by default.
    pub fn num_candidates(mut self, num_candidates: u64) -> ElasticsearchIndex<Tr> {
        self.num_candidates = Some(num_candidates);
        self
    }

    /// Inserted documents refresh the index this way, so searches see them
    /// sooner.
    pub fn refresh(mut self, refresh: Refresh) -> ElasticsearchIndex<Tr> {
        self.refresh = Some(refresh);
        self
    }

    /// The Elasticsearch index's name.
    pub fn index(&self) -> &str {
        &self.index
    }

    /// The retriever for a request: the vector or semantic search, filtered,
    /// and the keyword match if hybrid.
    // rig's error type, which the trait's methods return.
    #[allow(clippy::result_large_err)]
    async fn retriever(
        &self,
        req: &VectorSearchRequest<Filter>,
    ) -> Result<(RetrieverContainer, Option<RetrieverContainer>), VectorStoreError> {
        let samples = req.samples();
        if samples > MAX_RESULTS {
            return Err(VectorStoreError::SamplesOutOfRange {
                requested: samples,
                max: MAX_RESULTS,
            });
        }
        let filter = req.filter().map(|f| f.0.clone());
        let main = match &self.search {
            Search::Knn { field, model } => {
                let embedding = model.embed_text(req.query()).await?;
                let vector: Vec<f32> = embedding.vec.iter().map(|x| *x as f32).collect();
                let candidates = self
                    .num_candidates
                    .unwrap_or(100)
                    .max(samples)
                    .min(MAX_RESULTS);
                let mut knn = oxsearch_core::KnnRetriever::new(field.as_str(), samples as i32)
                    .query_vector(vector)
                    .num_candidates(candidates as i32);
                if let Some(filter) = &filter {
                    knn = knn.filter(filter.clone());
                }
                RetrieverContainer::from(knn)
            }
            Search::Semantic { field } => Retriever::standard(filtered(
                Query::semantic(field.as_str(), req.query()),
                &filter,
            )),
        };
        let keywords = self.hybrid.as_ref().map(|text| {
            Retriever::standard(filtered(
                Query::r#match(text.as_str(), req.query()),
                &filter,
            ))
        });
        Ok((main, keywords))
    }

    /// The hits for a request, documents read as `T`.
    #[allow(clippy::result_large_err)]
    async fn hits<T: DeserializeOwned + Send + 'static>(
        &self,
        req: &VectorSearchRequest<Filter>,
        source: bool,
    ) -> Result<Vec<Hit<T>>, VectorStoreError> {
        let (main, keywords) = self.retriever(req).await?;
        let size = req.samples() as i32;
        let mut body = search::RequestBody::new().size(size);
        if !source {
            body = body.source(false);
        }
        let hits = match keywords {
            None => {
                if let Some(threshold) = req.threshold() {
                    body = body.min_score(threshold);
                }
                let request = search::Request::new()
                    .index(self.index.as_str())
                    .body(body.retriever(main));
                let response: search::Response<T> =
                    self.client.send_as(&request).await.map_err(datastore)?;
                response.into_results()
            }
            Some(keywords) => {
                let base = serde_json::to_value(&body).map_err(VectorStoreError::JsonError)?;
                let base = serde_json::from_value(base).map_err(VectorStoreError::JsonError)?;
                let fused = Fusion::rrf([main, keywords])
                    .size(size)
                    .body(base)
                    .search::<T, Tr>(&self.client, self.index.as_str())
                    .await
                    .map_err(datastore)?;
                // RRF scores aren't similarities: a threshold doesn't apply.
                fused.hits
            }
        };
        Ok(hits)
    }
}

impl<Tr: Transport + Clone + 'static> VectorStoreIndex for ElasticsearchIndex<Tr> {
    type Filter = Filter;

    async fn top_n<T: DeserializeOwned + Send>(
        &self,
        req: VectorSearchRequest<Filter>,
    ) -> Result<Vec<VectorSearchResult<T>>, VectorStoreError> {
        let hits = self.hits::<Value>(&req, true).await?;
        let results: Result<Vec<_>, serde_json::Error> = hits
            .into_iter()
            .map(|hit| {
                let document = serde_json::from_value(hit.source.unwrap_or(Value::Null))?;
                Ok(VectorSearchResult {
                    score: hit.score.unwrap_or_default(),
                    id: hit.id.unwrap_or_default(),
                    document,
                })
            })
            .collect();
        Ok(results?)
    }

    async fn top_n_ids(
        &self,
        req: VectorSearchRequest<Filter>,
    ) -> Result<Vec<VectorSearchIdResult>, VectorStoreError> {
        let hits = self.hits::<Value>(&req, false).await?;
        Ok(hits
            .into_iter()
            .map(|hit| VectorSearchIdResult {
                score: hit.score.unwrap_or_default(),
                id: hit.id.unwrap_or_default(),
            })
            .collect())
    }
}

impl<Tr: Transport + Clone + 'static> InsertDocuments for ElasticsearchIndex<Tr> {
    /// Writes each document once per embedding, with the embedding in the
    /// kNN field; for a `semantic_text` field, once, and Elasticsearch
    /// embeds it.
    async fn insert_documents<Doc: Serialize + Embed + Send>(
        &self,
        documents: Vec<(Doc, Vec<Embedding>)>,
    ) -> Result<(), VectorStoreError> {
        let mut lines: Vec<bulk::RequestBody<Value>> = Vec::new();
        for (document, embeddings) in documents {
            let document = serde_json::to_value(&document)?;
            let action = || {
                bulk::RequestBody::OperationContainer(bulk::OperationContainer::Index(
                    bulk::IndexOperation::new(),
                ))
            };
            match &self.search {
                Search::Knn { field, .. } => {
                    for embedding in embeddings {
                        let mut document = document.clone();
                        let Value::Object(fields) = &mut document else {
                            return Err(VectorStoreError::datastore(Mismatch(
                                "a document to insert is a JSON object".to_owned(),
                            )));
                        };
                        fields.insert(field.clone(), serde_json::to_value(&embedding.vec)?);
                        lines.push(action());
                        lines.push(bulk::RequestBody::TDocument(document));
                    }
                }
                Search::Semantic { .. } => {
                    lines.push(action());
                    lines.push(bulk::RequestBody::TDocument(document));
                }
            }
        }
        if lines.is_empty() {
            return Ok(());
        }
        let mut request = bulk::Request::new().index(self.index.as_str()).body(lines);
        if let Some(refresh) = &self.refresh {
            request = request.refresh(refresh.clone());
        }
        let response = self.client.send(&request).await.map_err(datastore)?;
        if response.errors == Some(true) {
            let first = response
                .items
                .iter()
                .find_map(|item| item.value.error.as_ref())
                .and_then(|error| error.reason.clone())
                .unwrap_or_else(|| "a document was refused".to_owned());
            return Err(VectorStoreError::datastore(Mismatch(first)));
        }
        Ok(())
    }
}

/// A rig filter, as an Elasticsearch query.
#[derive(Clone, Debug)]
pub struct Filter(QueryContainer);

impl Filter {
    /// Any query, as a filter.
    pub fn query(query: impl Into<QueryContainer>) -> Filter {
        Filter(query.into())
    }

    fn range(key: &str, value: Value, greater: bool) -> Filter {
        let query = match value {
            Value::Number(n) => {
                let n = n.as_f64().unwrap_or_default();
                let range = NumberRangeQuery::new();
                let range = if greater { range.gt(n) } else { range.lt(n) };
                Query::range(key, range)
            }
            other => {
                let text = match other {
                    Value::String(s) => s,
                    other => other.to_string(),
                };
                let range = DateRangeQuery::new();
                let range = if greater {
                    range.gt(text)
                } else {
                    range.lt(text)
                };
                Query::range(key, range)
            }
        };
        Filter(query)
    }
}

impl From<Filter> for QueryContainer {
    fn from(filter: Filter) -> QueryContainer {
        filter.0
    }
}

impl SearchFilter for Filter {
    type Value = Value;

    fn eq(key: impl AsRef<str>, value: Value) -> Filter {
        let key = key.as_ref();
        Filter(match value {
            Value::Array(values) => {
                let values: Vec<FieldValue> = values.into_iter().map(field_value).collect();
                TermsQuery::new(key, TermsQueryField::Value(values)).into()
            }
            value => Query::term(key, field_value(value)),
        })
    }

    fn gt(key: impl AsRef<str>, value: Value) -> Filter {
        Filter::range(key.as_ref(), value, true)
    }

    fn lt(key: impl AsRef<str>, value: Value) -> Filter {
        Filter::range(key.as_ref(), value, false)
    }

    fn and(self, rhs: Filter) -> Filter {
        Filter(BoolQuery::new().filter(vec![self.0, rhs.0]).into())
    }

    fn or(self, rhs: Filter) -> Filter {
        Filter(
            BoolQuery::new()
                .should(vec![self.0, rhs.0])
                .minimum_should_match(1)
                .into(),
        )
    }
}

/// A JSON value as a term query's value.
fn field_value(value: Value) -> FieldValue {
    match value {
        Value::Bool(b) => FieldValue::from(b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => FieldValue::from(i),
            None => FieldValue::from(n.as_f64().unwrap_or_default()),
        },
        Value::String(s) => FieldValue::from(s),
        other => FieldValue::from(other.to_string()),
    }
}

/// A query with the filter, if any, in filter context.
fn filtered(query: QueryContainer, filter: &Option<QueryContainer>) -> QueryContainer {
    match filter {
        None => query,
        Some(filter) => BoolQuery::new().must(query).filter(filter.clone()).into(),
    }
}

fn datastore(error: oxsearch_core::Error) -> VectorStoreError {
    VectorStoreError::datastore(error)
}

/// What Elasticsearch refused, or a request that doesn't fit it.
#[derive(Debug)]
struct Mismatch(String);

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Mismatch {}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn json(filter: Filter) -> Value {
        serde_json::to_value(QueryContainer::from(filter)).unwrap()
    }

    #[test]
    fn filters_become_queries() {
        assert_eq!(
            json(Filter::eq("topic", json!("vectors"))),
            json!({"term": {"topic": "vectors"}})
        );
        assert_eq!(
            json(Filter::eq("tags", json!(["a", "b"]))),
            json!({"terms": {"tags": ["a", "b"]}})
        );
        assert_eq!(
            json(Filter::gt("year", json!(2020))),
            json!({"range": {"year": {"gt": 2020.0}}})
        );
        assert_eq!(
            json(Filter::lt("published", json!("now-1y"))),
            json!({"range": {"published": {"lt": "now-1y"}}})
        );
        assert_eq!(
            json(Filter::eq("a", json!(1)).and(Filter::eq("b", json!(true)))),
            json!({"bool": {"filter": [{"term": {"a": 1}}, {"term": {"b": true}}]}})
        );
        assert_eq!(
            json(Filter::eq("a", json!(1)).or(Filter::eq("b", json!(2)))),
            json!({"bool": {"should": [{"term": {"a": 1}}, {"term": {"b": 2}}], "minimum_should_match": 1}})
        );
    }
}
