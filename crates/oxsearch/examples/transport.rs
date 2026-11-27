//! A transport for a cluster in production: several nodes with failover,
//! retries with backoff, sniffing for the cluster's other nodes, and
//! credentials from the environment.
//!
//! It runs against the nodes in `OXSEARCH_URL`, separated by commas
//! (default `http://localhost:9200`), such as the test fixture, with the API
//! key in `OXSEARCH_API_KEY` if there is one:
//!
//! ```text
//! scripts/es-fixture.sh up
//! cargo run -p oxsearch --example transport
//! ```
//!
//! Its dependencies, in an application of your own:
//!
//! ```toml
//! [dependencies]
//! oxsearch = { version = "0.2", features = ["transport-reqwest"] }
//! tokio = { version = "1", features = ["macros", "rt"] }
//! ```

use std::time::Duration;

use oxsearch::auth::Auth;
use oxsearch::transport_reqwest::ReqwestTransport;
use oxsearch::{Client, info, ping};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let urls = std::env::var("OXSEARCH_URL").unwrap_or_else(|_| "http://localhost:9200".to_owned());

    // ANCHOR: builder
    // Requests go to the nodes in turn. One that fails rests, and is tried
    // again later; a request it couldn't answer goes to the next node.
    let mut builder = ReqwestTransport::builder()
        .nodes(urls.split(','))
        .retries(3)
        .backoff(Duration::from_millis(100), Duration::from_secs(5))
        .sniff_on_failure(true);
    // An API key, encoded as Elasticsearch gives it, on every request.
    if let Ok(key) = std::env::var("OXSEARCH_API_KEY") {
        builder = builder.auth(Auth::api_key(key));
    }
    let transport = builder.build()?;
    println!("nodes: {:?}", transport.nodes());
    let client = Client::new(transport);
    // ANCHOR_END: builder

    // ANCHOR: calls
    // `ping` answers whether the cluster is there; `info` says what it is.
    let up = client.send(&ping::Request::new()).await.is_ok();
    let info = client.send(&info::Request::new()).await?;
    println!(
        "cluster {:?} is up: {up}; Elasticsearch {:?}",
        info.cluster_name.as_deref().unwrap_or("?"),
        info.version.as_ref().and_then(|v| v.number.as_deref())
    );
    // ANCHOR_END: calls
    Ok(())
}
