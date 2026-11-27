//! Logstash's centrally managed pipelines against Elasticsearch: one put,
//! read, read among them all, and deleted; and one written as Kibana writes
//! them, with its metadata's version as a number, read.

use oxsearch::Method;
use oxsearch::logstash::{
    Pipeline, PipelineMetadata, PipelineSettings, delete_pipeline, get_pipeline, put_pipeline,
};
use oxsearch_integration::{TestIndex, client};
use serde_json::json;

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn pipelines() {
    let client = client();
    let id = TestIndex::named("logstash").name().to_owned();
    client
        .send(&put_pipeline::Request::new(id.as_str()).body(Pipeline::new(
            "a test's pipeline",
            "2026-11-23T00:00:00Z",
            "input { generator {} } output { null {} }",
            PipelineMetadata::new("logstash_pipeline", "1"),
            PipelineSettings::new(1, 125, 50, "memory", "1gb", 1024),
            "elastic",
        )))
        .await
        .unwrap();

    let read = client
        .send(&get_pipeline::Request::new().id(vec![id.clone()]))
        .await
        .unwrap();
    let pipeline = &read[id.as_str()];
    assert_eq!(pipeline.description.as_deref(), Some("a test's pipeline"));
    let settings = pipeline.pipeline_settings.as_ref().expect("the settings");
    assert_eq!(settings.pipeline_batch_size, Some(125));
    let all = client.send(&get_pipeline::Request::new()).await.unwrap();
    assert!(all.contains_key(id.as_str()), "{all:?}");

    client
        .send(&delete_pipeline::Request::new(id.as_str()))
        .await
        .unwrap();
    let left = client.send(&get_pipeline::Request::new()).await.unwrap();
    assert!(!left.contains_key(id.as_str()), "{left:?}");
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn written_by_kibana() {
    let client = client();
    let id = TestIndex::named("logstash-kibana").name().to_owned();
    client
        .send_raw(
            Method::Put,
            &format!("/_logstash/pipeline/{id}"),
            Some(&json!({
                "description": "Kibana's",
                "last_modified": "2026-11-23T00:00:00.000Z",
                "pipeline": "input { generator {} } output { null {} }",
                "pipeline_metadata": {"type": "logstash_pipeline", "version": 1},
                "pipeline_settings": {"pipeline.workers": 1, "queue.type": "memory"},
                "username": "elastic",
            })),
        )
        .await
        .unwrap();
    let raw = client
        .send_raw(Method::Get, &format!("/_logstash/pipeline/{id}"), None)
        .await
        .unwrap();
    let read = client
        .send(&get_pipeline::Request::new().id(vec![id.clone()]))
        .await;
    client
        .send(&delete_pipeline::Request::new(id.as_str()))
        .await
        .unwrap();
    let read = read.unwrap_or_else(|e| panic!("{e}, from {raw}"));
    assert_eq!(read[id.as_str()].description.as_deref(), Some("Kibana's"));
}
