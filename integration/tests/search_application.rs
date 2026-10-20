//! Search applications against Elasticsearch: a search template and its
//! indices behind one name, searched with parameters. They need a license
//! above Basic.

use oxsearch::search_application::{
    SearchApplicationParameters, SearchApplicationTemplate, delete, get, list, put, render_query,
    search,
};
use oxsearch::shapes::IndexMap;
use oxsearch::{Error, Refresh, Script, ScriptSource};
use oxsearch_docs::index;
use oxsearch_integration::{License, TestIndex, client};
use serde_json::{Value, json};

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn named_templates() {
    let client = client();
    let test = TestIndex::named("app-books");
    for (n, title) in ["HNSW graphs", "Bulk ingest"].into_iter().enumerate() {
        client
            .send(
                &index::Request::new(test.name())
                    .id(n.to_string())
                    .refresh(Refresh::True)
                    .body(json!({"title": title})),
            )
            .await
            .unwrap();
    }

    // A match on the title, its text a parameter.
    let mut defaults = IndexMap::new();
    defaults.insert("text".to_owned(), json!("search"));
    let template = SearchApplicationTemplate::new(
        Script::new()
            .source(ScriptSource::ScriptTemplate(
                json!({"query": {"match": {"title": "{{text}}"}}}),
            ))
            .params(defaults),
    );
    let name = format!("{}-app", test.name());
    let created = client
        .send(&put::Request::new(name.as_str()).body(
            SearchApplicationParameters::new(vec![test.name().to_owned()]).template(template),
        ))
        .await;
    if License::from_env() == License::Basic {
        let Err(Error::License(e)) = created else {
            panic!("{created:?}");
        };
        assert_eq!(e.feature, "search application");
        test.delete(&client).await;
        return;
    }
    created.unwrap();

    let read = client
        .send(&get::Request::new(name.as_str()))
        .await
        .unwrap();
    assert_eq!(read.indices, [test.name().to_owned()]);
    let listed = client.send(&list::Request::new()).await.unwrap();
    assert!(
        listed
            .results
            .iter()
            .any(|a| a.name.as_deref() == Some(name.as_str())),
        "{listed:?}"
    );

    let mut params = IndexMap::new();
    params.insert("text".to_owned(), json!("hnsw"));
    let found = client
        .send(
            &search::Request::new(name.as_str())
                .body(search::RequestBody::new().params(params.clone())),
        )
        .await
        .unwrap();
    let ids: Vec<Option<&str>> = found
        .hits
        .as_ref()
        .map(|h| h.hits.iter().map(|hit| hit.id.as_deref()).collect())
        .unwrap_or_default();
    assert_eq!(ids, [Some("0")]);

    // The search request the template renders. The specification gives
    // the response no properties, so it's read as JSON.
    let rendered: Value = client
        .send_as(
            &render_query::Request::new(name.as_str())
                .body(render_query::RequestBody::new().params(params)),
        )
        .await
        .unwrap();
    // The match comes back in its full form.
    assert_eq!(rendered["query"]["match"]["title"]["query"], "hnsw");

    client
        .send(&delete::Request::new(name.as_str()))
        .await
        .unwrap();
    test.delete(&client).await;
}
