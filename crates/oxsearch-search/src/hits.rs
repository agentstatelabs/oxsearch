//! Reading the hits of search responses.
//!
//! A response with no `hits` reads as one with no hits, so these don't
//! return options.

use crate::msearch::MultiSearchItem;
use crate::search::{Hit, Response, ResponseBody};
use crate::{scroll, search_template};

/// `results`, `documents` and `into_results` for the response types that
/// hold hits.
macro_rules! hits {
    ($($ty:ty),* $(,)?) => {
        $(
            impl<TDocument, TAggregations> $ty {
                /// The hits, best first.
                pub fn results(&self) -> &[Hit<TDocument>] {
                    self.hits.as_ref().map_or(&[], |hits| &hits.hits)
                }

                /// The documents of the hits that have a `_source`, best
                /// first.
                pub fn documents(&self) -> impl Iterator<Item = &TDocument> {
                    self.results().iter().filter_map(|hit| hit.source.as_ref())
                }

                /// The hits, best first, taken out of the response.
                pub fn into_results(self) -> Vec<Hit<TDocument>> {
                    self.hits.map(|hits| hits.hits).unwrap_or_default()
                }
            }
        )*
    };
}

hits!(
    ResponseBody<TDocument, TAggregations>,
    MultiSearchItem<TDocument, TAggregations>,
    search_template::Response<TDocument, TAggregations>,
);

impl<TDocument, TAggregations> Response<TDocument, TAggregations> {
    /// The hits, best first, taken out of the response.
    pub fn into_results(self) -> Vec<Hit<TDocument>> {
        self.0.into_results()
    }
}

impl<TDocument, TAggregations> scroll::Response<TDocument, TAggregations> {
    /// The hits, best first, taken out of the response.
    pub fn into_results(self) -> Vec<Hit<TDocument>> {
        self.0.into_results()
    }
}
