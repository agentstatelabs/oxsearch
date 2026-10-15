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

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn embeds_reranks_and_manages_endpoints() {
    // Elastic's own models need a license above Basic.
    if License::from_env() != License::Trial {
        return;
    }
    let client = client();

    let embedded = client
        .send(
            &sparse_embedding::Request::new(".elser-2-elasticsearch").body(
                sparse_embedding::RequestBody::new("vector search with hnsw"),
            ),
        )
        .await
        .unwrap();
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

    let ranked = client
        .send(
            &rerank::Request::new(".rerank-v1-elasticsearch").body(rerank::RequestBody::new(
                "how do vector indexes find neighbours?",
                rerank::RerankStringInput::from(vec![
                    "Shards have replicas on other nodes.".to_owned(),
                    "HNSW graphs find nearest neighbours quickly.".to_owned(),
                ]),
            )),
        )
        .await
        .unwrap();
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
