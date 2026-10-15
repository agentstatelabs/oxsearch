//! Open types where one crate would otherwise depend on another: a search
//! request's aggregations and a search response's aggregation results, so
//! that search doesn't depend on aggs.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A search's aggregations, as the request sends them: a JSON object of
/// named aggregations. `json!` values convert into it, and so will the
/// types in oxsearch-aggs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AggsBody(pub Value);

impl From<Value> for AggsBody {
    fn from(aggregations: Value) -> AggsBody {
        AggsBody(aggregations)
    }
}

impl From<Map<String, Value>> for AggsBody {
    fn from(aggregations: Map<String, Value>) -> AggsBody {
        AggsBody(Value::Object(aggregations))
    }
}

/// A search's aggregation results, as JSON by aggregation name: the default
/// for a search response's `TAggregations`. Name your own type there, such
/// as `search::Response<Article, MyAggregations>`, to read them typed.
pub type RawAggs = IndexMap<String, Value>;
