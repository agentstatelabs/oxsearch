//! Every document a query matches, as a stream, over a point in time.

use futures_core::Stream;
use futures_util::stream::{self, StreamExt, TryStreamExt};
use oxsearch_core::query_dsl::QueryContainer;
use oxsearch_core::{Client, Error, SlicedScroll, SortCombinations, SortResults, Transport};
use oxsearch_search::search::{self, Hit, PointInTimeReference};
use oxsearch_search::{close_point_in_time, open_point_in_time};
use serde::de::DeserializeOwned;

/// Reads every document a query matches, page by page, over a point in
/// time: a consistent view of the index that writes after it don't change.
///
/// ```ignore
/// let mut hits = Scan::new("articles")
///     .query(Article::TAGS.term("vector"))
///     .slices(4)
///     .stream::<Article, _>(&client);
/// while let Some(hit) = hits.try_next().await? {
///     println!("{:?}", hit.source);
/// }
/// ```
///
/// It counts pages and documents read through the `metrics` facade, as
/// `oxsearch.scan.pages` and `oxsearch.scan.documents`.
///
/// Each page renews the point in time's short keep-alive, the stream ends
/// at the first empty page, so no exact total is needed, and the point in
/// time is closed when the stream ends. A stream dropped before its end
/// leaves the point in time to expire after its keep-alive.
#[derive(Clone, Debug)]
pub struct Scan {
    index: String,
    query: Option<QueryContainer>,
    keep_alive: String,
    page_size: i32,
    slices: u32,
    body: search::RequestBody,
}

/// One page of a slice, with the point in time's id as of that page.
struct Page<T> {
    pit: String,
    hits: Vec<Hit<T>>,
}

impl Scan {
    /// Every document in an index, or in indices and aliases with commas
    /// between them.
    pub fn new(index: impl Into<String>) -> Scan {
        Scan {
            index: index.into(),
            query: None,
            keep_alive: "1m".to_owned(),
            page_size: 1000,
            slices: 1,
            body: search::RequestBody::new(),
        }
    }

    /// Only the documents a query matches.
    pub fn query(mut self, query: impl Into<QueryContainer>) -> Scan {
        self.query = Some(query.into());
        self
    }

    /// How long the point in time lives without a page read, renewed with
    /// each page: `"1m"` by default.
    pub fn keep_alive(mut self, keep_alive: impl Into<String>) -> Scan {
        self.keep_alive = keep_alive.into();
        self
    }

    /// Documents per page: 1,000 by default.
    pub fn page_size(mut self, page_size: i32) -> Scan {
        self.page_size = page_size;
        self
    }

    /// Reads the index in this many slices at once, each a stream of its
    /// own, merged as pages arrive. 1, the default, doesn't slice.
    pub fn slices(mut self, slices: u32) -> Scan {
        self.slices = slices.max(1);
        self
    }

    /// The rest of each search, such as `_source` filtering. Its query,
    /// sort, size, point in time and `search_after` are replaced.
    pub fn body(mut self, body: search::RequestBody) -> Scan {
        self.body = body;
        self
    }

    /// The documents, as a stream of hits in no particular order. The
    /// first error ends it.
    pub fn stream<T, Tr>(
        self,
        client: &Client<Tr>,
    ) -> impl Stream<Item = Result<Hit<T>, Error>> + Send + 'static
    where
        T: DeserializeOwned + Send + 'static,
        Tr: Transport + Clone + 'static,
    {
        let client = client.clone();
        let open = {
            let client = client.clone();
            let request =
                open_point_in_time::Request::new(self.index.as_str(), self.keep_alive.as_str());
            async move {
                let response = client.send(&request).await?;
                response.id.clone().ok_or_else(|| {
                    Error::Parameter("Elasticsearch opened a point in time without an id".into())
                })
            }
        };
        stream::once(open)
            .map_ok(move |pit| self.clone().pages::<T, Tr>(client.clone(), pit))
            .try_flatten()
            .map_ok(|hits| stream::iter(hits.into_iter().map(Ok)))
            .try_flatten()
    }

    /// Every slice's pages, merged, then the point in time closed.
    fn pages<T, Tr>(
        self,
        client: Client<Tr>,
        pit: String,
    ) -> impl Stream<Item = Result<Vec<Hit<T>>, Error>> + Send + 'static
    where
        T: DeserializeOwned + Send + 'static,
        Tr: Transport + Clone + 'static,
    {
        let slices = self.slices;
        let merged = stream::select_all((0..slices).map(|slice| {
            let slice = (slices > 1).then_some((slice, slices));
            self.slice::<T, Tr>(client.clone(), pit.clone(), slice)
                .boxed()
        }));
        // The point in time's latest id travels with the pages, so closing
        // it needs no state shared with the slices.
        enum State<S, Tr> {
            Reading {
                pages: S,
                pit: String,
                client: Client<Tr>,
            },
            Done,
        }
        stream::unfold(
            State::Reading {
                pages: merged,
                pit,
                client,
            },
            |state| async move {
                let State::Reading {
                    mut pages,
                    mut pit,
                    client,
                } = state
                else {
                    return None;
                };
                match pages.next().await {
                    Some(Ok(page)) => {
                        pit = page.pit;
                        Some((Ok(page.hits), State::Reading { pages, pit, client }))
                    }
                    Some(Err(error)) => {
                        close(&client, pit).await;
                        Some((Err(error), State::Done))
                    }
                    None => {
                        close(&client, pit).await;
                        None
                    }
                }
            },
        )
    }

    /// One slice's pages, each after the last hit's sort values, until the
    /// first empty page.
    fn slice<T, Tr>(
        &self,
        client: Client<Tr>,
        pit: String,
        slice: Option<(u32, u32)>,
    ) -> impl Stream<Item = Result<Page<T>, Error>> + Send + 'static
    where
        T: DeserializeOwned + Send + 'static,
        Tr: Transport + Clone + 'static,
    {
        let mut body = self
            .body
            .clone()
            .size(self.page_size)
            .sort(SortCombinations::from("_shard_doc"))
            .track_total_hits(false);
        if let Some(query) = &self.query {
            body = body.query(query.clone());
        }
        if let Some((id, max)) = slice {
            body = body.slice(SlicedScroll::new(id.to_string(), max as i32));
        }
        let keep_alive = self.keep_alive.clone();
        stream::try_unfold(Some((pit, None::<SortResults>)), move |state| {
            let client = client.clone();
            let body = body.clone();
            let keep_alive = keep_alive.clone();
            async move {
                let Some((pit, after)) = state else {
                    return Ok(None);
                };
                let mut body =
                    body.pit(PointInTimeReference::new(pit.as_str()).keep_alive(keep_alive));
                if let Some(after) = after {
                    body = body.search_after(after);
                }
                let request = search::Request::new().body(body);
                let response: search::Response<T> = client.send_as(&request).await?;
                let pit = response.pit_id.clone().unwrap_or(pit);
                let hits = response.into_results();
                metrics::counter!("oxsearch.scan.pages").increment(1);
                metrics::counter!("oxsearch.scan.documents").increment(hits.len() as u64);
                if hits.is_empty() {
                    // The end, with the latest id for closing.
                    return Ok(Some((Page { pit, hits }, None)));
                }
                let next = hits
                    .last()
                    .and_then(|hit| hit.sort.clone())
                    .map(|after| (pit.clone(), Some(after)));
                Ok(Some((Page { pit, hits }, next)))
            }
        })
    }
}

/// Closes a point in time. A failure leaves it to expire after its
/// keep-alive, which the documents read don't depend on.
async fn close<Tr: Transport>(client: &Client<Tr>, pit: String) {
    let request =
        close_point_in_time::Request::new().body(close_point_in_time::RequestBody::new(pit));
    let _ = client.send(&request).await;
}
