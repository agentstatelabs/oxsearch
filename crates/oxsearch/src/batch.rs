//! A bulk request of actions already written as NDJSON, which the bulk
//! loader and the document layer send.

use oxsearch_core::__private::encode as percent_encode;
use oxsearch_core::{Body, Endpoint, Error, Method, Refresh, Route};
use oxsearch_docs::bulk;

pub(crate) struct Batch<'a> {
    /// The index for actions that don't name one.
    pub index: Option<&'a str>,
    pub refresh: Option<&'a Refresh>,
    pub body: Vec<u8>,
}

impl Endpoint for Batch<'_> {
    type Response = bulk::Response;

    fn name(&self) -> &'static str {
        "bulk"
    }

    fn route(&self) -> Result<Route, Error> {
        Ok(match self.index {
            Some(index) => Route::new(Method::Post, format!("/{}/_bulk", percent_encode(index))),
            None => Route::new(Method::Post, "/_bulk"),
        })
    }

    fn query(&self) -> Result<Vec<(&'static str, String)>, Error> {
        Ok(self
            .refresh
            .map(|refresh| vec![("refresh", refresh.as_str().to_owned())])
            .unwrap_or_default())
    }

    fn body(&self) -> Result<Option<Body>, Error> {
        Ok(Some(Body::Ndjson(self.body.clone())))
    }
}
