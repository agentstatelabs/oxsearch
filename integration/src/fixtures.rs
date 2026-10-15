//! The mapping and documents most tests share.

use oxsearch_core::shapes::IndexMap;
use oxsearch_mapping::{
    DenseVectorProperty, DenseVectorSimilarity, KeywordProperty, Property, SparseVectorProperty,
    TextProperty, TypeMapping,
};

use crate::Article;

/// A mapping for [`Article`]s: `title` as text, `tag` as a keyword,
/// `embedding` as an indexed three-dimensional dense vector, and `terms` as
/// a sparse vector.
pub fn article_mapping() -> TypeMapping {
    let mut properties = IndexMap::new();
    properties.insert("title".to_owned(), Property::Text(TextProperty::new()));
    properties.insert("tag".to_owned(), Property::Keyword(KeywordProperty::new()));
    properties.insert(
        "embedding".to_owned(),
        Property::DenseVector(
            DenseVectorProperty::new()
                .dims(3)
                .index(true)
                .similarity(DenseVectorSimilarity::Cosine),
        ),
    );
    properties.insert(
        "terms".to_owned(),
        Property::SparseVector(SparseVectorProperty::new()),
    );
    TypeMapping::new().properties(properties)
}

/// Three articles, by id.
pub fn articles() -> Vec<(&'static str, Article)> {
    vec![
        (
            "1",
            Article::new(
                "Tuning HNSW graphs",
                "vector",
                [1.0, 0.0, 0.0],
                &[("hnsw", 2.0), ("graph", 1.0)],
            ),
        ),
        (
            "2",
            Article::new(
                "Bulk loading at speed",
                "ingest",
                [0.0, 1.0, 0.0],
                &[("bulk", 2.0), ("speed", 1.0)],
            ),
        ),
        (
            "3",
            Article::new(
                "Quantizing vectors",
                "vector",
                [0.7, 0.0, 0.7],
                &[("quantization", 2.0), ("hnsw", 0.5)],
            ),
        ),
    ]
}
