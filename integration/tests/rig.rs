//! oxsearch-rig against Elasticsearch: documents inserted through rig's
//! `InsertDocuments`, found through `VectorStoreIndex` by kNN with filters,
//! a threshold and hybrid fusion, and on a trial by a `semantic_text`
//! field.

use oxsearch::Refresh;
use oxsearch_integration::{License, TestIndex, client};
use oxsearch_rig::{ElasticsearchIndex, Filter};
use rig_core::driver::{Exchange, Local, Model, Opened, Opening, Step, Transport};
use rig_core::embeddings::embed::{EmbedError, TextEmbedder};
use rig_core::embeddings::{Embed, Embedding, EmbeddingResponse};
use rig_core::operation::Embedding as EmbeddingOp;
use rig_core::vector_store::request::{SearchFilter, VectorSearchRequest};
use rig_core::vector_store::{InsertDocuments, VectorStoreIndex};
use rig_core::wire::Capabilities;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// A stand-in for an embedding model: how much a text is about vectors,
/// ingestion and operations.
#[derive(Clone)]
struct Toy;

fn embed(text: &str) -> Vec<f64> {
    let text = text.to_lowercase();
    let score = |words: &[&str]| words.iter().filter(|w| text.contains(*w)).count() as f64 + 0.1;
    let vector = [
        score(&["vector", "hnsw", "embedding", "neighbour"]),
        score(&["bulk", "ingest", "index"]),
        score(&["shard", "replica", "node"]),
    ];
    let norm = vector.iter().map(|x| x * x).sum::<f64>().sqrt();
    vector.iter().map(|x| x / norm).collect()
}

impl Transport<Local<EmbeddingOp>> for Toy {
    fn send(&self, texts: Vec<String>, _: Exchange) -> Opening<Step<EmbeddingOp>> {
        let embeddings = texts
            .into_iter()
            .map(|text| Embedding {
                vec: embed(&text),
                document: text,
            })
            .collect();
        let response = EmbeddingResponse::new(embeddings);
        Opening::ready(Opened::new(futures_util::stream::iter([Ok(Step::End(
            response,
        ))])))
    }
}

fn model() -> Model<Local<EmbeddingOp>, Toy> {
    Model::new(
        Local::new("toy").with_capabilities(Capabilities::embedding(16, 3)),
        Toy,
    )
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Passage {
    text: String,
    topic: String,
}

impl Embed for Passage {
    fn embed(&self, embedder: &mut TextEmbedder) -> Result<(), EmbedError> {
        embedder.embed(self.text.clone());
        Ok(())
    }
}

fn passages() -> Vec<Passage> {
    [
        (
            "HNSW graphs find a vector's nearest neighbour quickly.",
            "vectors",
        ),
        (
            "Embedding vectors with int8 quantization saves memory.",
            "vectors",
        ),
        (
            "Bulk requests ingest documents into an index fast.",
            "ingest",
        ),
        ("Each shard has a replica on another node.", "operations"),
    ]
    .into_iter()
    .map(|(text, topic)| Passage {
        text: text.to_owned(),
        topic: topic.to_owned(),
    })
    .collect()
}

fn request(query: &str, samples: u64) -> VectorSearchRequest<Filter> {
    VectorSearchRequest::builder()
        .query(query)
        .samples(samples)
        .build()
        .map_filter(|f: rig_core::vector_store::request::Filter<serde_json::Value>| f.interpret())
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn knn_through_rig() {
    let client = client();
    let test = TestIndex::named("rig");
    client
        .send_raw(
            oxsearch::Method::Put,
            &format!("/{}", test.name()),
            Some(&json!({"mappings": {"properties": {
                "text": {"type": "text"},
                "topic": {"type": "keyword"},
                "embedding": {"type": "dense_vector", "dims": 3, "similarity": "cosine"}
            }}})),
        )
        .await
        .unwrap();
    let index = ElasticsearchIndex::knn(client.clone(), test.name(), "embedding", model())
        .refresh(Refresh::WaitFor);

    let documents: Vec<(Passage, Vec<Embedding>)> = passages()
        .into_iter()
        .map(|p| {
            let embedding = Embedding {
                vec: embed(&p.text),
                document: p.text.clone(),
            };
            (p, vec![embedding])
        })
        .collect();
    index.insert_documents(documents).await.unwrap();

    let found = index
        .top_n::<Passage>(request("nearest neighbour of a vector", 2))
        .await
        .unwrap();
    assert_eq!(found.len(), 2);
    assert!(
        found.iter().all(|r| r.document.topic == "vectors"),
        "{found:?}"
    );
    assert!(found[0].score >= found[1].score);

    // Filters narrow the candidates.
    let filtered = VectorSearchRequest::builder()
        .query("nearest neighbour of a vector")
        .samples(4)
        .filter(Filter::eq("topic", json!("operations")))
        .build();
    let found = index.top_n::<Passage>(filtered).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].document.topic, "operations");

    // A threshold drops weak matches: (1 + cosine) / 2 above 0.95.
    let strict = VectorSearchRequest::builder()
        .query("vector embedding")
        .samples(4)
        .threshold(0.95)
        .build()
        .map_filter(
            |f: rig_core::vector_store::request::Filter<serde_json::Value>| f.interpret::<Filter>(),
        );
    let found = index.top_n_ids(strict).await.unwrap();
    assert!(!found.is_empty() && found.len() < 4, "{found:?}");
    assert!(found.iter().all(|r| r.score >= 0.95));

    // Hybrid: kNN and a keyword match, fused by RRF, on the client on
    // Basic.
    let hybrid =
        ElasticsearchIndex::knn(client.clone(), test.name(), "embedding", model()).hybrid("text");
    let found = hybrid
        .top_n::<Passage>(request("bulk ingest", 2))
        .await
        .unwrap();
    assert_eq!(found[0].document.topic, "ingest");
    test.delete(&client).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn semantic_text_through_rig() {
    if License::from_env() != License::Trial {
        // ELSER, which semantic_text uses by default, needs a trial.
        return;
    }
    let client = client();
    let test = TestIndex::named("rig-semantic");
    client
        .send_raw(
            oxsearch::Method::Put,
            &format!("/{}", test.name()),
            Some(&json!({"mappings": {"properties": {
                "text": {"type": "semantic_text"},
                "topic": {"type": "keyword"}
            }}})),
        )
        .await
        .unwrap();
    let index =
        ElasticsearchIndex::semantic(client.clone(), test.name(), "text").refresh(Refresh::WaitFor);
    let documents: Vec<(Passage, Vec<Embedding>)> =
        passages().into_iter().map(|p| (p, Vec::new())).collect();
    index.insert_documents(documents).await.unwrap();
    let found = index
        .top_n::<Passage>(request(
            "how do replicas keep data safe across machines?",
            1,
        ))
        .await
        .unwrap();
    assert_eq!(found[0].document.topic, "operations");
    test.delete(&client).await;
}
