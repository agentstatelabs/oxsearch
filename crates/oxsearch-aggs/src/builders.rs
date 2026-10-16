//! Builders for the common aggregations, and the set of named aggregations
//! a search request takes.

use oxsearch_core::query_dsl::QueryContainer;
use oxsearch_core::shapes::IndexMap;
use oxsearch_core::{AggsBody, Field};

use crate::{
    AggregationContainer, AggregationContainerVariant, AggregationRange, AverageAggregation,
    AverageBucketAggregation, BucketsPath, CalendarInterval, CardinalityAggregation,
    CumulativeSumAggregation, DateHistogramAggregation, DerivativeAggregation, FiltersAggregation,
    HistogramAggregation, MaxAggregation, MaxBucketAggregation, MinAggregation,
    MinBucketAggregation, PercentilesAggregation, RangeAggregation, StatsAggregation,
    SumAggregation, SumBucketAggregation, TermsAggregation, TopHitsAggregation,
    ValueCountAggregation,
};

/// Named aggregations, for a search request's body:
///
/// ```ignore
/// let body = search::RequestBody::new().size(0).aggregations(
///     Aggs::new()
///         .add("by_tag", Agg::terms("tag", 10).sub("avg_price", Agg::avg("price")))
///         .add("price", Agg::stats("price")),
/// );
/// ```
///
/// Send it with `typed_keys`, and read the results as [`Aggregates`](crate::Aggregates).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Aggs(pub IndexMap<String, AggregationContainer>);

impl Aggs {
    /// No aggregations yet.
    pub fn new() -> Aggs {
        Aggs::default()
    }

    /// The aggregations with one more, by name.
    pub fn add(mut self, name: impl Into<String>, aggregation: AggregationContainer) -> Aggs {
        self.0.insert(name.into(), aggregation);
        self
    }
}

impl<N: Into<String>> FromIterator<(N, AggregationContainer)> for Aggs {
    fn from_iter<I: IntoIterator<Item = (N, AggregationContainer)>>(iter: I) -> Aggs {
        Aggs(iter.into_iter().map(|(n, a)| (n.into(), a)).collect())
    }
}

impl From<Aggs> for AggsBody {
    fn from(aggs: Aggs) -> AggsBody {
        // Generated types always write as JSON.
        AggsBody(serde_json::to_value(aggs.0).expect("aggregations write as JSON"))
    }
}

impl AggregationContainer {
    /// The aggregation with a sub-aggregation, run in each of its buckets.
    pub fn sub(mut self, name: impl Into<String>, aggregation: AggregationContainer) -> Self {
        self.aggregations
            .get_or_insert_with(IndexMap::new)
            .insert(name.into(), aggregation);
        self
    }
}

/// Builders for the common aggregations. Each returns an
/// [`AggregationContainer`]; for an option a builder doesn't take, build the
/// generated aggregation and wrap it with [`Agg::of`].
pub struct Agg;

impl Agg {
    /// Any aggregation: `Agg::of(TermsAggregation::new().field("tag").min_doc_count(2))`.
    pub fn of(aggregation: impl Into<AggregationContainerVariant>) -> AggregationContainer {
        AggregationContainer::new().variant(aggregation)
    }

    // Buckets.

    /// The `size` most frequent values of `field`, a bucket each.
    pub fn terms(field: impl Into<Field>, size: i32) -> AggregationContainer {
        Agg::of(TermsAggregation::new().field(field).size(size))
    }

    /// Buckets `interval` wide of a numeric field.
    pub fn histogram(field: impl Into<Field>, interval: f64) -> AggregationContainer {
        Agg::of(HistogramAggregation::new().field(field).interval(interval))
    }

    /// Buckets of a date field, a calendar unit each: a day, a month.
    pub fn date_histogram(
        field: impl Into<Field>,
        interval: CalendarInterval,
    ) -> AggregationContainer {
        Agg::of(
            DateHistogramAggregation::new()
                .field(field)
                .calendar_interval(interval),
        )
    }

    /// Buckets of a date field, a fixed time each: `"30m"`, `"7d"`.
    pub fn date_histogram_fixed(
        field: impl Into<Field>,
        interval: impl Into<String>,
    ) -> AggregationContainer {
        Agg::of(
            DateHistogramAggregation::new()
                .field(field)
                .fixed_interval(interval.into()),
        )
    }

    /// A bucket for each range of a numeric field, `from` inclusive and `to`
    /// exclusive, either open.
    pub fn range(
        field: impl Into<Field>,
        ranges: impl IntoIterator<Item = (Option<f64>, Option<f64>)>,
    ) -> AggregationContainer {
        let ranges: Vec<AggregationRange> = ranges
            .into_iter()
            .map(|(from, to)| {
                let mut range = AggregationRange::new();
                if let Some(from) = from {
                    range = range.from(from);
                }
                if let Some(to) = to {
                    range = range.to(to);
                }
                range
            })
            .collect();
        Agg::of(RangeAggregation::new().field(field).ranges(ranges))
    }

    /// A bucket for each named query, holding the documents it matches.
    pub fn filters<N: Into<String>>(
        filters: impl IntoIterator<Item = (N, QueryContainer)>,
    ) -> AggregationContainer {
        let filters: IndexMap<String, QueryContainer> =
            filters.into_iter().map(|(n, q)| (n.into(), q)).collect();
        Agg::of(FiltersAggregation::new().filters(crate::Buckets::Keyed(filters)))
    }

    // Metrics.

    /// The average of a numeric field.
    pub fn avg(field: impl Into<Field>) -> AggregationContainer {
        Agg::of(AverageAggregation::new().field(field))
    }

    /// The sum of a numeric field.
    pub fn sum(field: impl Into<Field>) -> AggregationContainer {
        Agg::of(SumAggregation::new().field(field))
    }

    /// The smallest value of a numeric field.
    pub fn min(field: impl Into<Field>) -> AggregationContainer {
        Agg::of(MinAggregation::new().field(field))
    }

    /// The largest value of a numeric field.
    pub fn max(field: impl Into<Field>) -> AggregationContainer {
        Agg::of(MaxAggregation::new().field(field))
    }

    /// How many values a field has.
    pub fn value_count(field: impl Into<Field>) -> AggregationContainer {
        Agg::of(ValueCountAggregation::new().field(field))
    }

    /// About how many distinct values a field has.
    pub fn cardinality(field: impl Into<Field>) -> AggregationContainer {
        Agg::of(CardinalityAggregation::new().field(field))
    }

    /// The count, min, max, average and sum of a numeric field.
    pub fn stats(field: impl Into<Field>) -> AggregationContainer {
        Agg::of(StatsAggregation::new().field(field))
    }

    /// Percentiles of a numeric field: `[50.0, 95.0, 99.0]`.
    pub fn percentiles(field: impl Into<Field>, percents: Vec<f64>) -> AggregationContainer {
        Agg::of(
            PercentilesAggregation::new()
                .field(field)
                .percents(percents),
        )
    }

    /// The best `size` hits in each bucket.
    pub fn top_hits(size: i32) -> AggregationContainer {
        Agg::of(TopHitsAggregation::new().size(size))
    }

    // Pipelines, over sibling or parent aggregations by path: `"by_month>sales"`.

    /// The average of a metric over a sibling's buckets.
    pub fn avg_bucket(path: impl Into<String>) -> AggregationContainer {
        Agg::of(AverageBucketAggregation::new().buckets_path(BucketsPath::Single(path.into())))
    }

    /// The sum of a metric over a sibling's buckets.
    pub fn sum_bucket(path: impl Into<String>) -> AggregationContainer {
        Agg::of(SumBucketAggregation::new().buckets_path(BucketsPath::Single(path.into())))
    }

    /// The largest value of a metric over a sibling's buckets, and its keys.
    pub fn max_bucket(path: impl Into<String>) -> AggregationContainer {
        Agg::of(MaxBucketAggregation::new().buckets_path(BucketsPath::Single(path.into())))
    }

    /// The smallest value of a metric over a sibling's buckets, and its keys.
    pub fn min_bucket(path: impl Into<String>) -> AggregationContainer {
        Agg::of(MinBucketAggregation::new().buckets_path(BucketsPath::Single(path.into())))
    }

    /// In each bucket of a histogram, the running total of a metric.
    pub fn cumulative_sum(path: impl Into<String>) -> AggregationContainer {
        Agg::of(CumulativeSumAggregation::new().buckets_path(BucketsPath::Single(path.into())))
    }

    /// In each bucket of a histogram, how much a metric changed from the
    /// previous bucket.
    pub fn derivative(path: impl Into<String>) -> AggregationContainer {
        Agg::of(DerivativeAggregation::new().buckets_path(BucketsPath::Single(path.into())))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn requests() {
        let body = AggsBody::from(
            Aggs::new()
                .add(
                    "by_tag",
                    Agg::terms("tag", 5).sub("avg_price", Agg::avg("price")),
                )
                .add(
                    "price_ranges",
                    Agg::range("price", [(None, Some(10.0)), (Some(10.0), None)]),
                )
                .add("avg_of_tags", Agg::avg_bucket("by_tag>avg_price")),
        );
        assert_eq!(
            body.0,
            json!({
                "by_tag": {"terms": {"field": "tag", "size": 5}, "aggregations": {"avg_price": {"avg": {"field": "price"}}}},
                "price_ranges": {"range": {"field": "price", "ranges": [{"to": 10.0}, {"from": 10.0}]}},
                "avg_of_tags": {"avg_bucket": {"buckets_path": "by_tag>avg_price"}}
            })
        );
    }
}
