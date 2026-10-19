//! Ingest pipelines against Elasticsearch: written, read, simulated and run
//! on documents as they're indexed, with failures handled; the inference
//! processor, which embeds text as it's indexed; and the grok patterns and
//! IP databases other processors use.

use oxsearch::admin::indices::{IndexSettings, create};
use oxsearch::ingest::{
    Document, GrokProcessor, InferenceProcessor, InputConfig, LowercaseProcessor, Pipeline,
    ProcessorContainer, RenameProcessor, SetProcessor, delete_pipeline, geo_ip_stats,
    get_geoip_database, get_ip_location_database, get_pipeline, processor_grok, put_pipeline,
    simulate,
};
use oxsearch::{Error, Refresh};
use oxsearch_docs::{get, index};
use oxsearch_inference::sparse_embedding;
use oxsearch_integration::{License, TestClient, TestIndex, client, deployed};
use serde_json::{Value, json};

/// Lowercases the tag, renames the headline to a title, and records who
/// indexed the document.
fn tidy() -> Vec<ProcessorContainer> {
    vec![
        LowercaseProcessor::new("tag").into(),
        RenameProcessor::new("headline", "title").into(),
        SetProcessor::new("indexed_by")
            .value(json!("oxsearch"))
            .into(),
    ]
}

/// The sources a simulation returns, in order.
async fn simulated(client: &TestClient, request: simulate::Request) -> Vec<Value> {
    let response = client.send(&request).await.unwrap();
    response
        .docs
        .iter()
        .map(|d| match (&d.doc, &d.error) {
            (Some(doc), _) => json!(doc.source),
            (None, Some(error)) => panic!("{error:?}"),
            (None, None) => panic!("{d:?}"),
        })
        .collect()
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn pipelines() {
    let client = client();
    let pipeline = TestIndex::named("pipeline").name().to_owned();

    let stored = client
        .send(
            &put_pipeline::Request::new(pipeline.as_str()).body(
                put_pipeline::RequestBody::new()
                    .description("Tidies articles")
                    .processors(tidy()),
            ),
        )
        .await
        .unwrap();
    assert_eq!(stored.acknowledged, Some(true));
    let read = client
        .send(&get_pipeline::Request::new().id(pipeline.as_str()))
        .await
        .unwrap();
    let read = &read.0[&pipeline];
    assert_eq!(read.description.as_deref(), Some("Tidies articles"));
    assert_eq!(read.processors.as_ref().map(Vec::len), Some(3));

    // Simulated by its id, on two documents.
    let docs = vec![
        Document::new(json!({"headline": "HNSW graphs", "tag": "VECTORS"})),
        Document::new(json!({"headline": "Bulk ingest", "tag": "Ingest"})),
    ];
    let sources = simulated(
        &client,
        simulate::Request::new()
            .id(pipeline.as_str())
            .body(simulate::RequestBody::new(docs.clone())),
    )
    .await;
    assert_eq!(
        sources[0],
        json!({"title": "HNSW graphs", "tag": "vectors", "indexed_by": "oxsearch"})
    );
    assert_eq!(sources[1]["tag"], "ingest");

    // Simulated without storing it, processor by processor.
    let verbose =
        client
            .send(&simulate::Request::new().verbose(true).body(
                simulate::RequestBody::new(docs).pipeline(Pipeline::new().processors(tidy())),
            ))
            .await
            .unwrap();
    let steps = verbose.docs[0].processor_results.as_ref().unwrap();
    assert_eq!(steps.len(), 3, "{steps:?}");

    // Run as documents are indexed: named in the request, or as the
    // index's default.
    let articles = TestIndex::named("piped");
    client
        .send(
            &create::Request::new(articles.name()).body(
                create::RequestBody::new()
                    .settings(IndexSettings::new().default_pipeline(pipeline.as_str())),
            ),
        )
        .await
        .unwrap();
    client
        .send(
            &index::Request::new(articles.name())
                .id("1")
                .refresh(Refresh::True)
                .body(json!({"headline": "Quantizing vectors", "tag": "VECTORS"})),
        )
        .await
        .unwrap();
    let got = client
        .send(&get::Request::new("1", articles.name()))
        .await
        .unwrap();
    assert_eq!(
        got.source,
        Some(json!({"title": "Quantizing vectors", "tag": "vectors", "indexed_by": "oxsearch"}))
    );
    articles.delete(&client).await;

    client
        .send(&delete_pipeline::Request::new(pipeline.as_str()))
        .await
        .unwrap();
    let gone = client
        .send(&get_pipeline::Request::new().id(pipeline.as_str()))
        .await;
    assert!(
        matches!(&gone, Err(Error::Elasticsearch(e)) if e.status == 404),
        "{gone:?}"
    );
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn failures() {
    let client = client();
    // A processor that fails records why, and the pipeline goes on.
    let handled = Pipeline::new().processors(vec![
        RenameProcessor::new("missing", "found")
            .on_failure(vec![
                SetProcessor::new("failure")
                    .value(json!("{{ _ingest.on_failure_message }}"))
                    .into(),
            ])
            .into(),
        SetProcessor::new("after").value(json!(true)).into(),
    ]);
    let sources = simulated(
        &client,
        simulate::Request::new().body(
            simulate::RequestBody::new(vec![Document::new(json!({"title": "x"}))])
                .pipeline(handled),
        ),
    )
    .await;
    let failure = sources[0]["failure"].as_str().unwrap();
    assert!(failure.contains("missing"), "{failure}");
    assert_eq!(sources[0]["after"], true);

    // Without a handler, the document fails.
    let unhandled =
        Pipeline::new().processors(vec![RenameProcessor::new("missing", "found").into()]);
    let response = client
        .send(
            &simulate::Request::new().body(
                simulate::RequestBody::new(vec![Document::new(json!({"title": "x"}))])
                    .pipeline(unhandled),
            ),
        )
        .await
        .unwrap();
    let error = response.docs[0]
        .error
        .as_ref()
        .expect("the document failed");
    assert_eq!(error.r#type.as_deref(), Some("illegal_argument_exception"));
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn grok_and_ip_databases() {
    let client = client();
    let patterns = client.send(&processor_grok::Request::new()).await.unwrap();
    assert!(
        patterns.patterns.contains_key("IP"),
        "{:?}",
        patterns.patterns.len()
    );

    let parsed = simulated(
        &client,
        simulate::Request::new().body(
            simulate::RequestBody::new(vec![Document::new(
                json!({"message": "10.0.0.7 GET /books 200"}),
            )])
            .pipeline(Pipeline::new().processors(vec![
                GrokProcessor::new(
                    "message",
                    vec!["%{IP:client} %{WORD:method} %{URIPATH:path} %{NUMBER:status:int}".into()],
                )
                .into(),
            ])),
        ),
    )
    .await;
    assert_eq!(parsed[0]["client"], "10.0.0.7");
    assert_eq!(parsed[0]["status"], 200);

    // The databases the GeoIP and IP location processors use: none
    // configured here, and the downloader's statistics.
    client.send(&geo_ip_stats::Request::new()).await.unwrap();
    client
        .send(&get_geoip_database::Request::new())
        .await
        .unwrap();
    client
        .send(&get_ip_location_database::Request::new())
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn inference_processor() {
    // Elastic's own models need a license above Basic.
    if License::from_env() != License::Trial {
        return;
    }
    let client = client();
    // Wait for ELSER to deploy before the pipeline calls it.
    let warm = sparse_embedding::Request::new(".elser-2-elasticsearch")
        .body(sparse_embedding::RequestBody::new("warm up"));
    deployed(|| client.send(&warm)).await;

    let embed = Pipeline::new().processors(vec![
        InferenceProcessor::new(".elser-2-elasticsearch")
            .input_output(InputConfig::new("text", "text_embedding"))
            .into(),
    ]);
    let sources = simulated(
        &client,
        simulate::Request::new().body(
            simulate::RequestBody::new(vec![Document::new(
                json!({"text": "HNSW graphs find nearest neighbours"}),
            )])
            .pipeline(embed),
        ),
    )
    .await;
    // A sparse vector: tokens and their weights.
    let tokens = sources[0]["text_embedding"].as_object().unwrap();
    assert!(!tokens.is_empty());
    assert!(tokens.values().all(Value::is_number), "{tokens:?}");
}
