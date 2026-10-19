//! Helpers for the integration tests, which run oxsearch against the
//! Elasticsearch fixture.
//!
//! The tests are ignored by default, so `cargo test` passes without a
//! cluster. To run them:
//!
//! ```text
//! scripts/es-fixture.sh up trial
//! OXSEARCH_TEST_LICENSE=trial cargo test -p oxsearch-integration -- --ignored
//! scripts/es-fixture.sh down
//! ```
//!
//! `OXSEARCH_TEST_URL` (default `http://127.0.0.1:9200`) is where the cluster
//! is, and `OXSEARCH_TEST_LICENSE` (`basic`, the default, or `trial`) is the
//! license the tests expect it to run.

pub mod fixtures;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use oxsearch::admin::indices::{create, delete, get_mapping};
use oxsearch_core::{Client, Error};
use oxsearch_mapping::TypeMapping;
use oxsearch_transport_reqwest::ReqwestTransport;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The client the tests use.
pub type TestClient = Client<Recorded<ReqwestTransport>>;

/// A transport that records each request's method and path in the file
/// `OXSEARCH_COVERAGE` names, for the coverage report
/// (`cargo run -p xtask -- coverage`).
#[derive(Clone, Debug)]
pub struct Recorded<T>(pub T);

impl<T: oxsearch_core::Transport> oxsearch_core::Transport for Recorded<T> {
    async fn send(
        &self,
        request: oxsearch_core::HttpRequest,
    ) -> Result<oxsearch_core::HttpResponse, oxsearch_core::TransportError> {
        if let Some(file) = std::env::var_os("OXSEARCH_COVERAGE") {
            use std::io::Write;
            let path = request.path_and_query.split('?').next().unwrap_or_default();
            let line = format!("{} {path}\n", request.method.as_str());
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(file)
            {
                // One write per line, so lines from tests running at once
                // don't interleave.
                let _ = file.write_all(line.as_bytes());
            }
        }
        self.0.send(request).await
    }
}

/// The license the tests expect the cluster to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum License {
    /// The free Basic license.
    Basic,
    /// A trial license, which allows everything a paid license does.
    Trial,
}

impl License {
    /// The license named by `OXSEARCH_TEST_LICENSE`, Basic when unset.
    pub fn from_env() -> License {
        match std::env::var("OXSEARCH_TEST_LICENSE").as_deref() {
            Err(_) | Ok("basic") => License::Basic,
            Ok("trial") => License::Trial,
            Ok(other) => panic!("OXSEARCH_TEST_LICENSE is {other:?}; expected basic or trial"),
        }
    }

    /// The license's type, as `GET /_license` reports it.
    pub fn as_str(self) -> &'static str {
        match self {
            License::Basic => "basic",
            License::Trial => "trial",
        }
    }
}

/// A client for the cluster at `OXSEARCH_TEST_URL`, or
/// `http://127.0.0.1:9200`.
pub fn client() -> TestClient {
    let url =
        std::env::var("OXSEARCH_TEST_URL").unwrap_or_else(|_| "http://127.0.0.1:9200".to_owned());
    Client::new(Recorded(
        ReqwestTransport::new(&url).expect("OXSEARCH_TEST_URL is an http URL"),
    ))
}

/// A document the tests index: text, a keyword, a dense vector and sparse
/// weights.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Article {
    /// The title.
    pub title: String,
    /// A tag.
    pub tag: String,
    /// A three-dimensional embedding. Elasticsearch 9 leaves vectors out of
    /// the `_source` it returns, so it reads back empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub embedding: Vec<f32>,
    /// Sparse weights by token, also left out of returned sources.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub terms: HashMap<String, f32>,
}

impl Article {
    /// An article.
    pub fn new(title: &str, tag: &str, embedding: [f32; 3], terms: &[(&str, f32)]) -> Article {
        Article {
            title: title.to_owned(),
            tag: tag.to_owned(),
            embedding: embedding.to_vec(),
            terms: terms.iter().map(|(t, w)| ((*t).to_owned(), *w)).collect(),
        }
    }
}

/// An index that one test owns. Delete it with [`TestIndex::delete`] at the
/// end of the test; the fixture is thrown away after a run anyway.
pub struct TestIndex {
    name: String,
}

impl TestIndex {
    /// A name no other test uses, for an index the test creates itself.
    pub fn named(prefix: &str) -> TestIndex {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock is after 1970")
            .as_nanos();
        let name = format!(
            "oxsearch-test-{prefix}-{nanos}-{}",
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        TestIndex { name }
    }

    /// Creates an index with a name no other test uses, and the mapping.
    pub async fn create(client: &TestClient, prefix: &str, mapping: &TypeMapping) -> TestIndex {
        let name = TestIndex::named(prefix).name;
        client
            .send(
                &create::Request::new(name.as_str())
                    .body(create::RequestBody::new().mappings(mapping.clone())),
            )
            .await
            .unwrap_or_else(|e| panic!("creating {name}: {e}"));
        TestIndex { name }
    }

    /// The index's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The index's mapping, as Elasticsearch reports it: the JSON, not the
    /// generated types' reading of it.
    pub async fn mapping(&self, client: &TestClient) -> Value {
        let response: Value = client
            .send_as(&get_mapping::Request::new().index(self.name.as_str()))
            .await
            .unwrap_or_else(|e| panic!("reading {}'s mapping: {e}", self.name));
        response[&self.name]["mappings"].clone()
    }

    /// Deletes the index.
    pub async fn delete(self, client: &TestClient) {
        client
            .send(&delete::Request::new(self.name.as_str()))
            .await
            .unwrap_or_else(|e| panic!("deleting {}: {e}", self.name));
    }
}

/// A call to one of Elastic's models, sent again while the model is still
/// deploying. On first use, a model can take longer to start than the 30 s
/// Elasticsearch waits for it (408), and the deployment goes on meanwhile; a
/// call during that time can be refused because the deployment is already
/// starting (409).
pub async fn deployed<T, F>(mut call: impl FnMut() -> F) -> T
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

/// Whether Elasticsearch read a request, even if it couldn't run it: the
/// error isn't one about the request's shape. For features the fixture can't
/// run, such as those that need a machine learning model.
pub fn understood(error: &Error) -> bool {
    match error {
        Error::License(_) => true,
        Error::Elasticsearch(e) => !matches!(
            e.error_type(),
            Some(
                "parsing_exception"
                    | "x_content_parse_exception"
                    | "named_object_not_found_exception"
            )
        ),
        _ => false,
    }
}
