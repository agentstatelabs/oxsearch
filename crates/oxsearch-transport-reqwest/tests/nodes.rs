//! The transport against small HTTP servers: failover, retries, backoff,
//! sniffing, credentials and compression.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use oxsearch_core::auth::Auth;
use oxsearch_core::{Body, HttpRequest, Method, Transport};
use oxsearch_transport_reqwest::ReqwestTransport;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

/// A request a server saw.
#[derive(Clone, Debug)]
struct Seen {
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

type Handler = dyn Fn(&Seen, usize) -> (u16, String) + Send + Sync;

/// An HTTP server that answers each request as `handler` says, given the
/// request and how many came before it.
struct Server {
    url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Server {
    async fn start(
        handler: impl Fn(&Seen, usize) -> (u16, String) + Send + Sync + 'static,
    ) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
        let handler: Arc<Handler> = Arc::new(handler);
        let record = seen.clone();
        tokio::spawn(async move {
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                tokio::spawn(connection(socket, handler.clone(), record.clone()));
            }
        });
        Server { url, seen }
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    /// The address it publishes, as a node does.
    fn address(&self) -> String {
        self.url.trim_start_matches("http://").to_owned()
    }
}

async fn connection(
    socket: tokio::net::TcpStream,
    handler: Arc<Handler>,
    seen: Arc<Mutex<Vec<Seen>>>,
) {
    let (read, mut write) = socket.into_split();
    let mut reader = BufReader::new(read);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
            return;
        }
        let path = line
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_owned();
        let mut headers = Vec::new();
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).await.unwrap();
            let header = header.trim_end();
            if header.is_empty() {
                break;
            }
            let (name, value) = header.split_once(':').unwrap();
            headers.push((name.trim().to_lowercase(), value.trim().to_owned()));
        }
        let length: usize = headers
            .iter()
            .find(|(n, _)| n == "content-length")
            .map_or(0, |(_, v)| v.parse().unwrap());
        let mut body = vec![0; length];
        reader.read_exact(&mut body).await.unwrap();
        let request = Seen {
            path,
            headers,
            body,
        };
        let count = {
            let mut seen = seen.lock().unwrap();
            seen.push(request.clone());
            seen.len() - 1
        };
        let (status, answer) = handler(&request, count);
        let response = format!(
            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{answer}",
            answer.len()
        );
        if write.write_all(response.as_bytes()).await.is_err() {
            return;
        }
    }
}

/// A URL nothing listens on.
fn closed() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    format!("http://{}", listener.local_addr().unwrap())
}

fn get(path: &str) -> HttpRequest {
    HttpRequest::new(Method::Get, path, None)
}

#[tokio::test]
async fn sends_credentials_and_headers() {
    let server = Server::start(|_, _| (200, "{}".into())).await;
    let transport = ReqwestTransport::builder()
        .node(&server.url)
        .auth(Auth::basic("elastic", "changeme"))
        .build()
        .unwrap();
    let request = HttpRequest::new(
        Method::Post,
        "/idx/_search",
        Some(Body::json(&serde_json::json!({"size": 1})).unwrap()),
    )
    .header("x-opaque-id", "test-1");
    let response = transport.send(request).await.unwrap();
    assert_eq!(response.status, 200);
    let seen = &server.seen()[0];
    assert_eq!(seen.path, "/idx/_search");
    assert_eq!(
        seen.header("authorization"),
        Some("Basic ZWxhc3RpYzpjaGFuZ2VtZQ==")
    );
    assert_eq!(seen.header("x-opaque-id"), Some("test-1"));
    assert_eq!(seen.header("content-type"), Some("application/json"));
    assert_eq!(seen.body, br#"{"size":1}"#);
}

#[tokio::test]
async fn sends_once_unless_asked() {
    let transport = ReqwestTransport::new(&closed()).unwrap();
    assert!(transport.send(get("/")).await.is_err());
}

#[tokio::test]
async fn fails_over_and_rests_the_failed_node() {
    let live = Server::start(|_, _| (200, "{}".into())).await;
    let dead = closed();
    let transport = ReqwestTransport::builder()
        .nodes([dead.as_str(), live.url.as_str()])
        .retries(2)
        .build()
        .unwrap();
    for _ in 0..3 {
        assert_eq!(transport.send(get("/")).await.unwrap().status, 200);
    }
    // The dead node was tried once, then left to rest.
    assert_eq!(live.seen().len(), 3);
    assert_eq!(transport.nodes(), [dead, live.url.clone()]);
}

#[tokio::test]
async fn unavailable_nodes_hand_over_to_the_next() {
    let busy = Server::start(|_, _| (503, "{}".into())).await;
    let fine = Server::start(|_, _| (200, "{}".into())).await;
    let transport = ReqwestTransport::builder()
        .nodes([busy.url.as_str(), fine.url.as_str()])
        .retries(1)
        .build()
        .unwrap();
    assert_eq!(transport.send(get("/")).await.unwrap().status, 200);
    assert_eq!((busy.seen().len(), fine.seen().len()), (1, 1));
}

#[tokio::test]
async fn backs_off_on_429_then_gives_the_last_answer() {
    let server = Server::start(|_, n| {
        if n < 2 {
            (429, "{}".into())
        } else {
            (200, "{}".into())
        }
    })
    .await;
    let transport = ReqwestTransport::builder()
        .node(&server.url)
        .retries(3)
        .backoff(Duration::from_millis(1), Duration::from_millis(5))
        .build()
        .unwrap();
    assert_eq!(transport.send(get("/")).await.unwrap().status, 200);
    assert_eq!(server.seen().len(), 3);

    let always = Server::start(|_, _| (503, "{}".into())).await;
    let transport = ReqwestTransport::builder()
        .node(&always.url)
        .retries(2)
        .backoff(Duration::from_millis(1), Duration::from_millis(5))
        .build()
        .unwrap();
    assert_eq!(transport.send(get("/")).await.unwrap().status, 503);
    assert_eq!(always.seen().len(), 3);
}

/// Answers sniffing with the published addresses, and anything else with
/// 200.
fn lister(addresses: Vec<String>) -> impl Fn(&Seen, usize) -> (u16, String) + Send + Sync {
    move |seen, _| {
        if seen.path.starts_with("/_nodes/http") {
            let nodes: serde_json::Map<String, serde_json::Value> = addresses
                .iter()
                .enumerate()
                .map(|(n, a)| {
                    (
                        format!("node-{n}"),
                        serde_json::json!({"http": {"publish_address": a}}),
                    )
                })
                .collect();
            (200, serde_json::json!({ "nodes": nodes }).to_string())
        } else {
            (200, "{}".into())
        }
    }
}

#[tokio::test]
async fn sniffs_before_the_first_request() {
    let other = Server::start(|_, _| (200, "{}".into())).await;
    let seed = Server::start(lister(vec![format!("localhost/{}", other.address())])).await;
    let transport = ReqwestTransport::builder()
        .node(&seed.url)
        .auth(Auth::bearer("token"))
        .sniff_every(Duration::from_secs(3600))
        .build()
        .unwrap();
    assert_eq!(transport.send(get("/")).await.unwrap().status, 200);
    // The hostname wins over the address it publishes.
    let port = other.address().rsplit_once(':').unwrap().1.to_owned();
    assert_eq!(transport.nodes(), [format!("http://localhost:{port}")]);
    let sniff = &seed.seen()[0];
    assert!(sniff.path.starts_with("/_nodes/http"));
    assert_eq!(sniff.header("authorization"), Some("Bearer token"));
}

#[tokio::test]
async fn sniffs_after_a_failure() {
    let other = Server::start(|_, _| (200, "{}".into())).await;
    let seed = Server::start(lister(vec![other.address()])).await;
    let transport = ReqwestTransport::builder()
        .nodes([closed(), seed.url.clone()])
        .retries(1)
        .sniff_on_failure(true)
        .build()
        .unwrap();
    // Fails over to the seed; the failure asks for a sniff.
    assert_eq!(transport.send(get("/")).await.unwrap().status, 200);
    // The next request sniffs first, from the seed.
    assert_eq!(transport.send(get("/")).await.unwrap().status, 200);
    assert_eq!(transport.nodes(), [other.url.as_str()]);
    assert_eq!(other.seen().len(), 1);
}

#[tokio::test]
async fn compresses_bodies() {
    let server = Server::start(|_, _| (200, "{}".into())).await;
    let transport = ReqwestTransport::builder()
        .node(&server.url)
        .gzip(true)
        .build()
        .unwrap();
    let body = Body::json(&serde_json::json!({"text": "x".repeat(1000)})).unwrap();
    transport
        .send(HttpRequest::new(Method::Post, "/idx/_doc", Some(body)))
        .await
        .unwrap();
    let seen = &server.seen()[0];
    assert_eq!(seen.header("content-encoding"), Some("gzip"));
    assert_eq!(&seen.body[..2], [0x1f, 0x8b]);
    assert!(seen.body.len() < 200);
}
