//! The default HTTP transport for oxsearch, built on `reqwest`.
//!
//! ```no_run
//! # async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! use std::time::Duration;
//!
//! use oxsearch_core::Client;
//! use oxsearch_core::auth::Auth;
//! use oxsearch_transport_reqwest::ReqwestTransport;
//!
//! // One node, each request sent once.
//! let client = Client::new(ReqwestTransport::new("http://localhost:9200")?);
//!
//! // Three nodes, with failover, retries, sniffing and an API key.
//! let transport = ReqwestTransport::builder()
//!     .nodes(["https://es1:9200", "https://es2:9200", "https://es3:9200"])
//!     .auth(Auth::api_key("VnVhQ2ZHY0JDZGJrU…"))
//!     .retries(3)
//!     .sniff_every(Duration::from_secs(300))
//!     .build()?;
//! let client = Client::new(transport);
//! # Ok(())
//! # }
//! ```
//!
//! Each layer is opt-in:
//!
//! - **Nodes and failover.** Requests go to the nodes in turn. A node that
//!   fails rests, for a minute after its first failure and twice as long
//!   after each one in a row, up to half an hour; then it's tried again.
//!   When every node rests, the one resting least is tried.
//! - **Retries** (`retries`, none by default). A request that couldn't
//!   reach a node, or that a node answered with 502, 503 or 504, goes to
//!   the next node. One answered with 429 goes again after a growing
//!   backoff, as does any retry once every node rests. Timeouts are retried
//!   only with `retry_on_timeout`, since the node may have acted on the
//!   request.
//! - **Sniffing** (`sniff_every`, `sniff_on_failure`): the node list is
//!   refreshed from the cluster's own list of HTTP addresses.
//! - **Authentication** (`auth`): basic, bearer or API key, on every
//!   request, sniffing included. For credentials that refresh, wrap the
//!   transport in [`oxsearch_core::auth::Authenticated`].
//! - **Elastic Cloud** (`cloud_id`): the deployment's URL from its Cloud
//!   ID.
//! - **Compression** (`gzip`, behind the `gzip` feature): request bodies
//!   are compressed, and responses may be.
//!
//! Timeouts, proxies and certificates are the `reqwest::Client`'s: build
//! one and pass it with `client`.

mod pool;

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use oxsearch_core::auth::{Auth, cloud_url};
use oxsearch_core::{HttpRequest, HttpResponse, Method, Transport, TransportError};
use pool::{Pool, published_url};

/// A transport that sends requests over HTTP with `reqwest`, to one node
/// or several. Clones share their nodes.
#[derive(Clone)]
pub struct ReqwestTransport {
    shared: Arc<Shared>,
}

struct Shared {
    client: reqwest::Client,
    settings: Settings,
    state: Mutex<State>,
}

struct Settings {
    scheme: String,
    auth: Option<Auth>,
    retries: u32,
    backoff: (Duration, Duration),
    retry_on_timeout: bool,
    sniff_every: Option<Duration>,
    sniff_on_failure: bool,
    gzip: bool,
}

struct State {
    pool: Pool,
    /// When the nodes were last sniffed; `None` asks for a sniff before the
    /// next request.
    sniffed: Option<Instant>,
    /// A request is sniffing, so others don't.
    sniffing: bool,
}

/// Builds a [`ReqwestTransport`]: see the crate's documentation for what
/// each setting does.
#[must_use]
pub struct Builder {
    client: Option<reqwest::Client>,
    nodes: Vec<String>,
    settings: Settings,
    rest: (Duration, Duration),
}

impl ReqwestTransport {
    /// A transport to the node at `url`, such as `http://localhost:9200`,
    /// sending each request once.
    pub fn new(url: &str) -> Result<ReqwestTransport, TransportError> {
        ReqwestTransport::builder().node(url).build()
    }

    /// A transport using a `reqwest` client you configured, for timeouts,
    /// proxies or certificates.
    pub fn with_client(
        client: reqwest::Client,
        url: &str,
    ) -> Result<ReqwestTransport, TransportError> {
        ReqwestTransport::builder().client(client).node(url).build()
    }

    /// A builder, for several nodes, retries, sniffing, authentication or
    /// compression.
    pub fn builder() -> Builder {
        Builder {
            client: None,
            nodes: Vec::new(),
            settings: Settings {
                scheme: "http".to_owned(),
                auth: None,
                retries: 0,
                backoff: (Duration::from_millis(100), Duration::from_secs(10)),
                retry_on_timeout: false,
                sniff_every: None,
                sniff_on_failure: false,
                gzip: false,
            },
            rest: (Duration::from_secs(60), Duration::from_secs(30 * 60)),
        }
    }

    /// The nodes requests go to now.
    pub fn nodes(&self) -> Vec<String> {
        self.state().pool.urls()
    }

    fn state(&self) -> MutexGuard<'_, State> {
        // The lock is held only between awaits, never across one; a panic
        // while holding it leaves a pool that's still usable.
        self.shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Sends a request to one node, as is.
    async fn send_to(
        &self,
        node: &str,
        request: &HttpRequest,
        body: Option<&bytes::Bytes>,
    ) -> Result<HttpResponse, reqwest::Error> {
        let method = method(request.method).unwrap_or(reqwest::Method::GET);
        let settings = &self.shared.settings;
        let mut builder = self
            .shared
            .client
            .request(method, format!("{node}{}", request.path_and_query));
        if let Some(auth) = &settings.auth {
            builder = builder.header(reqwest::header::AUTHORIZATION, auth.header());
        }
        for (name, value) in &request.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        if let (Some(content), Some(body)) = (&request.body, body) {
            builder = builder
                .header(reqwest::header::CONTENT_TYPE, content.content_type())
                .body(body.clone());
            if settings.gzip {
                builder = builder.header(reqwest::header::CONTENT_ENCODING, "gzip");
            }
        }
        let response = builder.send().await?;
        let status = response.status().as_u16();
        let body = response.bytes().await?.to_vec();
        Ok(HttpResponse::new(status, body))
    }

    /// Refreshes the nodes from the cluster's list, if it's time and no
    /// other request is sniffing.
    async fn sniff_if_due(&self) {
        let settings = &self.shared.settings;
        if settings.sniff_every.is_none() && !settings.sniff_on_failure {
            return;
        }
        let nodes = {
            let mut state = self.state();
            let due = match (state.sniffed, settings.sniff_every) {
                // Never sniffed, or a failure asked for it.
                (None, _) => true,
                (Some(at), Some(every)) => at.elapsed() >= every,
                (Some(_), None) => false,
            };
            if !due || state.sniffing {
                return;
            }
            state.sniffing = true;
            state.pool.in_turn(Instant::now())
        };
        // Done sniffing even if this request is dropped mid-sniff.
        struct Sniffing<'a>(&'a ReqwestTransport, Option<Vec<String>>);
        impl Drop for Sniffing<'_> {
            fn drop(&mut self) {
                let mut state = self.0.state();
                state.sniffing = false;
                state.sniffed = Some(Instant::now());
                if let Some(urls) = self.1.take() {
                    state.pool.replace(urls);
                }
            }
        }
        let mut sniffing = Sniffing(self, None);
        // The first node that answers lists them; one that can't rests.
        for node in nodes {
            match self.sniff(&node).await {
                Some(urls) => {
                    sniffing.1 = Some(urls);
                    break;
                }
                None => self.state().pool.failed(&node, Instant::now()),
            }
        }
    }

    /// The nodes with HTTP that a node lists, or `None` if it couldn't.
    async fn sniff(&self, node: &str) -> Option<Vec<String>> {
        let request = HttpRequest::new(
            Method::Get,
            "/_nodes/http?filter_path=nodes.*.http.publish_address",
            None,
        );
        let response = self.send_to(node, &request, None).await.ok()?;
        if response.status != 200 {
            return None;
        }
        let body: serde_json::Value = serde_json::from_slice(&response.body).ok()?;
        let scheme = &self.shared.settings.scheme;
        let mut urls: Vec<String> = body
            .get("nodes")?
            .as_object()?
            .values()
            .filter_map(|n| n.pointer("/http/publish_address")?.as_str())
            .filter_map(|address| published_url(scheme, address))
            .collect();
        urls.sort();
        urls.dedup();
        Some(urls)
    }
}

impl Transport for ReqwestTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        if method(request.method).is_none() {
            return Err(format!("{} isn't supported", request.method.as_str()).into());
        }
        self.sniff_if_due().await;
        let settings = &self.shared.settings;
        let body = match &request.body {
            Some(body) if settings.gzip => Some(gzip(body.bytes())?),
            Some(body) => Some(bytes::Bytes::copy_from_slice(body.bytes())),
            None => None,
        };
        let mut backoff = settings.backoff.0;
        let mut attempt = 0;
        loop {
            let node = self.state().pool.pick(Instant::now());
            let outcome = self.send_to(&node, &request, body.as_ref()).await;
            let last = attempt >= settings.retries;
            let now = Instant::now();
            // Whether to go again, and whether to wait first.
            let wait = match outcome {
                Ok(response) if matches!(response.status, 502..=504) && !last => {
                    self.failed(&node, now);
                    false
                }
                Ok(response) if response.status == 429 && !last => {
                    self.state().pool.answered(&node);
                    true
                }
                Ok(response) => {
                    self.state().pool.answered(&node);
                    return Ok(response);
                }
                Err(error) => {
                    let retry = error.is_connect()
                        || error.is_request() && !error.is_timeout()
                        || error.is_timeout() && settings.retry_on_timeout;
                    if !error.is_timeout() || settings.retry_on_timeout {
                        self.failed(&node, now);
                    }
                    if last || !retry {
                        return Err(Box::new(error));
                    }
                    false
                }
            };
            attempt += 1;
            if wait || !self.state().pool.any_alive(Instant::now()) {
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(settings.backoff.1);
            }
        }
    }
}

impl ReqwestTransport {
    /// A node failed: it rests, and the nodes are sniffed again if asked.
    fn failed(&self, node: &str, now: Instant) {
        let mut state = self.state();
        state.pool.failed(node, now);
        if self.shared.settings.sniff_on_failure {
            state.sniffed = None;
        }
    }
}

impl fmt::Debug for ReqwestTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReqwestTransport")
            .field("nodes", &self.nodes())
            .field("auth", &self.shared.settings.auth)
            .field("retries", &self.shared.settings.retries)
            .finish_non_exhaustive()
    }
}

impl Builder {
    /// A node, such as `http://localhost:9200`.
    pub fn node(mut self, url: impl Into<String>) -> Builder {
        self.nodes.push(url.into());
        self
    }

    /// Several nodes.
    pub fn nodes<U: Into<String>>(mut self, urls: impl IntoIterator<Item = U>) -> Builder {
        self.nodes.extend(urls.into_iter().map(Into::into));
        self
    }

    /// The Elastic Cloud deployment a Cloud ID names.
    pub fn cloud_id(mut self, cloud_id: &str) -> Result<Builder, TransportError> {
        self.nodes.push(cloud_url(cloud_id)?);
        Ok(self)
    }

    /// The `reqwest` client, for timeouts, proxies or certificates.
    pub fn client(mut self, client: reqwest::Client) -> Builder {
        self.client = Some(client);
        self
    }

    /// Credentials for every request.
    pub fn auth(mut self, auth: Auth) -> Builder {
        self.settings.auth = Some(auth);
        self
    }

    /// How many times a request goes again, to the next node or after a
    /// backoff. None by default.
    pub fn retries(mut self, retries: u32) -> Builder {
        self.settings.retries = retries;
        self
    }

    /// The first and longest waits before a request goes again: 100 ms and
    /// 10 s by default, doubling in between.
    pub fn backoff(mut self, initial: Duration, max: Duration) -> Builder {
        self.settings.backoff = (initial, max.max(initial));
        self
    }

    /// Whether a request that timed out goes again. Off by default: the
    /// node may have acted on it.
    pub fn retry_on_timeout(mut self, retry: bool) -> Builder {
        self.settings.retry_on_timeout = retry;
        self
    }

    /// How long a node rests after its first failure, and at most: 1 minute
    /// and 30 minutes by default, doubling in between.
    pub fn resurrect_after(mut self, initial: Duration, max: Duration) -> Builder {
        self.rest = (initial, max.max(initial));
        self
    }

    /// Refreshes the nodes from the cluster this often, and before the
    /// first request.
    pub fn sniff_every(mut self, every: Duration) -> Builder {
        self.settings.sniff_every = Some(every);
        self
    }

    /// Refreshes the nodes from the cluster after a node fails.
    pub fn sniff_on_failure(mut self, sniff: bool) -> Builder {
        self.settings.sniff_on_failure = sniff;
        self
    }

    /// Compresses request bodies with gzip, and accepts compressed
    /// responses.
    #[cfg(feature = "gzip")]
    pub fn gzip(mut self, gzip: bool) -> Builder {
        self.settings.gzip = gzip;
        self
    }

    /// The transport.
    pub fn build(mut self) -> Result<ReqwestTransport, TransportError> {
        if self.nodes.is_empty() {
            return Err("a transport needs a node".into());
        }
        let mut scheme = None;
        for url in &self.nodes {
            let parsed = reqwest::Url::parse(url)?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err(format!("{url} isn't an http or https URL").into());
            }
            scheme.get_or_insert_with(|| parsed.scheme().to_owned());
        }
        self.settings.scheme = scheme.unwrap_or_else(|| "http".to_owned());
        let client = match self.client {
            Some(client) => client,
            None => reqwest::Client::builder().build()?,
        };
        // Sniffing every so often starts with a sniff; sniffing only on
        // failure waits for one.
        let sniffed = self.settings.sniff_every.is_none().then(Instant::now);
        Ok(ReqwestTransport {
            shared: Arc::new(Shared {
                client,
                state: Mutex::new(State {
                    pool: Pool::new(self.nodes, self.rest),
                    sniffed,
                    sniffing: false,
                }),
                settings: self.settings,
            }),
        })
    }
}

/// The method, as `reqwest` names it.
fn method(method: Method) -> Option<reqwest::Method> {
    Some(match method {
        Method::Get => reqwest::Method::GET,
        Method::Post => reqwest::Method::POST,
        Method::Put => reqwest::Method::PUT,
        Method::Delete => reqwest::Method::DELETE,
        Method::Head => reqwest::Method::HEAD,
        _ => return None,
    })
}

/// A body compressed with gzip.
#[cfg(feature = "gzip")]
fn gzip(bytes: &[u8]) -> Result<bytes::Bytes, TransportError> {
    use std::io::Write;

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(bytes)?;
    Ok(encoder.finish()?.into())
}

#[cfg(not(feature = "gzip"))]
fn gzip(bytes: &[u8]) -> Result<bytes::Bytes, TransportError> {
    Ok(bytes::Bytes::copy_from_slice(bytes))
}
