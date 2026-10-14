//! The fixture is what the job asked for: Elasticsearch 9 on the expected
//! license, and the license decides what the cluster allows.

use oxsearch_integration::{Cluster, License};
use serde_json::json;

#[test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
fn runs_elasticsearch_9() {
    let root = Cluster::from_env().get("/");
    assert_eq!(root.status, 200, "{}", root.body);
    let version = root.body["version"]["number"]
        .as_str()
        .expect("GET / reports version.number");
    assert!(version.starts_with("9."), "Elasticsearch {version}");
    if let Ok(expected) = std::env::var("ELASTICSEARCH_VERSION") {
        assert_eq!(version, expected);
    }
}

#[test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
fn runs_the_expected_license() {
    let license = Cluster::from_env().get("/_license");
    assert_eq!(license.status, 200, "{}", license.body);
    assert_eq!(
        license.body["license"]["type"],
        License::from_env().as_str()
    );
    assert_eq!(license.body["license"]["status"], "active");
}

/// The free Basic license includes kNN, so it works on every license.
#[test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
fn knn_works_on_every_license() {
    let cluster = Cluster::from_env();
    let index = cluster.create_index(
        "knn",
        &json!({"mappings": {"properties": {
            "title": {"type": "text"},
            "embedding": {"type": "dense_vector", "dims": 3, "similarity": "cosine"}
        }}}),
    );
    index.add(&[
        json!({"title": "tuning hnsw", "embedding": [1.0, 0.0, 0.0]}),
        json!({"title": "bulk loading", "embedding": [0.0, 1.0, 0.0]}),
    ]);
    let found = index.search(&json!({
        "retriever": {"knn": {
            "field": "embedding",
            "query_vector": [0.9, 0.1, 0.0],
            "k": 1,
            "num_candidates": 10
        }}
    }));
    assert_eq!(found.status, 200, "{}", found.body);
    assert_eq!(
        found.body["hits"]["hits"][0]["_source"]["title"],
        "tuning hnsw"
    );
}

/// Server-side RRF needs a paid license or a trial; on Basic the cluster
/// refuses it with a license error.
#[test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
fn server_side_rrf_needs_a_trial() {
    let cluster = Cluster::from_env();
    let index = cluster.create_index(
        "rrf",
        &json!({"mappings": {"properties": {
            "title": {"type": "text"},
            "tag": {"type": "keyword"}
        }}}),
    );
    index.add(&[
        json!({"title": "tuning hnsw", "tag": "vector"}),
        json!({"title": "bulk loading", "tag": "vector"}),
    ]);
    let fused = index.search(&json!({
        "retriever": {"rrf": {"retrievers": [
            {"standard": {"query": {"match": {"title": "hnsw"}}}},
            {"standard": {"query": {"term": {"tag": "vector"}}}}
        ]}}
    }));
    match License::from_env() {
        License::Basic => {
            assert_eq!(fused.status, 403, "{}", fused.body);
            let cause = &fused.body["error"]["root_cause"][0];
            assert_eq!(cause["type"], "security_exception", "{}", fused.body);
            assert_eq!(
                cause["license.expired.feature"], "Reciprocal Rank Fusion (RRF)",
                "{}",
                fused.body
            );
        }
        License::Trial => {
            assert_eq!(fused.status, 200, "{}", fused.body);
            assert_eq!(
                fused.body["hits"]["hits"][0]["_source"]["title"],
                "tuning hnsw"
            );
        }
    }
}
