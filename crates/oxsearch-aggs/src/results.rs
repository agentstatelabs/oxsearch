//! Typed aggregation results.

use oxsearch_core::shapes::TypedKeys;
use oxsearch_search::search::Hit;

use crate::{
    Aggregate, Buckets, DateHistogramBucket, DoubleTermsBucket, FiltersBucket, HistogramBucket,
    LongTermsBucket, Percentiles, RangeBucket, StatsAggregate, StringTermsBucket,
};

/// A search response's aggregation results, typed: the response type
/// `search::Response<T, Aggregates>`, for a request sent with `typed_keys`,
/// which names each result's type in its key (`sterms#by_tag`). Read them
/// with [`Results`]; a bucket's sub-aggregations read the same way.
pub type Aggregates = TypedKeys<Aggregate>;

/// Reading aggregation results by name, for a response's [`Aggregates`]
/// and a bucket's sub-aggregations alike. A name that isn't there, or is
/// another kind of aggregation, reads as `None` or no buckets.
pub trait Results {
    /// A result, by its aggregation's name.
    fn aggregate(&self, name: &str) -> Option<&Aggregate>;

    /// A single-value metric: an average, sum, minimum, maximum, value count,
    /// cardinality, or a pipeline's value. `None` when no document had a
    /// value.
    fn value(&self, name: &str) -> Option<f64> {
        match self.aggregate(name)? {
            Aggregate::Avg(a) => a.value,
            Aggregate::Sum(a) => a.value,
            Aggregate::Min(a) => a.value,
            Aggregate::Max(a) => a.value,
            Aggregate::ValueCount(a) => a.value,
            Aggregate::WeightedAvg(a) => a.value,
            Aggregate::MedianAbsoluteDeviation(a) => a.value,
            Aggregate::Cardinality(a) => Some(a.value as f64),
            Aggregate::SimpleValue(a) => a.value,
            Aggregate::Derivative(a) => a.value,
            Aggregate::BucketMetricValue(a) => a.value,
            _ => None,
        }
    }

    /// A stats aggregation's result.
    fn stats(&self, name: &str) -> Option<&StatsAggregate> {
        match self.aggregate(name)? {
            Aggregate::Stats(stats) => Some(stats),
            _ => None,
        }
    }

    /// A percentiles aggregation's values, by percent.
    fn percentiles(&self, name: &str) -> Vec<(f64, Option<f64>)> {
        let values = match self.aggregate(name) {
            Some(Aggregate::TDigestPercentiles(p)) => &p.values,
            Some(Aggregate::HdrPercentiles(p)) => &p.values,
            _ => return Vec::new(),
        };
        match values {
            Percentiles::Keyed(keyed) => keyed
                .iter()
                .filter_map(|(percent, value)| {
                    let percent = percent.parse::<f64>().ok()?;
                    Some((percent, serde_json::to_value(value).ok()?.as_f64()))
                })
                .collect(),
            Percentiles::Array(items) => items.iter().map(|i| (i.key, i.value)).collect(),
        }
    }

    /// A terms aggregation's buckets on a text or keyword field.
    fn string_terms(&self, name: &str) -> Vec<&StringTermsBucket> {
        match self.aggregate(name) {
            Some(Aggregate::StringTerms(t)) => buckets(&t.buckets),
            _ => Vec::new(),
        }
    }

    /// A terms aggregation's buckets on an integer field.
    fn long_terms(&self, name: &str) -> Vec<&LongTermsBucket> {
        match self.aggregate(name) {
            Some(Aggregate::LongTerms(t)) => buckets(&t.buckets),
            _ => Vec::new(),
        }
    }

    /// A terms aggregation's buckets on a floating-point field.
    fn double_terms(&self, name: &str) -> Vec<&DoubleTermsBucket> {
        match self.aggregate(name) {
            Some(Aggregate::DoubleTerms(t)) => buckets(&t.buckets),
            _ => Vec::new(),
        }
    }

    /// A terms aggregation's keys and document counts, whatever the field's
    /// type, most frequent first.
    fn terms(&self, name: &str) -> Vec<(String, i64)> {
        let mut out: Vec<(String, i64)> = self
            .string_terms(name)
            .iter()
            .map(|b| (field_text(&b.key), b.doc_count))
            .collect();
        out.extend(
            self.long_terms(name)
                .iter()
                .map(|b| (b.key.to_string(), b.doc_count)),
        );
        out.extend(
            self.double_terms(name)
                .iter()
                .map(|b| (b.key.to_string(), b.doc_count)),
        );
        out
    }

    /// A histogram's buckets.
    fn histogram(&self, name: &str) -> Vec<&HistogramBucket> {
        match self.aggregate(name) {
            Some(Aggregate::Histogram(h)) => buckets(&h.buckets),
            _ => Vec::new(),
        }
    }

    /// A date histogram's buckets.
    fn date_histogram(&self, name: &str) -> Vec<&DateHistogramBucket> {
        match self.aggregate(name) {
            Some(Aggregate::DateHistogram(h)) => buckets(&h.buckets),
            _ => Vec::new(),
        }
    }

    /// A range aggregation's buckets.
    fn ranges(&self, name: &str) -> Vec<&RangeBucket> {
        match self.aggregate(name) {
            Some(Aggregate::Range(r)) => buckets(&r.buckets),
            _ => Vec::new(),
        }
    }

    /// A filters aggregation's buckets, by the filters' names.
    fn filters(&self, name: &str) -> Vec<(String, &FiltersBucket)> {
        match self.aggregate(name) {
            Some(Aggregate::Filters(f)) => match &f.buckets {
                Buckets::Keyed(keyed) => keyed.iter().map(|(k, b)| (k.clone(), b)).collect(),
                Buckets::Array(array) => array
                    .iter()
                    .enumerate()
                    .map(|(n, b)| (b.key.clone().unwrap_or_else(|| n.to_string()), b))
                    .collect(),
            },
            _ => Vec::new(),
        }
    }

    /// A top hits aggregation's hits, with documents as JSON.
    fn top_hits(&self, name: &str) -> &[Hit] {
        match self.aggregate(name) {
            Some(Aggregate::TopHits(t)) => &t.hits.hits,
            _ => &[],
        }
    }
}

impl Results for TypedKeys<Aggregate> {
    fn aggregate(&self, name: &str) -> Option<&Aggregate> {
        self.0.get(name)
    }
}

/// A bucket aggregation's buckets, keyed or not, in order.
pub fn buckets<T>(buckets: &Buckets<T>) -> Vec<&T> {
    match buckets {
        Buckets::Keyed(keyed) => keyed.values().collect(),
        Buckets::Array(array) => array.iter().collect(),
    }
}

fn field_text(value: &oxsearch_core::FieldValue) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn typed_results() {
        let results: Aggregates = serde_json::from_value(json!({
            "sterms#by_tag": {
                "doc_count_error_upper_bound": 0,
                "sum_other_doc_count": 0,
                "buckets": [
                    {"key": "vector", "doc_count": 2, "avg#avg_price": {"value": 15.0}},
                    {"key": "ingest", "doc_count": 1, "avg#avg_price": {"value": 5.0}}
                ]
            },
            "stats#price": {"count": 3, "min": 5.0, "max": 20.0, "avg": 11.67, "sum": 35.0},
            "cardinality#tags": {"value": 2},
            "tdigest_percentiles#p": {"values": {"50.0": 10.0, "95.0": 20.0}}
        }))
        .unwrap();
        assert_eq!(
            results.terms("by_tag"),
            [("vector".into(), 2), ("ingest".into(), 1)]
        );
        let vector = results.string_terms("by_tag")[0];
        assert_eq!(vector.aggregations.value("avg_price"), Some(15.0));
        assert_eq!(results.stats("price").unwrap().max, Some(20.0));
        assert_eq!(results.value("tags"), Some(2.0));
        assert_eq!(
            results.percentiles("p"),
            [(50.0, Some(10.0)), (95.0, Some(20.0))]
        );
        assert!(results.histogram("by_tag").is_empty());
        assert!(results.aggregate("missing").is_none());
    }
}
