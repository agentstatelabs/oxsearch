//! The README's quick start: semantic search over your own documents. It
//! needs a cluster whose license includes machine learning, such as a
//! trial, for the default ELSER model `semantic_text` uses:
//!
//! ```text
//! scripts/es-fixture.sh up trial
//! cargo run -p oxsearch --example quickstart --features derive
//! ```
//!
//! Its dependencies, in an application of your own:
//!
//! ```toml
//! [dependencies]
//! oxsearch = { version = "0.2", features = ["derive", "transport-reqwest"] }
//! serde = { version = "1", features = ["derive"] }
//! tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
//! ```

use oxsearch::document::{Document, Indexes};
use oxsearch::transport_reqwest::ReqwestTransport;
use oxsearch::{Client, Refresh};
use serde::{Deserialize, Serialize};

#[derive(Document, Serialize, Deserialize, Debug)]
#[index("quickstart-passages")]
struct Passage {
    #[semantic_text]
    text: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = Client::new(ReqwestTransport::new("http://localhost:9200")?);
    let passages = client.index::<Passage>().refresh(Refresh::WaitFor);
    let _ = passages.ensure_mapping().await?;
    passages
        .put(&Passage {
            text: "HNSW graphs find nearest neighbours fast.".into(),
        })
        .await?;
    passages
        .put(&Passage {
            text: "Replicas keep copies of shards on other nodes.".into(),
        })
        .await?;
    for hit in passages
        .search(Passage::TEXT.semantic("how do vector indexes work?"))
        .await?
    {
        println!("{:.2} {:?}", hit.score.unwrap_or_default(), hit.source);
    }
    Ok(())
}
