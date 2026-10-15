//! The client: sends endpoints over a transport and reads their responses.

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::endpoint::{Body, Endpoint, HttpRequest, Method, Transport};
use crate::error::Error;

/// Sends calls to a cluster over a [`Transport`].
#[derive(Clone, Debug)]
pub struct Client<T> {
    transport: T,
}

impl<T: Transport> Client<T> {
    /// A client over a transport.
    pub fn new(transport: T) -> Client<T> {
        Client { transport }
    }

    /// The transport.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Sends a call and reads its response, with documents and other user
    /// data as JSON.
    pub async fn send<E: Endpoint>(&self, endpoint: &E) -> Result<E::Response, Error> {
        self.send_as(endpoint).await
    }

    /// Sends a call and reads its response into another type, such as a
    /// search response with your own document type:
    /// `client.send_as::<_, search::Response<Article>>(&request)`.
    pub async fn send_as<E: Endpoint, R: DeserializeOwned>(
        &self,
        endpoint: &E,
    ) -> Result<R, Error> {
        let route = endpoint.route()?;
        let mut path_and_query = route.path;
        let query = endpoint.query()?;
        for (n, (name, value)) in query.iter().enumerate() {
            path_and_query.push(if n == 0 { '?' } else { '&' });
            path_and_query.push_str(name);
            path_and_query.push('=');
            path_and_query.push_str(&crate::__private::encode(value));
        }
        let request = HttpRequest {
            method: route.method,
            path_and_query,
            body: endpoint.body()?,
        };
        let response = self
            .transport
            .send(request)
            .await
            .map_err(Error::Transport)?;
        if !(200..300).contains(&response.status) {
            return Err(Error::from_response(response.status, &response.body));
        }
        // Calls that answer with no body, such as HEAD requests, read as an
        // empty object.
        let body: &[u8] = if response.body.is_empty() {
            b"{}"
        } else {
            &response.body
        };
        serde_json::from_slice(body).map_err(|source| Error::Decode {
            status: response.status,
            source,
            body: String::from_utf8_lossy(&body[..body.len().min(512)]).into_owned(),
        })
    }

    /// Sends any request, for APIs oxsearch doesn't cover yet: a path,
    /// percent-encoded and with any query, and an optional JSON body.
    pub async fn send_raw(
        &self,
        method: Method,
        path_and_query: &str,
        body: Option<&Value>,
    ) -> Result<Value, Error> {
        let body = match body {
            Some(body) => Some(Body::Json(serde_json::to_vec(body).map_err(Error::Encode)?)),
            None => None,
        };
        let request = HttpRequest {
            method,
            path_and_query: path_and_query.to_owned(),
            body,
        };
        let response = self
            .transport
            .send(request)
            .await
            .map_err(Error::Transport)?;
        if !(200..300).contains(&response.status) {
            return Err(Error::from_response(response.status, &response.body));
        }
        if response.body.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&response.body).map_err(|source| Error::Decode {
            status: response.status,
            source,
            body: String::from_utf8_lossy(&response.body[..response.body.len().min(512)])
                .into_owned(),
        })
    }
}
