//! Ingest simulation against Elasticsearch, on any license: a document run
//! through a pipeline that exists only in the request, and checked against
//! a strict index's mapping, which refuses a field it doesn't map until the
//! request adds one. Nothing is indexed, and no index is made.

use oxsearch::Method;
use oxsearch::ingest::{Document, Pipeline, SetProcessor};
use oxsearch::mapping::{KeywordProperty, Property, TypeMapping};
use oxsearch::shapes::IndexMap;
use oxsearch::simulate::ingest;
use oxsearch_integration::{TestIndex, client};
use serde_json::json;

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn ingest() {
    let client = client();
    let test = TestIndex::named("simulate");
    client
        .send_raw(
            Method::Put,
            &format!("/{}", test.name()),
            Some(&json!({"mappings": {
                "dynamic": "strict",
                "properties": {"title": {"type": "text"}},
            }})),
        )
        .await
        .unwrap();
    let document = || Document::new(json!({"title": "a note", "tag": "news"})).id("1");

    // Through a pipeline the request brings, which sets a field, into an
    // index that doesn't exist yet.
    let unmade = format!("{}-unmade", test.name());
    let pipeline = format!("{}-pipeline", test.name());
    let mut pipelines = IndexMap::new();
    pipelines.insert(
        pipeline.clone(),
        Pipeline::new().processors(vec![SetProcessor::new("checked").value(json!(true)).into()]),
    );
    let simulated = client
        .send(
            &ingest::Request::new()
                .index(unmade.as_str())
                .pipeline(pipeline.as_str())
                .body(ingest::RequestBody::new(vec![document()]).pipeline_substitutions(pipelines)),
        )
        .await
        .unwrap();
    let doc = simulated.docs[0].doc.as_ref().expect("the document");
    assert_eq!(doc.source.get("checked"), Some(&json!(true)), "{doc:?}");
    assert_eq!(doc.executed_pipelines, vec![pipeline]);

    // Against the strict mapping, which doesn't map the tag.
    let refused = client
        .send(
            &ingest::Request::new()
                .index(test.name())
                .body(ingest::RequestBody::new(vec![document()])),
        )
        .await
        .unwrap();
    let doc = refused.docs[0].doc.as_ref().expect("the document");
    let error = doc.error.as_ref().expect("the mapping's refusal");
    assert_eq!(
        error.r#type.as_deref(),
        Some("strict_dynamic_mapping_exception"),
        "{error:?}"
    );

    // With the tag added to the mapping for the simulation.
    let mut tag = IndexMap::new();
    tag.insert("tag".to_owned(), Property::Keyword(KeywordProperty::new()));
    let addition = TypeMapping::new().properties(tag);
    let taken = client
        .send(
            &ingest::Request::new()
                .index(test.name())
                .body(ingest::RequestBody::new(vec![document()]).mapping_addition(addition)),
        )
        .await
        .unwrap();
    let doc = taken.docs[0].doc.as_ref().expect("the document");
    assert!(doc.error.is_none(), "{doc:?}");

    test.delete(&client).await;
}
