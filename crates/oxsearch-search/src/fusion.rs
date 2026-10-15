//! Several retrievers' results fused into one list, on the server when its
//! license includes the `rrf` and `linear` retrievers, else on the client.
//!
//! On the client, the retrievers run in one multi-search and their results
//! are combined the way Elasticsearch combines them: reciprocal rank fusion
//! scores a document `Σ weight / (rank_constant + rank)` over the lists it
//! is in, ranks counting from 1; a linear combination scores it
//! `Σ weight × normalized score`. Documents the server would rank equal may
//! come back in another order: Elasticsearch breaks ties by its internal
//! document ids, the client by the order it first saw them.
//!
//! ```ignore
//! let fusion = Fusion::rrf([
//!     Retriever::standard(Query::r#match("text", question)),
//!     Retriever::knn("embedding", embed(question), 10, 100),
//! ])
//! .size(5);
//! let fused = fusion.search::<Passage, _>(&client, "passages").await?;
//! if let Some(why) = &fused.fallback {
//!     println!("fused on the client: {why}");
//! }
//! ```

use indexmap::IndexMap;
use indexmap::map::Entry;
use oxsearch_core::{
    Client, Error, Indices, InnerRetriever, LicenseError, LinearRetriever, RRFRetriever,
    RRFRetrieverComponent, RRFRetrieverEntry, RawAggs, RetrieverContainer, ScoreNormalizer,
    Transport,
};
use serde::de::DeserializeOwned;

use crate::msearch::{self, MultisearchHeader, RequestItem, ResponseItem};
use crate::search::{self, Hit, SearchRequestBody};

/// Elasticsearch's default `rank_constant` for reciprocal rank fusion.
pub const RANK_CONSTANT: u32 = 60;

/// Elasticsearch's default number of hits.
const SIZE: i32 = 10;

/// Below this, Elasticsearch's normalizers treat a range or a norm as zero.
const EPSILON: f64 = 1e-6;

/// How the lists are combined.
#[derive(Clone, Debug, PartialEq)]
enum Method {
    Rrf { rank_constant: u32 },
    Linear { normalizer: ScoreNormalizer },
}

/// Several retrievers' results, fused into one list: build it with
/// [`Fusion::rrf`] or [`Fusion::linear`], then run it with
/// [`Fusion::search`].
#[derive(Clone, Debug)]
pub struct Fusion {
    retrievers: Vec<(RetrieverContainer, f32)>,
    method: Method,
    rank_window_size: Option<i32>,
    size: i32,
    body: SearchRequestBody,
}

/// Fused hits, and where they were fused.
#[derive(Debug)]
#[non_exhaustive]
pub struct Fused<TDocument> {
    /// The hits, best first, each with its fused score.
    pub hits: Vec<Hit<TDocument>>,
    /// Why the hits were fused on the client: the cluster's license doesn't
    /// include the retriever. `None` when the server fused them.
    pub fallback: Option<Box<LicenseError>>,
}

impl Fusion {
    /// Reciprocal rank fusion of the retrievers' results, each weighted 1.
    pub fn rrf<R: Into<RetrieverContainer>>(retrievers: impl IntoIterator<Item = R>) -> Fusion {
        Fusion::weighted_rrf(retrievers.into_iter().map(|retriever| (retriever, 1.0)))
    }

    /// Reciprocal rank fusion of the retrievers' results, with their
    /// weights.
    pub fn weighted_rrf<R: Into<RetrieverContainer>>(
        retrievers: impl IntoIterator<Item = (R, f32)>,
    ) -> Fusion {
        Fusion::new(
            retrievers,
            Method::Rrf {
                rank_constant: RANK_CONSTANT,
            },
        )
    }

    /// A weighted sum of the retrievers' scores, normalized by
    /// [`Fusion::normalizer`] (none by default).
    pub fn linear<R: Into<RetrieverContainer>>(
        retrievers: impl IntoIterator<Item = (R, f32)>,
    ) -> Fusion {
        Fusion::new(
            retrievers,
            Method::Linear {
                normalizer: ScoreNormalizer::None,
            },
        )
    }

    fn new<R: Into<RetrieverContainer>>(
        retrievers: impl IntoIterator<Item = (R, f32)>,
        method: Method,
    ) -> Fusion {
        Fusion {
            retrievers: retrievers
                .into_iter()
                .map(|(retriever, weight)| (retriever.into(), weight))
                .collect(),
            method,
            rank_window_size: None,
            size: SIZE,
            body: SearchRequestBody::new(),
        }
    }

    /// For reciprocal rank fusion, how much a document's rank in a list
    /// counts against its rank in the others: higher evens them out. The
    /// default is 60.
    pub fn rank_constant(mut self, rank_constant: u32) -> Fusion {
        if let Method::Rrf { rank_constant: k } = &mut self.method {
            *k = rank_constant;
        }
        self
    }

    /// For a linear combination, how each retriever's scores are normalized
    /// before they're weighted: `Minmax` to the range 0 to 1, `L2Norm` to a
    /// unit vector, or `None`, the default, not at all.
    pub fn normalizer(mut self, normalizer: ScoreNormalizer) -> Fusion {
        if let Method::Linear { normalizer: n } = &mut self.method {
            *n = normalizer;
        }
        self
    }

    /// How many of each retriever's hits are fused: at least the number of
    /// hits returned, which is the default.
    pub fn rank_window_size(mut self, rank_window_size: i32) -> Fusion {
        self.rank_window_size = Some(rank_window_size);
        self
    }

    /// How many hits are returned. The default is 10.
    pub fn size(mut self, size: i32) -> Fusion {
        self.size = size;
        self
    }

    /// The rest of each search, such as `_source` filtering. Its retriever,
    /// size and aggregations are replaced; it mustn't set a query, a kNN
    /// search or a sort, which a retriever replaces.
    pub fn body(mut self, body: SearchRequestBody) -> Fusion {
        self.body = body;
        self
    }

    fn window(&self) -> i32 {
        self.rank_window_size.unwrap_or(self.size)
    }

    /// Elasticsearch's rule for the sizes, kept on the client too.
    fn check(&self) -> Result<(), Error> {
        if self.window() < self.size {
            return Err(Error::Parameter(format!(
                "rank_window_size ({}) must be at least size ({})",
                self.window(),
                self.size
            )));
        }
        Ok(())
    }

    /// The `rrf` or `linear` retriever that fuses on the server.
    pub fn retriever(&self) -> RetrieverContainer {
        match &self.method {
            Method::Rrf { rank_constant } => {
                let entries: Vec<RRFRetrieverEntry> = self
                    .retrievers
                    .iter()
                    .map(|(retriever, weight)| {
                        if *weight == 1.0 {
                            RRFRetrieverEntry::Retriever(retriever.clone())
                        } else {
                            RRFRetrieverEntry::Weighted(
                                RRFRetrieverComponent::new(retriever.clone()).weight(*weight),
                            )
                        }
                    })
                    .collect();
                RRFRetriever::new(entries)
                    .rank_constant(i32::try_from(*rank_constant).unwrap_or(i32::MAX))
                    .rank_window_size(self.window())
                    .into()
            }
            Method::Linear { normalizer } => {
                let inner: Vec<InnerRetriever> = self
                    .retrievers
                    .iter()
                    .map(|(retriever, weight)| {
                        InnerRetriever::new(retriever.clone()).weight(*weight)
                    })
                    .collect();
                LinearRetriever::new()
                    .retrievers(inner)
                    .normalizer(normalizer.clone())
                    .rank_window_size(self.window())
                    .into()
            }
        }
    }

    /// The multi-search that runs each retriever on its own, for fusing on
    /// the client.
    pub fn request(&self, index: impl Into<Indices>) -> msearch::Request {
        let mut items = Vec::with_capacity(2 * self.retrievers.len());
        for (retriever, _) in &self.retrievers {
            let mut body = self.body.clone();
            body.aggregations = None;
            items.push(RequestItem::Header(MultisearchHeader::new()));
            items.push(RequestItem::Body(
                body.retriever(retriever.clone()).size(self.window()),
            ));
        }
        msearch::Request::new().index(index).body(items)
    }

    /// Fuses on the server if the cluster's license includes the retriever,
    /// else on the client.
    pub async fn search<TDocument: DeserializeOwned, T: Transport>(
        &self,
        client: &Client<T>,
        index: impl Into<Indices>,
    ) -> Result<Fused<TDocument>, Error> {
        let index = index.into();
        match self.on_server(client, index.clone()).await {
            Ok(hits) => Ok(Fused {
                hits,
                fallback: None,
            }),
            Err(Error::License(why)) => Ok(Fused {
                hits: self.on_client(client, index).await?,
                fallback: Some(why),
            }),
            Err(e) => Err(e),
        }
    }

    /// Fuses on the server, with the `rrf` or `linear` retriever.
    pub async fn on_server<TDocument: DeserializeOwned, T: Transport>(
        &self,
        client: &Client<T>,
        index: impl Into<Indices>,
    ) -> Result<Vec<Hit<TDocument>>, Error> {
        self.check()?;
        // The search endpoint's body has the same properties as a
        // multi-search's.
        let body = serde_json::to_value(&self.body).map_err(Error::Encode)?;
        let body: search::RequestBody = serde_json::from_value(body).map_err(Error::Encode)?;
        let request = search::Request::new()
            .index(index)
            .body(body.retriever(self.retriever()).size(self.size));
        let response: search::Response<TDocument, RawAggs> = client.send_as(&request).await?;
        Ok(response.into_results())
    }

    /// Fuses on the client: runs the retrievers in one multi-search, then
    /// combines their results.
    pub async fn on_client<TDocument: DeserializeOwned, T: Transport>(
        &self,
        client: &Client<T>,
        index: impl Into<Indices>,
    ) -> Result<Vec<Hit<TDocument>>, Error> {
        self.check()?;
        let response: msearch::Response<TDocument, RawAggs> =
            client.send_as(&self.request(index)).await?;
        let mut lists = Vec::with_capacity(self.retrievers.len());
        for item in response.0.responses {
            match item {
                ResponseItem::Result(result) => lists.push(result.into_results()),
                ResponseItem::Failure(failure) => {
                    let status = failure
                        .status
                        .and_then(|status| u16::try_from(status).ok())
                        .unwrap_or(500);
                    let body = serde_json::to_vec(&failure).map_err(Error::Encode)?;
                    return Err(Error::from_response(status, &body));
                }
            }
        }
        self.fuse(lists)
    }

    /// Combines result lists, one per retriever in order, best hit first:
    /// for fusing the results of searches you ran yourself.
    pub fn fuse<TDocument>(
        &self,
        lists: Vec<Vec<Hit<TDocument>>>,
    ) -> Result<Vec<Hit<TDocument>>, Error> {
        self.check()?;
        let mut fused: IndexMap<Key, (f64, Hit<TDocument>)> = IndexMap::new();
        for (n, (mut list, (_, weight))) in lists.into_iter().zip(&self.retrievers).enumerate() {
            list.truncate(usize::try_from(self.window()).unwrap_or(0));
            let scores = match &self.method {
                Method::Rrf { rank_constant } => (1..=list.len())
                    .map(|rank| 1.0 / (f64::from(*rank_constant) + rank as f64))
                    .collect(),
                Method::Linear { normalizer } => {
                    let scores: Vec<f64> =
                        list.iter().map(|hit| hit.score.unwrap_or(0.0)).collect();
                    normalize(normalizer, scores)?
                }
            };
            for (position, (hit, score)) in list.into_iter().zip(scores).enumerate() {
                let score = f64::from(*weight) * score;
                let key = match (&hit.index, &hit.id) {
                    (index, Some(id)) => Key::Document(index.clone(), id.clone()),
                    (_, None) => Key::Position(n, position),
                };
                match fused.entry(key) {
                    Entry::Occupied(mut entry) => entry.get_mut().0 += score,
                    Entry::Vacant(entry) => {
                        entry.insert((score, hit));
                    }
                }
            }
        }
        let mut hits: Vec<(f64, Hit<TDocument>)> = fused.into_values().collect();
        // A stable sort: equal scores keep the order they were first seen in.
        hits.sort_by(|a, b| b.0.total_cmp(&a.0));
        hits.truncate(usize::try_from(self.size).unwrap_or(0));
        Ok(hits
            .into_iter()
            .map(|(score, mut hit)| {
                hit.score = Some(score);
                hit
            })
            .collect())
    }
}

/// What makes two hits the same document.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Key {
    Document(Option<String>, String),
    /// A hit without an id, which can't be matched with another.
    Position(usize, usize),
}

/// One retriever's scores, normalized as Elasticsearch's linear retriever
/// normalizes them.
fn normalize(normalizer: &ScoreNormalizer, scores: Vec<f64>) -> Result<Vec<f64>, Error> {
    let valid = || scores.iter().copied().filter(|score| !score.is_nan());
    if valid().next().is_none() {
        return Ok(scores);
    }
    Ok(match normalizer {
        ScoreNormalizer::None => scores,
        ScoreNormalizer::Minmax => {
            let min = valid().fold(f64::INFINITY, f64::min);
            let max = valid().fold(f64::NEG_INFINITY, f64::max);
            if max - min < EPSILON {
                // One hit, or hits scored the same.
                vec![1.0; scores.len()]
            } else {
                scores
                    .iter()
                    .map(|score| (score - min) / (max - min))
                    .collect()
            }
        }
        ScoreNormalizer::L2Norm => {
            let norm = valid().map(|score| score * score).sum::<f64>().sqrt();
            if norm < EPSILON {
                scores
            } else {
                scores.iter().map(|score| score / norm).collect()
            }
        }
        other => {
            return Err(Error::Parameter(format!(
                "oxsearch can't apply the score normalizer `{}` on the client",
                other.as_str()
            )));
        }
    })
}

#[cfg(test)]
mod tests {
    use oxsearch_core::{Query, Retriever};
    use serde_json::{Value, json};

    use super::*;

    fn hits(ids: &[(&str, f64)]) -> Vec<Hit> {
        ids.iter()
            .map(|(id, score)| {
                serde_json::from_value(json!({"_index": "i", "_id": id, "_score": score})).unwrap()
            })
            .collect()
    }

    fn scores(hits: &[Hit]) -> Vec<(String, f64)> {
        hits.iter()
            .map(|hit| {
                let score = (hit.score.unwrap() * 1e6).round() / 1e6;
                (hit.id.clone().unwrap(), score)
            })
            .collect()
    }

    fn two() -> [RetrieverContainer; 2] {
        [
            Retriever::standard(Query::r#match("text", "hnsw")),
            Retriever::knn("embedding", vec![1.0, 0.0], 3, 10),
        ]
    }

    #[test]
    fn reciprocal_rank_fusion() {
        let fusion = Fusion::rrf(two()).rank_constant(1).size(3);
        let fused = fusion
            .fuse(vec![
                hits(&[("a", 9.0), ("b", 5.0), ("c", 1.0)]),
                hits(&[("c", 0.9), ("a", 0.8)]),
            ])
            .unwrap();
        // a: 1/2 + 1/3, c: 1/4 + 1/2, b: 1/3.
        assert_eq!(
            scores(&fused),
            [
                ("a".into(), 0.833333),
                ("c".into(), 0.75),
                ("b".into(), 0.333333)
            ]
        );
    }

    #[test]
    fn weights_and_window() {
        let [keywords, neighbours] = two();
        let fusion = Fusion::weighted_rrf([(keywords, 1.0), (neighbours, 2.0)])
            .rank_constant(1)
            .rank_window_size(3)
            .size(3);
        let fused = fusion
            .fuse(vec![
                hits(&[("a", 9.0), ("b", 5.0), ("c", 3.0), ("d", 1.0)]),
                hits(&[("d", 0.9)]),
            ])
            .unwrap();
        // d: 2/2 from the second list; its rank of 4 in the first is outside
        // the window. a: 1/2, b: 1/3; c's 1/4 is cut by the size.
        assert_eq!(
            scores(&fused),
            [("d".into(), 1.0), ("a".into(), 0.5), ("b".into(), 0.333333)]
        );
    }

    #[test]
    fn linear_combinations() {
        let [keywords, neighbours] = two();
        let lists = || {
            vec![
                hits(&[("a", 4.0), ("b", 2.0), ("c", 0.0)]),
                hits(&[("c", 0.5)]),
            ]
        };
        let linear = Fusion::linear([(keywords, 1.0), (neighbours, 2.0)]);
        assert_eq!(
            scores(&linear.clone().fuse(lists()).unwrap()),
            [("a".into(), 4.0), ("b".into(), 2.0), ("c".into(), 1.0)]
        );
        // Min-max: a single hit normalizes to 1.
        let minmax = linear.clone().normalizer(ScoreNormalizer::Minmax);
        assert_eq!(
            scores(&minmax.fuse(lists()).unwrap()),
            [("c".into(), 2.0), ("a".into(), 1.0), ("b".into(), 0.5)]
        );
        let l2 = linear.clone().normalizer(ScoreNormalizer::L2Norm);
        assert_eq!(
            scores(&l2.fuse(lists()).unwrap()),
            [
                ("c".into(), 2.0),
                ("a".into(), 0.894427),
                ("b".into(), 0.447214)
            ]
        );
        let unknown = linear.normalizer(ScoreNormalizer::Raw("softmax".into()));
        assert!(matches!(unknown.fuse(lists()), Err(Error::Parameter(_))));
    }

    #[test]
    fn the_window_holds_the_hits() {
        let fusion = Fusion::rrf(two()).rank_window_size(2).size(3);
        assert!(matches!(
            fusion.fuse(vec![hits(&[])]),
            Err(Error::Parameter(_))
        ));
    }

    #[test]
    fn hits_without_ids_stay_apart() {
        let fusion = Fusion::rrf(two());
        let anonymous: Vec<Hit> = vec![serde_json::from_value(json!({"_score": 1.0})).unwrap()];
        let fused = fusion.fuse(vec![anonymous.clone(), anonymous]).unwrap();
        assert_eq!(fused.len(), 2);
    }

    #[test]
    fn retrievers_for_the_server() {
        let [keywords, neighbours] = two();
        let rrf =
            Fusion::weighted_rrf([(keywords.clone(), 1.0), (neighbours.clone(), 2.0)]).size(4);
        let rrf: Value = serde_json::to_value(rrf.retriever()).unwrap();
        assert_eq!(rrf["rrf"]["rank_constant"], 60);
        assert_eq!(rrf["rrf"]["rank_window_size"], 4);
        assert!(rrf["rrf"]["retrievers"][0]["standard"].is_object());
        assert_eq!(rrf["rrf"]["retrievers"][1]["weight"], 2.0);
        let linear = Fusion::linear([(keywords, 1.0), (neighbours, 2.0)])
            .normalizer(ScoreNormalizer::Minmax)
            .rank_window_size(20);
        let linear: Value = serde_json::to_value(linear.retriever()).unwrap();
        assert_eq!(linear["linear"]["normalizer"], "minmax");
        assert_eq!(linear["linear"]["rank_window_size"], 20);
        assert_eq!(linear["linear"]["retrievers"][1]["weight"], 2.0);
    }

    #[test]
    fn the_multi_search() {
        use oxsearch_core::Endpoint;

        let fusion = Fusion::rrf(two()).size(4).body(
            SearchRequestBody::new()
                .source(false)
                .aggregations(json!({"by_topic": {"terms": {"field": "topic"}}})),
        );
        let request = fusion.request("passages");
        assert_eq!(
            Endpoint::route(&request).unwrap().path,
            "/passages/_msearch"
        );
        let body = Endpoint::body(&request).unwrap().unwrap().into_bytes();
        let lines: Vec<Value> = String::from_utf8(body)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], json!({}));
        assert_eq!(lines[1]["size"], 4);
        assert_eq!(lines[1]["_source"], false);
        assert!(lines[1].get("aggregations").is_none());
        assert!(lines[1]["retriever"]["standard"].is_object());
        assert!(lines[3]["retriever"]["knn"].is_object());
    }
}
