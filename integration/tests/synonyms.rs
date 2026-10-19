//! Synonym sets: rules stored in Elasticsearch that a search analyzer reads,
//! and that take effect without reindexing when they change.

use oxsearch::Error;
use oxsearch::admin::indices::{IndexSettings, IndexSettingsAnalysis, analyze, create};
use oxsearch::analysis::{CustomAnalyzer, SynonymGraphTokenFilter, TokenFilterDefinition};
use oxsearch::shapes::IndexMap;
use oxsearch::synonyms::{
    SynonymRule, delete_synonym, delete_synonym_rule, get_synonym, get_synonym_rule,
    get_synonyms_sets, put_synonym, put_synonym_rule,
};
use oxsearch_integration::{TestClient, TestIndex, client};

/// The tokens the index's `with_synonyms` analyzer makes of the text.
async fn tokens(client: &TestClient, index: &str, text: &str) -> Vec<String> {
    let analyzed = client
        .send(
            &analyze::Request::new().index(index).body(
                analyze::RequestBody::new()
                    .analyzer("with_synonyms")
                    .text(text.to_owned()),
            ),
        )
        .await
        .unwrap();
    analyzed
        .tokens
        .unwrap_or_default()
        .into_iter()
        .filter_map(|t| t.token)
        .collect()
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn synonym_sets() {
    let client = client();
    let set = TestIndex::named("synonyms").name().to_owned();
    let created = client
        .send(
            &put_synonym::Request::new(set.as_str()).body(put_synonym::RequestBody::new(vec![
                SynonymRule::new("hnsw, graph index").id("graphs"),
            ])),
        )
        .await
        .unwrap();
    assert!(created.result.is_some());
    let read = client
        .send(&get_synonym::Request::new(set.as_str()))
        .await
        .unwrap();
    assert_eq!(read.count, Some(1));
    let sets = client
        .send(&get_synonyms_sets::Request::new())
        .await
        .unwrap();
    assert!(
        sets.results
            .iter()
            .any(|s| s.synonyms_set.as_deref() == Some(set.as_str())),
        "{sets:?}"
    );

    // A search analyzer that reads the set.
    let test = TestIndex::named("synonyms-index");
    let mut filters = IndexMap::new();
    filters.insert(
        "synonyms".to_owned(),
        TokenFilterDefinition::from(
            SynonymGraphTokenFilter::new()
                .synonyms_set(set.clone())
                .updateable(true),
        )
        .into(),
    );
    let mut analyzers = IndexMap::new();
    analyzers.insert(
        "with_synonyms".to_owned(),
        CustomAnalyzer::new("standard")
            .filter(vec!["lowercase".to_owned(), "synonyms".to_owned()])
            .into(),
    );
    client
        .send(
            &create::Request::new(test.name()).body(
                create::RequestBody::new().settings(
                    IndexSettings::new().analysis(
                        IndexSettingsAnalysis::new()
                            .filter(filters)
                            .analyzer(analyzers),
                    ),
                ),
            ),
        )
        .await
        .unwrap();
    let mut expanded = tokens(&client, test.name(), "HNSW").await;
    expanded.sort();
    assert_eq!(expanded, ["graph", "hnsw", "index"]);

    // A rule added to the set takes effect at once.
    client
        .send(&put_synonym_rule::Request::new(set.as_str(), "rag").body(
            put_synonym_rule::RequestBody::new("rag => retrieval augmented generation"),
        ))
        .await
        .unwrap();
    assert_eq!(
        tokens(&client, test.name(), "rag").await,
        ["retrieval", "augmented", "generation"]
    );
    let rule = client
        .send(&get_synonym_rule::Request::new(set.as_str(), "rag"))
        .await
        .unwrap();
    assert_eq!(
        rule.synonyms.as_deref(),
        Some("rag => retrieval augmented generation")
    );
    client
        .send(&delete_synonym_rule::Request::new(set.as_str(), "rag"))
        .await
        .unwrap();
    assert_eq!(tokens(&client, test.name(), "rag").await, ["rag"]);

    // A set an index uses can't be deleted.
    let in_use = client
        .send(&delete_synonym::Request::new(set.as_str()))
        .await;
    assert!(
        matches!(&in_use, Err(Error::Elasticsearch(e)) if e.status == 400),
        "{in_use:?}"
    );
    test.delete(&client).await;
    client
        .send(&delete_synonym::Request::new(set.as_str()))
        .await
        .unwrap();
}
