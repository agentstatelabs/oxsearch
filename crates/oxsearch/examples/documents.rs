//! The typed document layer, on any license: a struct with an id, text,
//! keywords and an embedding is the document type, and `Index<Article>`
//! creates its index, writes, reads and deletes articles, and searches
//! them with queries its field handles build.
//!
//! It runs against Elasticsearch at `OXSEARCH_URL` (default
//! `http://localhost:9200`), such as the test fixture:
//!
//! ```text
//! scripts/es-fixture.sh up
//! cargo run -p oxsearch --example documents --features derive
//! ```
//!
//! Its dependencies, in an application of your own:
//!
//! ```toml
//! [dependencies]
//! oxsearch = { version = "0.3", features = ["derive"] }
//! serde = { version = "1", features = ["derive"] }
//! tokio = { version = "1", features = ["macros", "rt"] }
//! ```

use oxsearch::document::{Document, Indexes};
use oxsearch::fusion::Fusion;
use oxsearch::transport_reqwest::ReqwestTransport;
use oxsearch::{Client, Refresh, Retriever};
use serde::{Deserialize, Serialize};

// ANCHOR: type
/// An article. Each field's mapped type comes from an attribute, or else
/// from its Rust type: `title` is text, being a `String`.
#[derive(Document, Serialize, Deserialize, Debug, Clone, PartialEq)]
#[index("oxsearch-example-articles")]
struct Article {
    /// The document's id in the index.
    #[id]
    slug: String,
    title: String,
    #[keyword]
    tags: Vec<String>,
    #[dense_vector(dims = 3, similarity = "cosine")]
    embedding: Vec<f32>,
}
// ANCHOR_END: type

fn article(slug: &str, title: &str, tags: &[&str], embedding: [f32; 3]) -> Article {
    Article {
        slug: slug.to_owned(),
        title: title.to_owned(),
        tags: tags.iter().map(|t| (*t).to_owned()).collect(),
        embedding: embedding.to_vec(),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let url = std::env::var("OXSEARCH_URL").unwrap_or_else(|_| "http://localhost:9200".to_owned());
    let client = Client::new(ReqwestTransport::new(&url)?);

    // ANCHOR: write
    let articles = client.index::<Article>().refresh(Refresh::WaitFor);
    let _ = articles.delete_index().await;
    let report = articles.ensure_mapping().await?;
    println!(
        "created: {}, fields added: {:?}",
        report.created, report.added
    );
    for one in [
        article(
            "hnsw",
            "Tuning HNSW graphs",
            &["vectors", "rust"],
            [0.9, 0.1, 0.1],
        ),
        article(
            "bulk",
            "Bulk loading at speed",
            &["ingest"],
            [0.1, 0.9, 0.1],
        ),
        article("shards", "Sizing shards", &["operations"], [0.1, 0.1, 0.9]),
    ] {
        articles.put(&one).await?;
    }
    // ANCHOR_END: write

    // ANCHOR: read
    let hnsw = articles
        .get("hnsw")
        .await?
        .expect("the article was written");
    println!(
        "{} at version {:?}: {}",
        hnsw.id, hnsw.version, hnsw.document.title
    );
    // ANCHOR_END: read

    // ANCHOR: search
    // A term query on a keyword field, built from the field's handle.
    let tagged = articles
        .search(Retriever::standard(Article::TAGS.term("vectors")))
        .await?;
    // Nearest neighbours of a vector: 2 of them, from 10 candidates.
    let near = articles
        .search(Article::EMBEDDING.knn(vec![0.8, 0.2, 0.1], 2, 10))
        .await?;
    // Both fused with RRF, on the server if the license includes it, and
    // on the client if it doesn't.
    let fused = articles
        .fuse(&Fusion::rrf([
            Retriever::standard(Article::TITLE.r#match("graphs")),
            Article::EMBEDDING.knn(vec![0.8, 0.2, 0.1], 2, 10),
        ]))
        .await?;
    // ANCHOR_END: search
    for (what, hits) in [("tagged", &tagged), ("near", &near), ("fused", &fused.hits)] {
        let titles: Vec<&str> = hits
            .iter()
            .filter_map(|hit| hit.source.as_ref())
            .map(|a| a.title.as_str())
            .collect();
        println!("{what}: {titles:?}");
    }
    assert_eq!(tagged.len(), 1);

    // ANCHOR: delete
    assert!(articles.delete("bulk").await?);
    assert!(articles.get("bulk").await?.is_none());
    articles.delete_index().await?;
    // ANCHOR_END: delete
    Ok(())
}
