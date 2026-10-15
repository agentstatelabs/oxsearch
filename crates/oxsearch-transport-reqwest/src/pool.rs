//! The nodes requests go to: which are alive, which are resting after a
//! failure, and which to try next.

use std::time::{Duration, Instant};

/// A node and how it has been doing.
#[derive(Clone, Debug)]
pub(crate) struct Node {
    /// The node's URL, without a trailing `/`.
    pub(crate) url: String,
    /// Until when the node rests after failing, if it is.
    dead_until: Option<Instant>,
    /// Failures in a row.
    failures: u32,
}

/// The nodes, in the order they're tried.
#[derive(Debug)]
pub(crate) struct Pool {
    nodes: Vec<Node>,
    next: usize,
    /// How long a node rests after its first failure, and at most.
    rest: (Duration, Duration),
}

impl Pool {
    pub(crate) fn new(urls: Vec<String>, rest: (Duration, Duration)) -> Pool {
        Pool {
            nodes: urls.into_iter().map(Node::new).collect(),
            next: 0,
            rest,
        }
    }

    pub(crate) fn urls(&self) -> Vec<String> {
        self.nodes.iter().map(|n| n.url.clone()).collect()
    }

    /// The next node to try: the next alive one in turn, or one whose rest
    /// is over. When every node is resting, the one that rests least, so a
    /// request always has somewhere to go.
    pub(crate) fn pick(&mut self, now: Instant) -> String {
        let len = self.nodes.len();
        for offset in 0..len {
            let i = (self.next + offset) % len;
            if self.nodes[i].dead_until.is_none_or(|until| until <= now) {
                self.next = (i + 1) % len;
                return self.nodes[i].url.clone();
            }
        }
        let soonest = self
            .nodes
            .iter()
            .min_by_key(|n| n.dead_until)
            .expect("a pool has a node");
        soonest.url.clone()
    }

    /// Every node, in the order to try them: those alive or done resting
    /// in turn, then the resting ones, soonest back first.
    pub(crate) fn in_turn(&mut self, now: Instant) -> Vec<String> {
        let len = self.nodes.len();
        let start = self.next;
        let (mut alive, mut resting): (Vec<&Node>, Vec<&Node>) = (0..len)
            .map(|offset| &self.nodes[(start + offset) % len])
            .partition(|n| n.dead_until.is_none_or(|until| until <= now));
        resting.sort_by_key(|n| n.dead_until);
        alive.extend(resting);
        alive.into_iter().map(|n| n.url.clone()).collect()
    }

    /// Whether any node is alive or done resting.
    pub(crate) fn any_alive(&self, now: Instant) -> bool {
        self.nodes
            .iter()
            .any(|n| n.dead_until.is_none_or(|until| until <= now))
    }

    /// A request to the node failed: it rests, longer each time in a row.
    pub(crate) fn failed(&mut self, url: &str, now: Instant) {
        if let Some(node) = self.nodes.iter_mut().find(|n| n.url == url) {
            node.failures = node.failures.saturating_add(1);
            let doublings = node.failures.saturating_sub(1).min(16);
            let rest = self.rest.0.saturating_mul(1 << doublings).min(self.rest.1);
            node.dead_until = Some(now + rest);
        }
    }

    /// A request to the node got an answer: it's alive.
    pub(crate) fn answered(&mut self, url: &str) {
        if let Some(node) = self.nodes.iter_mut().find(|n| n.url == url) {
            node.failures = 0;
            node.dead_until = None;
        }
    }

    /// The nodes sniffing found: those already known keep how they've been
    /// doing, new ones join, and those gone leave. Finding none changes
    /// nothing.
    pub(crate) fn replace(&mut self, urls: Vec<String>) {
        if urls.is_empty() {
            return;
        }
        let mut nodes = Vec::with_capacity(urls.len());
        for url in urls {
            match self.nodes.iter().find(|n| n.url == url) {
                Some(known) => nodes.push(known.clone()),
                None => nodes.push(Node::new(url)),
            }
        }
        self.nodes = nodes;
        self.next %= self.nodes.len();
    }
}

impl Node {
    fn new(url: String) -> Node {
        Node {
            url: url.trim_end_matches('/').to_owned(),
            dead_until: None,
            failures: 0,
        }
    }
}

/// A node's URL from the `publish_address` sniffing reads: `host:port`,
/// `hostname/ip:port` or `[ipv6]:port`. The hostname wins over the address,
/// for certificates.
pub(crate) fn published_url(scheme: &str, address: &str) -> Option<String> {
    let (host, port) = match address.split_once('/') {
        Some((hostname, rest)) if !hostname.is_empty() => {
            let (_, port) = rest.rsplit_once(':')?;
            (hostname, port)
        }
        Some((_, rest)) => rest.rsplit_once(':')?,
        None => address.rsplit_once(':')?,
    };
    port.parse::<u16>().ok()?;
    Some(format!("{scheme}://{host}:{port}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool() -> Pool {
        Pool::new(
            vec!["http://a:9200/".into(), "http://b:9200".into()],
            (Duration::from_secs(60), Duration::from_secs(1800)),
        )
    }

    #[test]
    fn round_robin_skips_resting_nodes() {
        let mut pool = pool();
        let now = Instant::now();
        assert_eq!(pool.pick(now), "http://a:9200");
        assert_eq!(pool.pick(now), "http://b:9200");
        pool.failed("http://a:9200", now);
        assert_eq!(pool.pick(now), "http://b:9200");
        assert_eq!(pool.pick(now), "http://b:9200");
        // After its rest, a node is tried again.
        let later = now + Duration::from_secs(61);
        assert_eq!(pool.pick(later), "http://a:9200");
        pool.answered("http://a:9200");
        assert_eq!(pool.pick(now), "http://b:9200");
        assert_eq!(pool.pick(now), "http://a:9200");
    }

    #[test]
    fn rests_grow_and_every_request_has_a_node() {
        let mut pool = pool();
        let now = Instant::now();
        pool.failed("http://a:9200", now);
        pool.failed("http://a:9200", now);
        pool.failed("http://b:9200", now);
        assert!(!pool.any_alive(now));
        // b rests 60 s, a 120 s: b is tried first.
        assert_eq!(pool.pick(now), "http://b:9200");
        assert!(pool.any_alive(now + Duration::from_secs(90)));
        assert_eq!(pool.pick(now + Duration::from_secs(90)), "http://b:9200");
        assert_eq!(pool.pick(now + Duration::from_secs(121)), "http://a:9200");
    }

    #[test]
    fn every_node_in_turn() {
        let mut pool = pool();
        let now = Instant::now();
        pool.failed("http://a:9200", now);
        assert_eq!(pool.in_turn(now), ["http://b:9200", "http://a:9200"]);
    }

    #[test]
    fn sniffed_nodes_replace_the_list() {
        let mut pool = pool();
        let now = Instant::now();
        pool.failed("http://b:9200", now);
        pool.replace(vec!["http://b:9200".into(), "http://c:9200".into()]);
        assert_eq!(pool.urls(), ["http://b:9200", "http://c:9200"]);
        // b still rests.
        assert_eq!(pool.pick(now), "http://c:9200");
        pool.replace(Vec::new());
        assert_eq!(pool.urls().len(), 2);
    }

    #[test]
    fn published_addresses() {
        assert_eq!(
            published_url("http", "127.0.0.1:9200").unwrap(),
            "http://127.0.0.1:9200"
        );
        assert_eq!(
            published_url("https", "es1.example.com/10.0.0.1:9243").unwrap(),
            "https://es1.example.com:9243"
        );
        assert_eq!(
            published_url("http", "/10.0.0.1:9200").unwrap(),
            "http://10.0.0.1:9200"
        );
        assert_eq!(
            published_url("http", "[::1]:9200").unwrap(),
            "http://[::1]:9200"
        );
        assert!(published_url("http", "nonsense").is_none());
    }
}
