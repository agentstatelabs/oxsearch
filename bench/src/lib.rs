//! What the benchmarks share: the documents, the canned Elasticsearch
//! responses, and a server that answers every request with one of them.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A document of about 470 bytes as JSON, with the usual mix of fields.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Doc {
    pub id: u64,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub price: f64,
    pub views: u32,
    pub published: String,
    pub author: Author,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Author {
    pub name: String,
    pub email: String,
}

/// The `n`th document.
pub fn doc(n: u64) -> Doc {
    Doc {
        id: n,
        title: format!("Tuning HNSW graphs, part {n}"),
        body: "Graph-based vector indexes trade memory for recall: more links per \
               node find neighbours faster and more surely, and cost more to build. \
               These notes measure that trade on a few million vectors, with \
               quantization on and off, and say which settings held up."
            .to_owned(),
        tags: vec!["vector".into(), "hnsw".into(), format!("series-{}", n % 7)],
        price: 19.5 + (n % 100) as f64,
        views: (n * 37 % 10_000) as u32,
        published: format!("2026-{:02}-{:02}T10:00:00Z", n % 12 + 1, n % 28 + 1),
        author: Author {
            name: format!("Author {}", n % 50),
            email: format!("author{}@example.com", n % 50),
        },
    }
}

/// A search response as Elasticsearch writes it, with `hits` documents.
pub fn search_response(hits: u64) -> Vec<u8> {
    let hits: Vec<Value> = (0..hits)
        .map(|n| {
            json!({
                "_index": "bench",
                "_id": n.to_string(),
                "_score": 1.0 + 1.0 / (n + 1) as f64,
                "_source": doc(n),
            })
        })
        .collect();
    serde_json::to_vec(&json!({
        "took": 3,
        "timed_out": false,
        "_shards": {"total": 1, "successful": 1, "skipped": 0, "failed": 0},
        "hits": {
            "total": {"value": hits.len(), "relation": "eq"},
            "max_score": 2.0,
            "hits": hits,
        },
    }))
    .expect("JSON")
}

/// An index response, for a write.
pub fn index_response() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "_index": "bench", "_id": "1", "_version": 2, "result": "updated",
        "_shards": {"total": 1, "successful": 1, "failed": 0},
        "_seq_no": 7, "_primary_term": 1,
    }))
    .expect("JSON")
}

/// A search response as you'd write it yourself for the official client,
/// which hands you the body to read: only what you use.
#[derive(Debug, Deserialize)]
pub struct MinimalSearch<T> {
    pub hits: MinimalHits<T>,
}

#[derive(Debug, Deserialize)]
pub struct MinimalHits<T> {
    pub hits: Vec<MinimalHit<T>>,
}

#[derive(Debug, Deserialize)]
pub struct MinimalHit<T> {
    #[serde(rename = "_id")]
    pub id: String,
    #[serde(rename = "_source")]
    pub source: T,
}

/// An index response as you'd write it yourself.
#[derive(Debug, Deserialize)]
pub struct MinimalWrite {
    #[serde(rename = "_id")]
    pub id: String,
    pub result: String,
}

/// Starts a server on a free local port that answers every request with
/// `body`, over HTTP/1.1 with keep-alive, and returns its URL. It reads each
/// request whole, by its `content-length`, before answering.
pub async fn serve(body: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
    let url = format!("http://{}", listener.local_addr().expect("an address"));
    let body = Arc::new(body);
    tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                return;
            };
            socket.set_nodelay(true).ok();
            tokio::spawn(answer(socket, body.clone()));
        }
    });
    url
}

async fn answer(mut socket: TcpStream, body: Arc<Vec<u8>>) {
    // The official client checks that it's talking to Elasticsearch.
    let head = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nx-elastic-product: Elasticsearch\r\ncontent-length: {}\r\n\r\n",
        body.len()
    );
    let mut response = head.into_bytes();
    response.extend_from_slice(&body);
    let mut buffer = Vec::with_capacity(16 * 1024);
    let mut chunk = [0u8; 16 * 1024];
    loop {
        // The head.
        let end = loop {
            if let Some(end) = find(&buffer, b"\r\n\r\n") {
                break end + 4;
            }
            match socket.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(n) => buffer.extend_from_slice(&chunk[..n]),
            }
        };
        let length = content_length(&buffer[..end]);
        // The body.
        while buffer.len() < end + length {
            match socket.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(n) => buffer.extend_from_slice(&chunk[..n]),
            }
        }
        buffer.drain(..end + length);
        if socket.write_all(&response).await.is_err() {
            return;
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn content_length(head: &[u8]) -> usize {
    String::from_utf8_lossy(head)
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())
                .flatten()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_are_about_470_bytes() {
        let size = serde_json::to_vec(&doc(42)).unwrap().len();
        assert!((440..500).contains(&size), "{size}");
    }
}
