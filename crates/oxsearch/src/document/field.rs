//! Typed field handles: each offers the queries that suit its field's
//! mapped type.

use std::borrow::Cow;
use std::fmt;
use std::marker::PhantomData;
use std::ops::{Bound, RangeBounds};

use oxsearch_core::query_dsl::{DateRangeQuery, NumberRangeQuery, QueryContainer};
use oxsearch_core::{FieldValue, Query, Retriever, RetrieverContainer};

/// The mapped types a [`Field`] can have, as markers.
pub mod kind {
    /// `text`: analyzed full text.
    #[derive(Debug)]
    pub enum Text {}
    /// `keyword`: exact values.
    #[derive(Debug)]
    pub enum Keyword {}
    /// `semantic_text`: text embedded by an inference endpoint.
    #[derive(Debug)]
    pub enum SemanticText {}
    /// `dense_vector`.
    #[derive(Debug)]
    pub enum DenseVector {}
    /// `sparse_vector`: weights by token.
    #[derive(Debug)]
    pub enum SparseVector {}
    /// A number type.
    #[derive(Debug)]
    pub enum Number {}
    /// `date`.
    #[derive(Debug)]
    pub enum Date {}
    /// `boolean`.
    #[derive(Debug)]
    pub enum Boolean {}
    /// An object, whose fields are its own.
    #[derive(Debug)]
    pub enum Object {}
}

use kind::{Boolean, Date, DenseVector, Keyword, Number, Object, SemanticText, SparseVector, Text};

/// A field of a document type, named as Elasticsearch names it, with the
/// queries that suit its mapped type `K`. `#[derive(Document)]` gives each
/// field one, as a constant: `Article::TITLE`.
pub struct Field<K> {
    name: Cow<'static, str>,
    kind: PhantomData<fn() -> K>,
}

impl<K> Field<K> {
    /// A field by name.
    pub const fn new(name: &'static str) -> Field<K> {
        Field {
            name: Cow::Borrowed(name),
            kind: PhantomData,
        }
    }

    /// The field's name, with its path through any objects.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Documents with a value in the field.
    pub fn exists(&self) -> QueryContainer {
        Query::exists(self.name())
    }
}

impl<K> Clone for Field<K> {
    fn clone(&self) -> Field<K> {
        Field {
            name: self.name.clone(),
            kind: PhantomData,
        }
    }
}

impl<K> fmt::Debug for Field<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Field({})", self.name)
    }
}

impl Field<Object> {
    /// One of the object's fields: `Article::AUTHOR.field(Author::NAME)` is
    /// `author.name`.
    pub fn field<K>(&self, field: Field<K>) -> Field<K> {
        Field {
            name: Cow::Owned(format!("{}.{}", self.name, field.name)),
            kind: PhantomData,
        }
    }
}

impl Field<Text> {
    /// Documents whose field matches `text`, analyzed.
    pub fn r#match(&self, text: impl Into<String>) -> QueryContainer {
        Query::r#match(self.name(), text.into())
    }

    /// Documents whose field contains `text` as a phrase.
    pub fn match_phrase(&self, text: impl Into<String>) -> QueryContainer {
        Query::match_phrase(self.name(), text)
    }

    /// Documents whose field has `text`'s terms in order and close
    /// together.
    pub fn intervals(&self, text: impl Into<String>) -> QueryContainer {
        Query::intervals(self.name(), text)
    }
}

impl Field<Keyword> {
    /// Documents whose field is exactly `value`.
    pub fn term(&self, value: impl Into<String>) -> QueryContainer {
        Query::term(self.name(), value.into())
    }

    /// Documents whose field is exactly one of `values`.
    pub fn terms<V: Into<String>>(&self, values: impl IntoIterator<Item = V>) -> QueryContainer {
        Query::terms(self.name(), values.into_iter().map(Into::into))
    }
}

impl Field<SemanticText> {
    /// Documents whose field means something close to `text`.
    pub fn semantic(&self, text: impl Into<String>) -> QueryContainer {
        Query::semantic(self.name(), text)
    }
}

impl Field<DenseVector> {
    /// The `k` nearest neighbours of `vector`, from `num_candidates` per
    /// shard, as a retriever.
    pub fn knn(&self, vector: Vec<f32>, k: i32, num_candidates: i32) -> RetrieverContainer {
        Retriever::knn(self.name(), vector, k, num_candidates)
    }

    /// The `k` nearest neighbours of `vector`, as a query.
    pub fn knn_query(&self, vector: Vec<f32>, k: i32) -> QueryContainer {
        Query::knn(self.name(), vector, k)
    }
}

impl Field<SparseVector> {
    /// Documents that score well against these token weights.
    pub fn sparse_vector<T: Into<String>>(
        &self,
        weights: impl IntoIterator<Item = (T, f32)>,
    ) -> QueryContainer {
        Query::sparse_vector(self.name(), weights)
    }

    /// Documents that score well against `text`, expanded by the inference
    /// endpoint `inference_id`.
    pub fn sparse_vector_text(
        &self,
        inference_id: impl Into<String>,
        text: impl Into<String>,
    ) -> QueryContainer {
        Query::sparse_vector_text(self.name(), inference_id, text)
    }
}

impl Field<Number> {
    /// Documents whose field is exactly `value`.
    pub fn term(&self, value: impl Into<FieldValue>) -> QueryContainer {
        Query::term(self.name(), value)
    }

    /// Documents whose field is in a range: `Product::PRICE.range(10.0..20.0)`.
    pub fn range(&self, bounds: impl RangeBounds<f64>) -> QueryContainer {
        let mut range = NumberRangeQuery::new();
        range = match bounds.start_bound() {
            Bound::Included(v) => range.gte(*v),
            Bound::Excluded(v) => range.gt(*v),
            Bound::Unbounded => range,
        };
        range = match bounds.end_bound() {
            Bound::Included(v) => range.lte(*v),
            Bound::Excluded(v) => range.lt(*v),
            Bound::Unbounded => range,
        };
        Query::range(self.name(), range)
    }
}

impl Field<Date> {
    /// Documents whose field is in a range of dates or date math:
    /// `Article::PUBLISHED.range("now-7d".."now")`.
    pub fn range<'a>(&self, bounds: impl RangeBounds<&'a str>) -> QueryContainer {
        let mut range = DateRangeQuery::new();
        range = match bounds.start_bound() {
            Bound::Included(v) => range.gte(*v),
            Bound::Excluded(v) => range.gt(*v),
            Bound::Unbounded => range,
        };
        range = match bounds.end_bound() {
            Bound::Included(v) => range.lte(*v),
            Bound::Excluded(v) => range.lt(*v),
            Bound::Unbounded => range,
        };
        Query::range(self.name(), range)
    }
}

impl Field<Boolean> {
    /// Documents whose field is `value`.
    pub fn term(&self, value: bool) -> QueryContainer {
        Query::term(self.name(), value)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn json(query: QueryContainer) -> Value {
        serde_json::to_value(query).unwrap()
    }

    #[test]
    fn queries_by_kind() {
        const TITLE: Field<Text> = Field::new("title");
        const PRICE: Field<Number> = Field::new("price");
        const PUBLISHED: Field<Date> = Field::new("published");
        const AUTHOR: Field<Object> = Field::new("author");
        const NAME: Field<Keyword> = Field::new("name");
        assert_eq!(
            json(TITLE.r#match("hnsw")),
            json!({"match": {"title": "hnsw"}})
        );
        assert_eq!(
            json(PRICE.range(10.0..20.0)),
            json!({"range": {"price": {"gte": 10.0, "lt": 20.0}}})
        );
        assert_eq!(
            json(PUBLISHED.range("now-7d"..)),
            json!({"range": {"published": {"gte": "now-7d"}}})
        );
        assert_eq!(
            json(AUTHOR.field(NAME).term("ada")),
            json!({"term": {"author.name": "ada"}})
        );
    }
}
