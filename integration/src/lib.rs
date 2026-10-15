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

use oxsearch_core::{Client, Error, Method};
use oxsearch_mapping::TypeMapping;
use oxsearch_transport_reqwest::ReqwestTransport;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The client the tests use.
pub type TestClient = Client<ReqwestTransport>;

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
    Client::new(ReqwestTransport::new(&url).expect("OXSEARCH_TEST_URL is an http URL"))
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

    /// Creates an index with a name no other test uses. Index creation isn't
    /// in the first slice's crates, so it goes through the raw call, with a
    /// mapping built from the generated types.
    pub async fn create(client: &TestClient, prefix: &str, mapping: &TypeMapping) -> TestIndex {
        let name = TestIndex::named(prefix).name;
        client
            .send_raw(
                Method::Put,
                &format!("/{name}"),
                Some(&json!({ "mappings": mapping })),
            )
            .await
            .unwrap_or_else(|e| panic!("creating {name}: {e}"));
        TestIndex { name }
    }

    /// The index's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The index's mapping, as Elasticsearch reports it.
    pub async fn mapping(&self, client: &TestClient) -> Value {
        let response = client
            .send_raw(Method::Get, &format!("/{}/_mapping", self.name), None)
            .await
            .unwrap_or_else(|e| panic!("reading {}'s mapping: {e}", self.name));
        response[&self.name]["mappings"].clone()
    }

    /// Deletes the index.
    pub async fn delete(self, client: &TestClient) {
        client
            .send_raw(Method::Delete, &format!("/{}", self.name), None)
            .await
            .unwrap_or_else(|e| panic!("deleting {}: {e}", self.name));
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
