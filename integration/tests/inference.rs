//! The inference API: the endpoints a cluster has, including the ones
//! Elasticsearch defines itself.

use oxsearch_inference::{delete, get, inference, put, rerank, sparse_embedding};
use oxsearch_integration::{License, client};
use serde_json::json;

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn lists_inference_endpoints() {
    let response = client().send(&get::Request::new()).await.unwrap();
    let ids: Vec<&str> = response
        .endpoints
        .iter()
        .filter_map(|e| e.inference_id.as_deref())
        .collect();
    assert!(ids.contains(&".elser-2-elasticsearch"), "{ids:?}");
    for endpoint in &response.endpoints {
        assert!(endpoint.service.is_some(), "{endpoint:?}");
        assert!(endpoint.task_type.is_some(), "{endpoint:?}");
    }
}

/// A call to one of Elastic's models, sent again while the model is still
/// deploying. On first use, a model can take longer to start than the 30 s
/// Elasticsearch waits for it (408), and the deployment goes on meanwhile; a
/// call during that time can be refused because the deployment is already
/// starting (409).
async fn deployed<T, F>(mut call: impl FnMut() -> F) -> T
where
    F: std::future::Future<Output = Result<T, oxsearch_core::Error>>,
{
    use oxsearch_core::Error;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
    loop {
        let starting = match call().await {
            Err(Error::Elasticsearch(e))
                if e.error_type() == Some("model_deployment_timeout_exception") =>
            {
                e
            }
            Err(Error::Conflict(e))
                if e.causes()
                    .any(|c| c.r#type.as_deref() == Some("resource_already_exists_exception")) =>
            {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                e
            }
            other => return other.unwrap(),
        };
        assert!(
            std::time::Instant::now() < deadline,
            "the model didn't deploy in 5 minutes: {starting}"
        );
    }
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn embeds_reranks_and_manages_endpoints() {
    // Elastic's own models need a license above Basic.
    if License::from_env() != License::Trial {
        return;
    }
    let client = client();

    let embed = sparse_embedding::Request::new(".elser-2-elasticsearch").body(
        sparse_embedding::RequestBody::new("vector search with hnsw"),
    );
    let embedded = deployed(|| client.send(&embed)).await;
    assert_eq!(embedded.sparse_embedding.len(), 1, "{embedded:?}");

    // The general call, by endpoint alone.
    let embedded =
        client
            .send(&inference::Request::new(".elser-2-elasticsearch").body(
                inference::RequestBody::new(vec!["replicas".to_owned(), "shards".to_owned()]),
            ))
            .await
            .unwrap();
    let oxsearch_inference::InferenceResult::SparseEmbedding(results) = &embedded.0 else {
        panic!("sparse embeddings, not {embedded:?}")
    };
    assert_eq!(results.len(), 2);

    let rank = rerank::Request::new(".rerank-v1-elasticsearch").body(rerank::RequestBody::new(
        "how do vector indexes find neighbours?",
        rerank::RerankStringInput::from(vec![
            "Shards have replicas on other nodes.".to_owned(),
            "HNSW graphs find nearest neighbours quickly.".to_owned(),
        ]),
    ));
    let ranked = deployed(|| client.send(&rank)).await;
    assert_eq!(ranked.rerank[0].index, Some(1), "{ranked:?}");

    // An endpoint of our own, on the ELSER deployment the default one made.
    let id = "oxsearch-test-sparse";
    let _ = client.send(&delete::Request::new(id)).await;
    client
        .send(
            &put::Request::new(id)
                .task_type(oxsearch_inference::TaskType::SparseEmbedding)
                .body(oxsearch_inference::InferenceEndpoint::new(
                    "elasticsearch",
                    json!({"deployment_id": ".elser-2-elasticsearch"}),
                )),
        )
        .await
        .unwrap();
    let ours = client
        .send(&get::Request::new().inference_id(id))
        .await
        .unwrap();
    assert_eq!(ours.endpoints.len(), 1);
    client.send(&delete::Request::new(id)).await.unwrap();
}
