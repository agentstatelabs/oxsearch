//! What a call to Elasticsearch is: the [`Endpoint`] every generated
//! request implements, and the [`Transport`] that carries it.

use std::future::Future;

use serde::Serialize;
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
    /// A JSON body.
    pub fn json<T: Serialize + ?Sized>(value: &T) -> Result<Body, Error> {
        serde_json::to_vec(value)
            .map(Body::Json)
            .map_err(Error::Encode)
    }

    /// A newline-delimited JSON body: one line per item.
    pub fn ndjson<T: Serialize>(lines: impl IntoIterator<Item = T>) -> Result<Body, Error> {
        let mut out = Vec::new();
        for line in lines {
            serde_json::to_writer(&mut out, &line).map_err(Error::Encode)?;
            out.push(b'\n');
        }
        Ok(Body::Ndjson(out))
    }

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
    /// A route, with a path already percent-encoded.
    pub fn new(method: Method, path: impl Into<String>) -> Route {
        Route {
            method,
            path: path.into(),
        }
    }

    /// A route from its path's segments, each percent-encoded:
    /// `Route::segments(Method::Get, ["_cat", "count", index])`.
    pub fn segments<S: AsRef<str>>(method: Method, segments: impl IntoIterator<Item = S>) -> Route {
        let mut path = String::new();
        for segment in segments {
            path.push('/');
            path.push_str(&crate::__private::encode(segment.as_ref()));
        }
        if path.is_empty() {
            path.push('/');
        }
        Route { method, path }
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
    /// Headers besides the body's content type, which the transport sets:
    /// what layers add, such as `authorization`.
    pub headers: Vec<(String, String)>,
    /// The body.
    pub body: Option<Body>,
}

impl HttpRequest {
    /// A request without headers.
    pub fn new(
        method: Method,
        path_and_query: impl Into<String>,
        body: Option<Body>,
    ) -> HttpRequest {
        HttpRequest {
            method,
            path_and_query: path_and_query.into(),
            headers: Vec::new(),
            body,
        }
    }

    /// The request with a header set, replacing any of the same name.
    pub fn header(mut self, name: &str, value: impl Into<String>) -> HttpRequest {
        self.headers.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        self.headers.push((name.to_ascii_lowercase(), value.into()));
        self
    }
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

/// A transport by reference is a transport, so a layer can wrap a shared one.
impl<T: Transport> Transport for &T {
    fn send(
        &self,
        request: HttpRequest,
    ) -> impl Future<Output = Result<HttpResponse, TransportError>> + Send {
        (**self).send(request)
    }
}

/// Carries requests to a cluster and brings back its responses: an HTTP
/// client, or a layer around one, such as [`Authenticated`](crate::auth::Authenticated).
///
/// A layer of your own is a transport that wraps another:
///
/// ```
/// use oxsearch_core::{HttpRequest, HttpResponse, Transport, TransportError};
///
/// /// Tags every request, so the cluster's slow logs and tasks show where it
/// /// came from.
/// struct Tagged<T> {
///     inner: T,
///     tag: String,
/// }
///
/// impl<T: Transport> Transport for Tagged<T> {
///     async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
///         self.inner.send(request.header("x-opaque-id", self.tag.as_str())).await
///     }
/// }
/// ```
pub trait Transport: Send + Sync {
    /// Sends a request.
    fn send(
        &self,
        request: HttpRequest,
    ) -> impl Future<Output = Result<HttpResponse, TransportError>> + Send;
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn routes_from_segments() {
        let route = Route::segments(Method::Get, ["_cat", "count", "logs 2026/10"]);
        assert_eq!(route.path, "/_cat/count/logs%202026%2F10");
        assert_eq!(Route::segments(Method::Get, [""; 0]).path, "/");
    }

    #[test]
    fn bodies() {
        let body = Body::json(&json!({"a": 1})).unwrap();
        assert_eq!(body.content_type(), "application/json");
        assert_eq!(body.bytes(), br#"{"a":1}"#);
        let lines = Body::ndjson([json!({"index": {}}), json!({"a": 1})]).unwrap();
        assert_eq!(lines.into_bytes(), b"{\"index\":{}}\n{\"a\":1}\n");
    }
}
