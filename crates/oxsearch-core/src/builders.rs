//! Builders for the common queries and retrievers: one call each, over the
//! generated types.
//!
//! ```
//! use oxsearch_core::{Query, Retriever};
//!
//! let hybrid = Retriever::rrf([
//!     Retriever::standard(Query::r#match("title", "tuning hnsw")),
//!     Retriever::knn("embedding", vec![0.1, 0.7, 0.2], 10, 100),
//! ]);
//! ```
//!
//! For an option a builder doesn't take, build the generated type and
//! convert it: `KnnRetriever::new("embedding", 10).similarity(0.8).into()`.

use serde_json::Value;

use crate::query_dsl::{
    BoolQuery, ExistsQuery, IdsQuery, IntervalsMatch, IntervalsQuery, IntervalsQueryVariant,
    MatchAllQuery, MatchPhraseQuery, MatchQuery, MatchQueryQuery, MultiMatchQuery, NestedQuery,
    QueryContainer, RangeQuery, SemanticQuery, SparseVectorQuery, SparseVectorQueryVariant,
    TermQuery, TermsQuery, TermsQueryField,
};
use crate::search::{Rescore, RescoreQuery, RescoreVariant};
use crate::shapes::{IndexMap, SingleKey};
use crate::{
    DiversifyRetriever, DiversifyRetrieverTypes, FieldValue, InnerRetriever, KnnQuery,
    KnnRetriever, LinearRetriever, PinnedRetriever, RRFRetriever, RRFRetrieverEntry,
    RescorerRetriever, RetrieverContainer, RuleRetriever, StandardRetriever,
    TextSimilarityReranker,
};

/// Builders for queries. Each returns a [`QueryContainer`], except
/// [`Query::bool`], whose clauses you add.
pub struct Query;

impl Query {
    /// Every document.
    pub fn match_all() -> QueryContainer {
        MatchAllQuery::new().into()
    }

    /// Documents whose `field` matches `text`, analyzed: `{"match": …}`.
    pub fn r#match(field: impl Into<String>, text: impl Into<MatchQueryQuery>) -> QueryContainer {
        keyed(field, MatchQuery::new(text)).into()
    }

    /// Documents whose `field` contains `text` as a phrase.
    pub fn match_phrase(field: impl Into<String>, text: impl Into<String>) -> QueryContainer {
        keyed(field, MatchPhraseQuery::new(text)).into()
    }

    /// Documents where any of `fields` matches `text`.
    pub fn multi_match<F: Into<String>>(
        fields: impl IntoIterator<Item = F>,
        text: impl Into<String>,
    ) -> QueryContainer {
        let fields: Vec<String> = fields.into_iter().map(Into::into).collect();
        MultiMatchQuery::new(text).fields(fields).into()
    }

    /// Documents whose `field` is exactly `value`, not analyzed.
    pub fn term(field: impl Into<String>, value: impl Into<FieldValue>) -> QueryContainer {
        keyed(field, TermQuery::new(value)).into()
    }

    /// Documents whose `field` is exactly one of `values`.
    pub fn terms<V: Into<FieldValue>>(
        field: impl Into<String>,
        values: impl IntoIterator<Item = V>,
    ) -> QueryContainer {
        let values: Vec<FieldValue> = values.into_iter().map(Into::into).collect();
        TermsQuery::new(field, TermsQueryField::Value(values)).into()
    }

    /// Documents whose `field` is in a range: build the range with the
    /// generated types, such as `NumberRangeQuery::new().gte(10.0)`.
    pub fn range(field: impl Into<String>, range: impl Into<RangeQuery>) -> QueryContainer {
        keyed(field, range.into()).into()
    }

    /// Documents that have a value in `field`.
    pub fn exists(field: impl Into<String>) -> QueryContainer {
        ExistsQuery::new(field).into()
    }

    /// Documents with these ids.
    pub fn ids<I: Into<String>>(ids: impl IntoIterator<Item = I>) -> QueryContainer {
        let ids: Vec<String> = ids.into_iter().map(Into::into).collect();
        IdsQuery::new().values(ids).into()
    }

    /// A boolean combination: add `must`, `should`, `filter` and `must_not`
    /// clauses, then convert it with `.into()`.
    pub fn bool() -> BoolQuery {
        BoolQuery::new()
    }

    /// Queries on nested objects at `path`.
    pub fn nested(path: impl Into<String>, query: impl Into<QueryContainer>) -> QueryContainer {
        NestedQuery::new(path, query).into()
    }

    /// The `k` nearest neighbours of `vector` in a `dense_vector` field.
    pub fn knn(field: impl Into<String>, vector: Vec<f32>, k: i32) -> QueryContainer {
        KnnQuery::new(field).query_vector(vector).k(k).into()
    }

    /// Documents whose `semantic_text` field means something close to
    /// `text`. The field's inference endpoint embeds `text`; Elastic's
    /// default, ELSER, needs a license above Basic.
    pub fn semantic(field: impl Into<String>, text: impl Into<String>) -> QueryContainer {
        SemanticQuery::new(field, text).into()
    }

    /// Documents whose `sparse_vector` field scores well against these token
    /// weights.
    pub fn sparse_vector<T: Into<String>>(
        field: impl Into<String>,
        weights: impl IntoIterator<Item = (T, f32)>,
    ) -> QueryContainer {
        let weights: IndexMap<String, f32> =
            weights.into_iter().map(|(t, w)| (t.into(), w)).collect();
        SparseVectorQuery::new(field)
            .variant(SparseVectorQueryVariant::QueryVector(weights))
            .into()
    }

    /// Documents whose `sparse_vector` field scores well against `text`,
    /// expanded by the inference endpoint `inference_id`.
    pub fn sparse_vector_text(
        field: impl Into<String>,
        inference_id: impl Into<String>,
        text: impl Into<String>,
    ) -> QueryContainer {
        SparseVectorQuery::new(field)
            .variant(SparseVectorQueryVariant::InferenceId(inference_id.into()))
            .query(text)
            .into()
    }

    /// Documents where `text`'s terms appear in `field` in order and close
    /// together: an `intervals` query with one `match` rule.
    pub fn intervals(field: impl Into<String>, text: impl Into<String>) -> QueryContainer {
        let rule =
            IntervalsQuery::new().variant(IntervalsQueryVariant::Match(IntervalsMatch::new(text)));
        keyed(field, rule).into()
    }
}

/// A query on one field: `{"<field>": value}`.
fn keyed<V>(field: impl Into<String>, value: V) -> SingleKey<String, V> {
    SingleKey::new(field, value)
}

/// A query is a standard retriever: wherever a retriever goes, a query can.
impl From<QueryContainer> for RetrieverContainer {
    fn from(query: QueryContainer) -> RetrieverContainer {
        Retriever::standard(query)
    }
}

/// Builders for retrievers. Each returns a [`RetrieverContainer`], and
/// those that combine retrievers take queries too.
pub struct Retriever;

impl Retriever {
    /// The documents a query finds, by its score.
    pub fn standard(query: impl Into<QueryContainer>) -> RetrieverContainer {
        StandardRetriever::new().query(query).into()
    }

    /// The `k` nearest neighbours of `vector`, from `num_candidates` per
    /// shard.
    pub fn knn(
        field: impl Into<String>,
        vector: Vec<f32>,
        k: i32,
        num_candidates: i32,
    ) -> RetrieverContainer {
        KnnRetriever::new(field, k)
            .query_vector(vector)
            .num_candidates(num_candidates)
            .into()
    }

    /// Reciprocal rank fusion of the retrievers' results, or queries'. Needs
    /// a license above Basic on the server; see `oxsearch::fusion` to fuse
    /// on the client instead.
    pub fn rrf<R: Into<RetrieverContainer>>(
        retrievers: impl IntoIterator<Item = R>,
    ) -> RetrieverContainer {
        let entries: Vec<RRFRetrieverEntry> = retrievers
            .into_iter()
            .map(|retriever| retriever.into().into())
            .collect();
        RRFRetriever::new(entries).into()
    }

    /// A weighted sum of the retrievers' scores. Needs a license above
    /// Basic on the server.
    pub fn linear<R: Into<RetrieverContainer>>(
        retrievers: impl IntoIterator<Item = (R, f32)>,
    ) -> RetrieverContainer {
        let inner: Vec<InnerRetriever> = retrievers
            .into_iter()
            .map(|(retriever, weight)| InnerRetriever::new(retriever.into()).weight(weight))
            .collect();
        LinearRetriever::new().retrievers(inner).into()
    }

    /// `retriever`'s results with these documents first.
    pub fn pinned<I: Into<String>>(
        retriever: impl Into<RetrieverContainer>,
        ids: impl IntoIterator<Item = I>,
    ) -> RetrieverContainer {
        let ids: Vec<String> = ids.into_iter().map(Into::into).collect();
        PinnedRetriever::new(retriever.into()).ids(ids).into()
    }

    /// `retriever`'s top `window_size` results, rescored by `query`.
    pub fn rescorer(
        retriever: impl Into<RetrieverContainer>,
        query: impl Into<QueryContainer>,
        window_size: i32,
    ) -> RetrieverContainer {
        let rescore = Rescore::new()
            .window_size(window_size)
            .variant(RescoreVariant::Query(RescoreQuery::new(query)));
        RescorerRetriever::new(retriever.into(), rescore).into()
    }

    /// `retriever`'s results, reranked by how well `field` answers `text`,
    /// with a reranking model. Needs a license above Basic.
    pub fn rerank(
        retriever: impl Into<RetrieverContainer>,
        field: impl Into<String>,
        text: impl Into<String>,
    ) -> RetrieverContainer {
        TextSimilarityReranker::new(retriever.into(), text, field).into()
    }

    /// `retriever`'s results with the query rules in `ruleset` applied, for
    /// the criteria in `match_criteria`. Needs a license above Basic.
    pub fn rule(
        retriever: impl Into<RetrieverContainer>,
        ruleset: impl Into<String>,
        match_criteria: Value,
    ) -> RetrieverContainer {
        RuleRetriever::new(ruleset.into(), match_criteria, retriever.into()).into()
    }

    /// `retriever`'s results, made diverse by maximal marginal relevance on
    /// a `dense_vector` field, relative to `vector`.
    pub fn diversify(
        retriever: impl Into<RetrieverContainer>,
        field: impl Into<String>,
        vector: Vec<f32>,
    ) -> RetrieverContainer {
        DiversifyRetriever::new(DiversifyRetrieverTypes::Mmr, field, retriever.into())
            .query_vector(vector)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn json(value: &impl serde::Serialize) -> Value {
        serde_json::to_value(value).unwrap()
    }

    #[test]
    fn queries() {
        assert_eq!(
            json(&Query::r#match("title", "hnsw")),
            json!({"match": {"title": "hnsw"}})
        );
        assert_eq!(
            json(&Query::term("tag", "vector")),
            json!({"term": {"tag": "vector"}})
        );
        assert_eq!(
            json(&Query::terms("tag", ["a", "b"])),
            json!({"terms": {"tag": ["a", "b"]}})
        );
        assert_eq!(
            json(
                &Query::bool()
                    .must(Query::r#match("title", "hnsw"))
                    .filter(Query::term("tag", "vector"))
            ),
            json!({"must": {"match": {"title": "hnsw"}}, "filter": {"term": {"tag": "vector"}}})
        );
        assert_eq!(
            json(&Query::sparse_vector("terms", [("hnsw", 1.0)])),
            json!({"sparse_vector": {"field": "terms", "query_vector": {"hnsw": 1.0}}})
        );
        assert_eq!(
            json(&Query::intervals("title", "bulk loading")),
            json!({"intervals": {"title": {"match": {"query": "bulk loading"}}}})
        );
    }

    #[test]
    fn retrievers() {
        let hybrid = Retriever::rrf([
            Retriever::standard(Query::r#match("title", "hnsw")),
            Retriever::knn("embedding", vec![1.0, 0.0], 5, 50),
        ]);
        assert_eq!(
            json(&hybrid),
            json!({"rrf": {"retrievers": [
                {"standard": {"query": {"match": {"title": "hnsw"}}}},
                {"knn": {"field": "embedding", "k": 5, "query_vector": [1.0, 0.0], "num_candidates": 50}}
            ]}})
        );
        let linear = Retriever::linear([(Query::match_all(), 2.0)]);
        assert_eq!(
            json(&linear),
            json!({"linear": {"retrievers": [{"retriever": {"standard": {"query": {"match_all": {}}}}, "weight": 2.0}]}})
        );
    }
}
