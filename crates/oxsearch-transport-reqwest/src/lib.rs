//! The default HTTP transport for oxsearch, built on `reqwest`.
//!
//! ```no_run
//! # async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! use oxsearch_core::Client;
//! use oxsearch_transport_reqwest::ReqwestTransport;
//!
//! let client = Client::new(ReqwestTransport::new("http://localhost:9200")?);
//! let info = client.send(&oxsearch_core::info::Request::new()).await?;
//! # Ok(())
//! # }
//! ```
//!
//! It sends each request once, to one node. Retries, failover across nodes
//! and authentication are still to come.

use oxsearch_core::{HttpRequest, HttpResponse, Method, Transport, TransportError};

/// A transport that sends requests to one node over HTTP with `reqwest`.
#[derive(Clone, Debug)]
pub struct ReqwestTransport {
    client: reqwest::Client,
    base: String,
}

impl ReqwestTransport {
    /// A transport to the node at `url`, such as `http://localhost:9200`.
    pub fn new(url: &str) -> Result<ReqwestTransport, TransportError> {
        ReqwestTransport::with_client(reqwest::Client::new(), url)
    }

    /// A transport using a `reqwest` client you configured, for timeouts,
    /// proxies or certificates.
    pub fn with_client(
        client: reqwest::Client,
        url: &str,
    ) -> Result<ReqwestTransport, TransportError> {
        let parsed = reqwest::Url::parse(url)?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(format!("{url} isn't an http or https URL").into());
        }
        Ok(ReqwestTransport {
            client,
            base: url.trim_end_matches('/').to_owned(),
        })
    }
}

impl Transport for ReqwestTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let method = match request.method {
            Method::Get => reqwest::Method::GET,
            Method::Post => reqwest::Method::POST,
            Method::Put => reqwest::Method::PUT,
            Method::Delete => reqwest::Method::DELETE,
            Method::Head => reqwest::Method::HEAD,
            _ => return Err(format!("{} isn't supported", request.method.as_str()).into()),
        };
        let mut builder = self
            .client
            .request(method, format!("{}{}", self.base, request.path_and_query));
        if let Some(body) = request.body {
            builder = builder
                .header(reqwest::header::CONTENT_TYPE, body.content_type())
                .body(body.into_bytes());
        }
        let response = builder.send().await?;
        let status = response.status().as_u16();
        let body = response.bytes().await?.to_vec();
        Ok(HttpResponse::new(status, body))
    }
}
