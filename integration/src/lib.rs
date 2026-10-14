//! Helpers for the integration tests, which run against the Elasticsearch
//! fixture.
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

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

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

/// A response's status and JSON body (`Value::Null` when it has none).
#[derive(Debug)]
pub struct Response {
    /// The HTTP status code.
    pub status: u16,
    /// The body, parsed as JSON.
    pub body: Value,
}

/// The fixture cluster, reached over plain HTTP.
pub struct Cluster {
    url: String,
    agent: ureq::Agent,
}

impl Cluster {
    /// The cluster at `OXSEARCH_TEST_URL`, or `http://127.0.0.1:9200`.
    pub fn from_env() -> Cluster {
        let url = std::env::var("OXSEARCH_TEST_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:9200".to_owned());
        let agent = ureq::Agent::config_builder()
            // Error statuses are what some tests check for.
            .http_status_as_error(false)
            .build()
            .into();
        Cluster {
            url: url.trim_end_matches('/').to_owned(),
            agent,
        }
    }

    /// `GET path`.
    pub fn get(&self, path: &str) -> Response {
        self.finish(path, self.agent.get(self.url(path)).call())
    }

    /// `POST path` with a JSON body.
    pub fn post(&self, path: &str, body: &Value) -> Response {
        self.finish(path, self.agent.post(self.url(path)).send_json(body))
    }

    /// `POST path` with no body.
    pub fn post_empty(&self, path: &str) -> Response {
        self.finish(path, self.agent.post(self.url(path)).send_empty())
    }

    /// `PUT path` with a JSON body.
    pub fn put(&self, path: &str, body: &Value) -> Response {
        self.finish(path, self.agent.put(self.url(path)).send_json(body))
    }

    /// `DELETE path`.
    pub fn delete(&self, path: &str) -> Response {
        self.finish(path, self.agent.delete(self.url(path)).call())
    }

    /// Creates an index with a name no other test uses, from a create-index
    /// body (settings and mappings). The index is deleted when the returned
    /// guard drops, even if the test fails.
    pub fn create_index(&self, prefix: &str, body: &Value) -> TestIndex<'_> {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock is after 1970")
            .as_nanos();
        let name = format!(
            "oxsearch-test-{prefix}-{nanos}-{}",
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let created = self.put(&format!("/{name}"), body);
        assert_eq!(created.status, 200, "creating {name}: {}", created.body);
        TestIndex {
            cluster: self,
            name,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.url)
    }

    fn finish(
        &self,
        path: &str,
        result: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Response {
        let mut response = result.unwrap_or_else(|e| {
            panic!(
                "{}{path}: {e} (is the fixture up? scripts/es-fixture.sh up)",
                self.url
            )
        });
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .read_to_string()
            .unwrap_or_else(|e| panic!("{}{path}: reading the body: {e}", self.url));
        let body = if text.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text)
                .unwrap_or_else(|e| panic!("{}{path}: the body isn't JSON ({e}): {text}", self.url))
        };
        Response { status, body }
    }
}

/// An index that one test owns, deleted when this drops.
pub struct TestIndex<'a> {
    cluster: &'a Cluster,
    name: String,
}

impl TestIndex<'_> {
    /// The index's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Indexes documents and refreshes, so searches see them.
    pub fn add(&self, documents: &[Value]) {
        for document in documents {
            let indexed = self.cluster.post(&format!("/{}/_doc", self.name), document);
            assert_eq!(
                indexed.status, 201,
                "indexing into {}: {}",
                self.name, indexed.body
            );
        }
        let refreshed = self.cluster.post_empty(&format!("/{}/_refresh", self.name));
        assert_eq!(
            refreshed.status, 200,
            "refreshing {}: {}",
            self.name, refreshed.body
        );
    }

    /// Runs a search request body against this index.
    pub fn search(&self, body: &Value) -> Response {
        self.cluster.post(&format!("/{}/_search", self.name), body)
    }
}

impl Drop for TestIndex<'_> {
    fn drop(&mut self) {
        // Best effort: a cleanup failure shouldn't hide the test's own.
        let _ = self
            .cluster
            .agent
            .delete(self.cluster.url(&format!("/{}", self.name)))
            .call();
    }
}
