//! The cat APIs against Elasticsearch, on any license: every table, read
//! as typed rows from its JSON form, with an index, an alias, a template
//! and a component template of the test's own to find in them; and the
//! list of tables, which is text.

use oxsearch::Method;
use oxsearch::Refresh;
use oxsearch::cat::{
    aliases, allocation, circuit_breaker, component_templates, count, fielddata, health, help,
    indices, master, ml_data_frame_analytics, ml_datafeeds, ml_jobs, ml_trained_models, nodeattrs,
    nodes, pending_tasks, plugins, recovery, repositories, segments, shards, snapshots, tasks,
    templates, thread_pool, transforms,
};
use oxsearch_docs::index;
use oxsearch_integration::{TestIndex, client};
use serde_json::json;

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn tables() {
    let client = client();
    let test = TestIndex::named("cat");
    for n in 0..3 {
        client
            .send(
                &index::Request::new(test.name())
                    .refresh(Refresh::True)
                    .body(json!({"title": format!("row {n}"), "tag": "cat"})),
            )
            .await
            .unwrap();
    }
    let alias = format!("{}-alias", test.name());
    let template = format!("{}-template", test.name());
    let component = format!("{}-component", test.name());
    for (path, body) in [
        (format!("/{}/_alias/{alias}", test.name()), json!({})),
        (
            format!("/_component_template/{component}"),
            json!({"template": {"settings": {"number_of_replicas": 0}}}),
        ),
        (
            format!("/_index_template/{template}"),
            json!({"index_patterns": [format!("{template}-*")], "composed_of": [component]}),
        ),
    ] {
        client
            .send_raw(Method::Put, &path, Some(&body))
            .await
            .unwrap();
    }

    // The test's own index, alias and templates, found in their tables.
    let rows = client
        .send(&indices::Request::new().index(test.name()))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].index.as_deref(), Some(test.name()));
    let rows = client
        .send(&count::Request::new().index(test.name()))
        .await
        .unwrap();
    assert_eq!(rows[0].count.as_deref(), Some("3"));
    let rows = client
        .send(&aliases::Request::new().name(alias.as_str()))
        .await
        .unwrap();
    assert_eq!(rows[0].index.as_deref(), Some(test.name()));
    let rows = client
        .send(&templates::Request::new().name(template.as_str()))
        .await
        .unwrap();
    assert_eq!(rows[0].name.as_deref(), Some(template.as_str()));
    let rows = client
        .send(&component_templates::Request::new().name(component.as_str()))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let rows = client
        .send(&shards::Request::new().index(test.name()))
        .await
        .unwrap();
    assert!(
        rows.iter().any(|r| r.state.as_deref() == Some("STARTED")),
        "{rows:?}"
    );
    client
        .send(&segments::Request::new().index(test.name()))
        .await
        .unwrap();
    client
        .send(&recovery::Request::new().index(test.name()))
        .await
        .unwrap();

    // The cluster and its one node.
    let rows = client.send(&health::Request::new()).await.unwrap();
    assert!(rows[0].status.is_some(), "{rows:?}");
    let rows = client.send(&nodes::Request::new()).await.unwrap();
    assert_eq!(rows.len(), 1);
    let rows = client.send(&master::Request::new()).await.unwrap();
    assert!(rows[0].node.is_some(), "{rows:?}");
    client.send(&allocation::Request::new()).await.unwrap();
    client.send(&nodeattrs::Request::new()).await.unwrap();
    client.send(&plugins::Request::new()).await.unwrap();
    client.send(&thread_pool::Request::new()).await.unwrap();
    client.send(&fielddata::Request::new()).await.unwrap();
    client.send(&pending_tasks::Request::new()).await.unwrap();
    client.send(&circuit_breaker::Request::new()).await.unwrap();
    client.send(&tasks::Request::new()).await.unwrap();
    client.send(&repositories::Request::new()).await.unwrap();
    client.send(&snapshots::Request::new()).await.unwrap();
    client.send(&transforms::Request::new()).await.unwrap();

    // Machine learning's tables, empty or not; Basic lists them too.
    client.send(&ml_jobs::Request::new()).await.unwrap();
    client.send(&ml_datafeeds::Request::new()).await.unwrap();
    client
        .send(&ml_data_frame_analytics::Request::new())
        .await
        .unwrap();
    client
        .send(&ml_trained_models::Request::new())
        .await
        .unwrap();

    // The list of tables is text, whatever the format asked for.
    let list = client.send(&help::Request::new()).await.unwrap();
    assert!(list.contains("/_cat/indices"), "{list:?}");

    for path in [
        format!("/_index_template/{template}"),
        format!("/_component_template/{component}"),
    ] {
        client.send_raw(Method::Delete, &path, None).await.unwrap();
    }
    test.delete(&client).await;
}
