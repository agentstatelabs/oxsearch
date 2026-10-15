//! Fusing a keyword match with nearest neighbours on the client gives the
//! scores the server's `rrf` and `linear` retrievers give, and on the Basic
//! license, which doesn't include them, `Fusion::search` falls back to the
//! client.

use std::collections::BTreeMap;

use oxsearch_core::{Query, Refresh, Retriever, RetrieverContainer, ScoreNormalizer};
use oxsearch_docs::index;
use oxsearch_integration::fixtures::article_mapping;
use oxsearch_integration::{Article, License, TestClient, TestIndex, client};
use oxsearch_search::fusion::Fusion;
use oxsearch_search::search::Hit;

const TITLES: [&str; 12] = [
    "Vector search with HNSW graphs",
    "Search relevance tuning",
    "Vector quantization in practice",
    "Bulk loading vectors at speed",
    "Hybrid search: keywords and vectors",
    "Shards, replicas and search latency",
    "Graph algorithms for nearest neighbours",
    "Search templates and parameters",
    "Embedding models for semantic search",
    "Index lifecycle management",
    "Vector databases compared",
    "Snapshot and restore",
];

/// An index of twelve articles, with embeddings spread around a circle.
async fn passages(client: &TestClient) -> TestIndex {
    let test = TestIndex::create(client, "fusion", &article_mapping()).await;
    for (n, title) in TITLES.iter().enumerate() {
        let angle = n as f32 * 0.5;
        let embedding = [angle.cos(), angle.sin(), (n % 3) as f32 * 0.3 + 0.1];
        client
            .send(
                &index::Request::new(test.name())
                    .id(n.to_string())
                    .refresh(Refresh::True)
                    .body(Article::new(title, "article", embedding, &[])),
            )
            .await
            .unwrap();
    }
    test
}

fn keywords() -> RetrieverContainer {
    Retriever::standard(Query::r#match("title", "vector search"))
}

fn neighbours() -> RetrieverContainer {
    Retriever::knn("embedding", vec![1.0, 0.2, 0.3], 6, 20)
}

/// Each hit's score by id, checking they come best first.
fn scores(hits: &[Hit<Article>]) -> BTreeMap<String, f64> {
    let scores: Vec<f64> = hits.iter().map(|hit| hit.score.unwrap()).collect();
    assert!(
        scores.windows(2).all(|pair| pair[0] >= pair[1]),
        "hits best first: {scores:?}"
    );
    hits.iter()
        .map(|hit| (hit.id.clone().unwrap(), hit.score.unwrap()))
        .collect()
}

/// The same documents with the same scores, to within the server's 32-bit
/// floats.
fn assert_close(name: &str, server: &BTreeMap<String, f64>, client: &BTreeMap<String, f64>) {
    assert_eq!(
        server.keys().collect::<Vec<_>>(),
        client.keys().collect::<Vec<_>>(),
        "{name}: the same documents"
    );
    for (id, score) in server {
        let other = client[id];
        assert!(
            (score - other).abs() <= 1e-4 * score.abs().max(1.0),
            "{name}: document {id} scored {score} on the server, {other} on the client"
        );
    }
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn client_fusion_matches_the_server() {
    let client = client();
    let test = passages(&client).await;
    // Room for every document either retriever finds, so no ties are cut.
    let fusions = [
        ("rrf", Fusion::rrf([keywords(), neighbours()])),
        (
            "weighted rrf",
            Fusion::weighted_rrf([(keywords(), 1.0), (neighbours(), 3.0)]).rank_constant(10),
        ),
        (
            "linear",
            Fusion::linear([(keywords(), 1.0), (neighbours(), 2.0)]),
        ),
        (
            "linear, min-max",
            Fusion::linear([(keywords(), 1.0), (neighbours(), 2.0)])
                .normalizer(ScoreNormalizer::Minmax),
        ),
        (
            "linear, l2",
            Fusion::linear([(keywords(), 1.0), (neighbours(), 2.0)])
                .normalizer(ScoreNormalizer::L2Norm),
        ),
    ];
    for (name, fusion) in fusions {
        let fusion = fusion.rank_window_size(12).size(12);
        let on_client = fusion
            .on_client::<Article, _>(&client, test.name())
            .await
            .unwrap_or_else(|e| panic!("{name} on the client: {e}"));
        let on_client = scores(&on_client);
        assert!(
            on_client.len() > 6,
            "{name}: the lists overlap only in part"
        );
        let fused = fusion
            .search::<Article, _>(&client, test.name())
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        match License::from_env() {
            License::Basic => {
                let why = fused.fallback.expect("Basic doesn't include fusion");
                assert!(!why.feature.is_empty());
                assert_eq!(scores(&fused.hits), on_client, "{name}");
            }
            License::Trial => {
                assert!(fused.fallback.is_none(), "{name}: fused on the server");
                assert_close(name, &scores(&fused.hits), &on_client);
            }
        }
    }
    test.delete(&client).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn fused_hits_carry_their_documents() {
    let client = client();
    let test = passages(&client).await;
    let fused = Fusion::rrf([keywords(), neighbours()])
        .size(3)
        .on_client::<Article, _>(&client, test.name())
        .await
        .unwrap();
    assert_eq!(fused.len(), 3);
    for hit in &fused {
        let article = hit.source.as_ref().expect("a hit has its source");
        assert!(TITLES.contains(&article.title.as_str()));
    }
    test.delete(&client).await;
}
