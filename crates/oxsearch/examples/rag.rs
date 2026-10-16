//! The retrieval half of a RAG application: index passages with their
//! embeddings, find the ones that answer a question by fusing a keyword
//! match with nearest neighbours, and count the passages by topic.
//!
//! It runs against Elasticsearch at `OXSEARCH_URL` (default
//! `http://localhost:9200`), such as the test fixture:
//!
//! ```text
//! scripts/es-fixture.sh up
//! cargo run -p oxsearch --example rag --features admin,transport-reqwest
//! ```
//!
//! On the Basic license, which doesn't include server-side RRF, the fusion
//! runs on the client instead. The embeddings are a toy stand-in for an
//! embedding model, so the example needs nothing but Elasticsearch.

use oxsearch::admin::indices::{create, delete};
use oxsearch::docs::bulk;
use oxsearch::fusion::Fusion;
use oxsearch::mapping::{
    DenseVectorProperty, DenseVectorSimilarity, KeywordProperty, Property, TextProperty,
    TypeMapping,
};
use oxsearch::search::search;
use oxsearch::shapes::IndexMap;
use oxsearch::transport_reqwest::ReqwestTransport;
use oxsearch::{Client, Query, Refresh, Retriever};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// A passage of a document, as the application stores it.
#[derive(Debug, Serialize, Deserialize)]
struct Passage {
    text: String,
    topic: String,
    /// Elasticsearch 9 leaves vectors out of the sources it returns.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    embedding: Vec<f32>,
}

/// The aggregation results the application asks for.
#[derive(Debug, Deserialize)]
struct Topics {
    by_topic: Terms,
}

#[derive(Debug, Deserialize)]
struct Terms {
    buckets: Vec<Bucket>,
}

#[derive(Debug, Deserialize)]
struct Bucket {
    key: String,
    doc_count: u64,
}

/// A stand-in for an embedding model: how much a text is about vectors,
/// ingestion and operations.
fn embed(text: &str) -> Vec<f32> {
    let text = text.to_lowercase();
    let score = |words: &[&str]| words.iter().filter(|w| text.contains(*w)).count() as f32 + 0.1;
    let vector = [
        score(&["vector", "hnsw", "embedding", "neighbour", "quantiz"]),
        score(&["bulk", "ingest", "index", "throughput"]),
        score(&["shard", "replica", "node", "cluster"]),
    ];
    let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
    vector.iter().map(|x| x / norm).collect()
}

const INDEX: &str = "oxsearch-example-rag";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let url = std::env::var("OXSEARCH_URL").unwrap_or_else(|_| "http://localhost:9200".to_owned());
    let client = Client::new(ReqwestTransport::new(&url)?);

    // The mapping, from the generated types. Creating an index isn't in the
    // first slice's crates yet, so it goes through the raw call.
    let mut properties = IndexMap::new();
    properties.insert("text".to_owned(), Property::Text(TextProperty::new()));
    properties.insert(
        "topic".to_owned(),
        Property::Keyword(KeywordProperty::new()),
    );
    properties.insert(
        "embedding".to_owned(),
        Property::DenseVector(
            DenseVectorProperty::new()
                .dims(3)
                .similarity(DenseVectorSimilarity::Cosine),
        ),
    );
    let _ = client.send(&delete::Request::new(INDEX)).await;
    let mapping = TypeMapping::new().properties(properties);
    client
        .send(&create::Request::new(INDEX).body(create::RequestBody::new().mappings(mapping)))
        .await?;

    // Index the passages in one bulk request.
    let passages = [
        (
            "HNSW graphs find approximate nearest neighbours of a vector quickly.",
            "vectors",
        ),
        (
            "Quantizing embeddings to int8 cuts a vector index's memory by four.",
            "vectors",
        ),
        (
            "Bulk requests index thousands of documents per round trip.",
            "ingest",
        ),
        (
            "Each shard has replicas on other nodes of the cluster.",
            "operations",
        ),
    ];
    let mut lines: Vec<bulk::RequestBody<Passage>> = Vec::new();
    for (n, (text, topic)) in passages.iter().enumerate() {
        lines.push(bulk::RequestBody::OperationContainer(
            bulk::OperationContainer::Index(
                bulk::IndexOperation::new().index(INDEX).id(n.to_string()),
            ),
        ));
        lines.push(bulk::RequestBody::TDocument(Passage {
            text: (*text).to_owned(),
            topic: (*topic).to_owned(),
            embedding: embed(text),
        }));
    }
    let indexed = client
        .send(&bulk::Request::new().refresh(Refresh::True).body(lines))
        .await?;
    println!("indexed {} passages", indexed.items.len());

    // Retrieve for a question: a keyword match and nearest neighbours,
    // fused with RRF, on the server if its license includes it.
    let question = "How do vector indexes find nearest neighbours?";
    let fused = Fusion::rrf([
        Retriever::standard(Query::r#match("text", question)),
        Retriever::knn("embedding", embed(question), 3, 10),
    ])
    .size(3)
    .search::<Passage, _>(&client, INDEX)
    .await?;
    if let Some(why) = &fused.fallback {
        println!("{why}: fused on the client");
    }
    println!("\n{question}");
    for hit in &fused.hits {
        if let Some(passage) = &hit.source {
            println!(
                "  {:.3}  [{}] {}",
                hit.score.unwrap_or_default(),
                passage.topic,
                passage.text
            );
        }
    }

    // Count the passages by topic.
    let count = search::Request::new().index(INDEX).body(
        search::RequestBody::new()
            .size(0)
            .aggregations(json!({ "by_topic": { "terms": { "field": "topic" } } })),
    );
    let response: search::Response<Passage, Topics> = client.send_as(&count).await?;
    if let Some(topics) = &response.aggregations {
        let counts: Vec<String> = topics
            .by_topic
            .buckets
            .iter()
            .map(|b| format!("{} {}", b.key, b.doc_count))
            .collect();
        println!("by topic: {}", counts.join(", "));
    }

    client.send(&delete::Request::new(INDEX)).await?;
    Ok(())
}
