//! Generated types are reachable where their namespace puts them, without
//! going through `oxsearch_core`.

use oxsearch::info;
use oxsearch::search::search::{Highlight, HighlightField, Hit, RescoreQuery};
use oxsearch::shapes::SingleKey;
use oxsearch::{FieldValue, KnnRetriever, Refresh, RetrieverContainer, ScoreNormalizer};

#[test]
fn paths() {
    let _: RetrieverContainer = KnnRetriever::new("embedding", 3).into();
    let _ = (
        Refresh::True,
        ScoreNormalizer::Minmax,
        FieldValue::from("x"),
    );
    let field: SingleKey<String, HighlightField> = SingleKey::new("title", HighlightField::new());
    let _ = Highlight::new(field);
    let _ = RescoreQuery::new(oxsearch::Query::match_all());
    let _: Option<Hit> = None;
    let _ = info::Request::new();
    let _ = oxsearch::mapping::DenseVectorSimilarity::Cosine;
    let _ = oxsearch::docs::bulk::FailureStoreStatus::Used;
}
