//! Authentication: a layer that signs each request, with fixed credentials
//! or your own that refresh.

use std::fmt;
use std::future::Future;

use crate::endpoint::{HttpRequest, HttpResponse, Transport};
use crate::error::TransportError;

/// Where an `authorization` header comes from. [`Auth`] gives fixed ones;
/// implement it for credentials that expire, such as tokens from an
/// identity provider.
pub trait Credentials: Send + Sync {
    /// The `authorization` header's value for the next request.
    fn authorization(&self) -> impl Future<Output = Result<String, TransportError>> + Send;

    /// Whether [`Credentials::refresh`] can give a new value, so a request
    /// Elasticsearch refuses with 401 is worth sending again.
    fn refreshes(&self) -> bool {
        false
    }

    /// Elasticsearch refused the last value with 401: get a new one. The
    /// refused request is sent once more with it.
    fn refresh(&self) -> impl Future<Output = Result<(), TransportError>> + Send {
        async { Ok(()) }
    }
}

/// Fixed credentials.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Auth {
    /// A user name and password.
    Basic {
        /// The user name.
        username: String,
        /// The password.
        password: String,
    },
    /// A bearer token, such as a service account's.
    Bearer(String),
    /// An API key, as Elasticsearch encodes it.
    ApiKey(String),
}

impl Auth {
    /// A user name and password.
    pub fn basic(username: impl Into<String>, password: impl Into<String>) -> Auth {
        Auth::Basic {
            username: username.into(),
            password: password.into(),
        }
    }

    /// A bearer token.
    pub fn bearer(token: impl Into<String>) -> Auth {
        Auth::Bearer(token.into())
    }

    /// An API key, encoded: the `encoded` value Elasticsearch returns when
    /// it creates one.
    pub fn api_key(encoded: impl Into<String>) -> Auth {
        Auth::ApiKey(encoded.into())
    }

    /// An API key, from its id and secret.
    pub fn api_key_id(id: &str, key: &str) -> Auth {
        Auth::ApiKey(base64(format!("{id}:{key}").as_bytes()))
    }

    /// The `authorization` header's value.
    pub fn header(&self) -> String {
        match self {
            Auth::Basic { username, password } => {
                format!(
                    "Basic {}",
                    base64(format!("{username}:{password}").as_bytes())
                )
            }
            Auth::Bearer(token) => format!("Bearer {token}"),
            Auth::ApiKey(key) => format!("ApiKey {key}"),
        }
    }
}

/// Credentials don't show in logs.
impl fmt::Debug for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Auth::Basic { username, .. } => write!(f, "Basic({username}, …)"),
            Auth::Bearer(_) => f.write_str("Bearer(…)"),
            Auth::ApiKey(_) => f.write_str("ApiKey(…)"),
        }
    }
}

impl Credentials for Auth {
    fn authorization(&self) -> impl Future<Output = Result<String, TransportError>> + Send {
        let header = self.header();
        async move { Ok(header) }
    }
}

/// A transport that signs each request with credentials: a layer around
/// another transport.
///
/// ```no_run
/// # fn wrap<T: oxsearch_core::Transport>(transport: T) {
/// use oxsearch_core::Client;
/// use oxsearch_core::auth::{Auth, Authenticated};
///
/// let client = Client::new(Authenticated::new(transport, Auth::api_key("VnVhQ2ZHY0JDZGJrU…")));
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct Authenticated<T, C = Auth> {
    inner: T,
    credentials: C,
}

impl<T, C> Authenticated<T, C> {
    /// Signs `inner`'s requests with `credentials`.
    pub fn new(inner: T, credentials: C) -> Authenticated<T, C> {
        Authenticated { inner, credentials }
    }

    /// The transport inside.
    pub fn inner(&self) -> &T {
        &self.inner
    }
}

impl<T: Transport, C: Credentials> Transport for Authenticated<T, C> {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let again = self.credentials.refreshes().then(|| request.clone());
        let header = self.credentials.authorization().await?;
        let response = self
            .inner
            .send(request.header("authorization", header))
            .await?;
        match again {
            Some(request) if response.status == 401 => {
                self.credentials.refresh().await?;
                let header = self.credentials.authorization().await?;
                self.inner
                    .send(request.header("authorization", header))
                    .await
            }
            _ => Ok(response),
        }
    }
}

/// The URL of the Elastic Cloud deployment a Cloud ID names: `name:` then,
/// in base64, the host, `$`, the Elasticsearch id and `$`, the Kibana id.
pub fn cloud_url(cloud_id: &str) -> Result<String, TransportError> {
    let invalid = || TransportError::from(format!("{cloud_id:?} isn't a Cloud ID"));
    let encoded = cloud_id.rsplit_once(':').map_or(cloud_id, |(_, e)| e);
    let decoded =
        String::from_utf8(unbase64(encoded).ok_or_else(invalid)?).map_err(|_| invalid())?;
    let mut parts = decoded.split('$');
    let (Some(host), Some(elasticsearch)) = (parts.next(), parts.next()) else {
        return Err(invalid());
    };
    if host.is_empty() || elasticsearch.is_empty() {
        return Err(invalid());
    }
    let (host, mut port) = match host.rsplit_once(':') {
        Some((host, port)) => (host, port),
        None => (host, "443"),
    };
    let elasticsearch = match elasticsearch.split_once(':') {
        Some((id, own)) => {
            port = own;
            id
        }
        None => elasticsearch,
    };
    Ok(format!("https://{elasticsearch}.{host}:{port}"))
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn unbase64(text: &str) -> Option<Vec<u8>> {
    let text = text.trim_end_matches('=');
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut n = 0u32;
    for (i, c) in text.bytes().enumerate() {
        let value = ALPHABET.iter().position(|a| *a == c)? as u32;
        n = n << 6 | value;
        if i % 4 == 3 {
            out.extend_from_slice(&[(n >> 16) as u8, (n >> 8) as u8, n as u8]);
            n = 0;
        }
    }
    match text.len() % 4 {
        0 => {}
        2 => out.push((n >> 4) as u8),
        3 => out.extend_from_slice(&[(n >> 10) as u8, (n >> 2) as u8]),
        _ => return None,
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;
    use crate::endpoint::Method;

    #[test]
    fn base64_both_ways() {
        for text in ["", "f", "fo", "foo", "foob", "fooba", "foobar"] {
            assert_eq!(unbase64(&base64(text.as_bytes())).unwrap(), text.as_bytes());
        }
        assert_eq!(base64(b"elastic:changeme"), "ZWxhc3RpYzpjaGFuZ2VtZQ==");
    }

    #[test]
    fn headers() {
        assert_eq!(
            Auth::basic("elastic", "changeme").header(),
            "Basic ZWxhc3RpYzpjaGFuZ2VtZQ=="
        );
        assert_eq!(Auth::api_key_id("id", "key").header(), "ApiKey aWQ6a2V5");
        assert_eq!(Auth::bearer("t").header(), "Bearer t");
        assert!(!format!("{:?}", Auth::basic("elastic", "changeme")).contains("changeme"));
    }

    #[test]
    fn cloud_ids() {
        let id = format!(
            "my-deployment:{}",
            base64(b"us-east-1.aws.found.io$abc123$def456")
        );
        assert_eq!(
            cloud_url(&id).unwrap(),
            "https://abc123.us-east-1.aws.found.io:443"
        );
        let id = format!("x:{}", base64(b"eu-west-1.aws.found.io:9243$abc$def"));
        assert_eq!(
            cloud_url(&id).unwrap(),
            "https://abc.eu-west-1.aws.found.io:9243"
        );
        assert!(cloud_url("nonsense").is_err());
    }

    /// Answers 401 to anything but the current token, and keeps the
    /// headers it saw.
    struct Gate {
        token: Mutex<String>,
        seen: Mutex<Vec<String>>,
    }

    impl Transport for Gate {
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
            let header = request
                .headers
                .iter()
                .find(|(n, _)| n == "authorization")
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            self.seen.lock().unwrap().push(header.clone());
            let ok = header == format!("Bearer {}", self.token.lock().unwrap());
            Ok(HttpResponse::new(if ok { 200 } else { 401 }, Vec::new()))
        }
    }

    /// Tokens numbered from 1; refreshing moves to the next.
    struct Rotating(AtomicU32);

    impl Credentials for Rotating {
        async fn authorization(&self) -> Result<String, TransportError> {
            Ok(format!("Bearer t{}", self.0.load(Ordering::SeqCst)))
        }

        fn refreshes(&self) -> bool {
            true
        }

        async fn refresh(&self) -> Result<(), TransportError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        let mut future = std::pin::pin!(future);
        loop {
            if let std::task::Poll::Ready(out) = future.as_mut().poll(&mut cx) {
                return out;
            }
        }
    }

    #[test]
    fn refreshes_on_401() {
        let gate = Gate {
            token: Mutex::new("t2".to_owned()),
            seen: Mutex::default(),
        };
        let layer = Authenticated::new(&gate, Rotating(AtomicU32::new(1)));
        let request = HttpRequest::new(Method::Get, "/", None);
        let response = block_on(layer.send(request)).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(*gate.seen.lock().unwrap(), ["Bearer t1", "Bearer t2"]);
    }

    #[test]
    fn fixed_credentials_are_sent_once() {
        let gate = Gate {
            token: Mutex::new("right".to_owned()),
            seen: Mutex::default(),
        };
        let layer = Authenticated::new(&gate, Auth::bearer("wrong"));
        let response = block_on(layer.send(HttpRequest::new(Method::Get, "/", None))).unwrap();
        assert_eq!(response.status, 401);
        assert_eq!(gate.seen.lock().unwrap().len(), 1);
    }
}
