//! What a call to Elasticsearch is: the [`Endpoint`] every generated
//! request implements, and the [`Transport`] that carries it.

use std::future::Future;

use serde::de::DeserializeOwned;

use crate::error::{Error, TransportError};

/// An HTTP method.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Method {
    /// `GET`.
    Get,
    /// `POST`.
    Post,
    /// `PUT`.
    Put,
    /// `DELETE`.
    Delete,
    /// `HEAD`.
    Head,
}

impl Method {
    /// The method's name, such as `GET`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
            Method::Head => "HEAD",
        }
    }
}

/// A request's body.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Body {
    /// JSON.
    Json(Vec<u8>),
    /// Newline-delimited JSON, as bulk and multi-search take.
    Ndjson(Vec<u8>),
}

impl Body {
    /// The body's media type.
    pub fn content_type(&self) -> &'static str {
        match self {
            Body::Json(_) => "application/json",
            Body::Ndjson(_) => "application/x-ndjson",
        }
    }

    /// The body's bytes.
    pub fn bytes(&self) -> &[u8] {
        match self {
            Body::Json(bytes) | Body::Ndjson(bytes) => bytes,
        }
    }

    /// The body's bytes, owned.
    pub fn into_bytes(self) -> Vec<u8> {
        match self {
            Body::Json(bytes) | Body::Ndjson(bytes) => bytes,
        }
    }
}

/// Where a call goes: its method and its path, percent-encoded.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Route {
    /// The method.
    pub method: Method,
    /// The path, percent-encoded, without a query.
    pub path: String,
}

impl Route {
    /// A route.
    pub fn new(method: Method, path: impl Into<String>) -> Route {
        Route {
            method,
            path: path.into(),
        }
    }
}

/// A call Elasticsearch understands, and the response it returns. Every
/// generated request implements it, and anything else can: the client
/// sends a user's own endpoint the same way.
pub trait Endpoint {
    /// The response, with documents and other user data as JSON. Use
    /// [`Client::send_as`](crate::Client::send_as) to read them into your
    /// own types.
    type Response: DeserializeOwned;

    /// The endpoint's name in the specification, such as `search` or
    /// `indices.create`.
    fn name(&self) -> &'static str;

    /// The method and path, from the parameters set.
    fn route(&self) -> Result<Route, Error>;

    /// The query parameters set, not yet percent-encoded.
    fn query(&self) -> Result<Vec<(&'static str, String)>, Error>;

    /// The body, if the call has one.
    fn body(&self) -> Result<Option<Body>, Error>;
}

/// The HTTP request a transport sends.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct HttpRequest {
    /// The method.
    pub method: Method,
    /// The path and query, percent-encoded, starting with `/`.
    pub path_and_query: String,
    /// The body.
    pub body: Option<Body>,
}

/// The HTTP response a transport returns.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct HttpResponse {
    /// The status code.
    pub status: u16,
    /// The body's bytes.
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// A response.
    pub fn new(status: u16, body: Vec<u8>) -> HttpResponse {
        HttpResponse { status, body }
    }
}

/// Carries requests to a cluster and brings back its responses: an HTTP
/// client, and later the layers around one (authentication, retries,
/// telemetry).
pub trait Transport: Send + Sync {
    /// Sends a request.
    fn send(
        &self,
        request: HttpRequest,
    ) -> impl Future<Output = Result<HttpResponse, TransportError>> + Send;
}
