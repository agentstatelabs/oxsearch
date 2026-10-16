//! Mappings for AI search, built from the generated types, accepted by
//! Elasticsearch and read back into the same types.

use oxsearch_core::shapes::IndexMap;
use oxsearch_integration::{TestIndex, client};
use oxsearch_mapping::{
    DenseVectorIndexOptions, DenseVectorIndexOptionsType, DenseVectorProperty,
    DenseVectorSimilarity, Property, SemanticTextProperty, SparseVectorProperty, TypeMapping,
};

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn vector_and_semantic_mappings_round_trip() {
    let client = client();
    let mut properties = IndexMap::new();
    properties.insert(
        "embedding".to_owned(),
        Property::DenseVector(
            DenseVectorProperty::new()
                .dims(64)
                .similarity(DenseVectorSimilarity::DotProduct)
                .index_options(
                    DenseVectorIndexOptions::new(DenseVectorIndexOptionsType::Int8Hnsw).m(24),
                ),
        ),
    );
    properties.insert(
        "terms".to_owned(),
        Property::SparseVector(SparseVectorProperty::new()),
    );
    properties.insert(
        "body".to_owned(),
        Property::SemanticText(SemanticTextProperty::new().inference_id(".elser-2-elasticsearch")),
    );
    let test = TestIndex::create(
        &client,
        "mappings",
        &TypeMapping::new().properties(properties),
    )
    .await;

    // Elasticsearch's own description of the mapping reads into the
    // generated types.
    let read: TypeMapping = serde_json::from_value(test.mapping(&client).await).unwrap();
    let properties = read.properties.expect("the mapping has properties");
    let Some(Property::DenseVector(embedding)) = properties.get("embedding") else {
        panic!("embedding: {:?}", properties.get("embedding"));
    };
    assert_eq!(embedding.dims, Some(64));
    let options = embedding.index_options.as_ref().expect("index options");
    assert_eq!(options.r#type, Some(DenseVectorIndexOptionsType::Int8Hnsw));
    assert_eq!(options.m, Some(24));
    assert!(
        matches!(properties.get("terms"), Some(Property::SparseVector(_))),
        "{properties:?}"
    );
    let Some(Property::SemanticText(body)) = properties.get("body") else {
        panic!("body: {:?}", properties.get("body"));
    };
    assert_eq!(body.inference_id.as_deref(), Some(".elser-2-elasticsearch"));
    test.delete(&client).await;
}

/// Semantic search over `semantic_text`: documents are embedded on ingest
/// by Elasticsearch's default ELSER endpoint. On a trial the model is
/// deployed on first use; on Basic the request must still be one
/// Elasticsearch understands.
#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn semantic_search() {
    use oxsearch_core::Refresh;
    use oxsearch_core::query_dsl::{QueryContainer, SemanticQuery};
    use oxsearch_docs::index;
    use oxsearch_integration::{License, understood};
    use oxsearch_search::search;
    use serde_json::json;

    let client = client();
    let mut properties = IndexMap::new();
    properties.insert(
        "body".to_owned(),
        Property::SemanticText(SemanticTextProperty::new()),
    );
    let test = TestIndex::create(
        &client,
        "semantic",
        &TypeMapping::new().properties(properties),
    )
    .await;
    let indexed = client
        .send(
            &index::Request::new(test.name())
                .id("1")
                .refresh(Refresh::True)
                .body(json!({"body": "Graph indexes trade memory for fast approximate nearest neighbour search."})),
        )
        .await;
    let query = QueryContainer::Semantic(SemanticQuery::new(
        "body",
        "how do vector indexes find neighbours quickly?",
    ));
    let searched = client
        .send(
            &search::Request::new()
                .index(test.name())
                .body(search::RequestBody::new().query(query)),
        )
        .await;
    match License::from_env() {
        License::Trial => {
            indexed.unwrap();
            let hits = searched.unwrap().hits.clone().expect("hits").hits;
            assert_eq!(hits.first().and_then(|h| h.id.as_deref()), Some("1"));
        }
        License::Basic => {
            if let Err(e) = indexed {
                assert!(understood(&e), "{e}");
            }
            if let Err(e) = searched {
                assert!(understood(&e), "{e}");
            }
        }
    }
    test.delete(&client).await;
}
