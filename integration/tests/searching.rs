//! The search endpoints beyond `search` itself: counting, scrolling,
//! explaining, field capabilities, templates, ranking evaluation, shards and
//! terms. Each is built from the generated types and sent to Elasticsearch.

use oxsearch::search::{
    clear_scroll, count, explain, field_caps, msearch, msearch_template, rank_eval,
    render_search_template, scroll, search, search_shards, search_template, terms_enum,
};
use oxsearch::shapes::IndexMap;
use oxsearch::{Query, Refresh, ping};
use oxsearch_docs::index;
use oxsearch_integration::fixtures::{article_mapping, articles};
use oxsearch_integration::{Article, TestClient, TestIndex, client};
use serde_json::{Value, json};

/// An index holding the three articles.
async fn articles_index(client: &TestClient, prefix: &str) -> TestIndex {
    let test = TestIndex::create(client, prefix, &article_mapping()).await;
    for (id, article) in articles() {
        client
            .send(
                &index::Request::new(test.name())
                    .id(id)
                    .refresh(Refresh::True)
                    .body(article),
            )
            .await
            .unwrap();
    }
    test
}

fn params(pairs: &[(&str, Value)]) -> IndexMap<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect()
}

const TEMPLATE: &str = r#"{"query": {"term": {"tag": "{{tag}}"}}}"#;

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn counting_scrolling_and_explaining() {
    let client = client();
    let test = articles_index(&client, "searching").await;

    client.send(&ping::Request::new()).await.unwrap();

    let counted = client
        .send(
            &count::Request::new()
                .index(test.name())
                .body(count::RequestBody::new().query(Query::term("tag", "vector"))),
        )
        .await
        .unwrap();
    assert_eq!(counted.count, Some(2));

    // Scroll through one hit at a time, then clear the scroll.
    let first: search::Response<Article> = client
        .send_as(
            &search::Request::new()
                .index(test.name())
                .scroll("1m")
                .body(search::RequestBody::new().size(1)),
        )
        .await
        .unwrap();
    let mut scroll_id = first.scroll_id.clone().expect("a scroll id");
    let mut seen = first.results().len();
    loop {
        let page: scroll::Response<Article> = client
            .send_as(
                &scroll::Request::new()
                    .body(scroll::RequestBody::new(scroll_id.as_str()).scroll("1m")),
            )
            .await
            .unwrap();
        if page.results().is_empty() {
            break;
        }
        seen += page.results().len();
        scroll_id = page.scroll_id.clone().unwrap_or(scroll_id);
    }
    assert_eq!(seen, 3);
    let cleared = client
        .send(
            &clear_scroll::Request::new()
                .body(clear_scroll::RequestBody::new().scroll_id(scroll_id)),
        )
        .await
        .unwrap();
    assert_eq!(cleared.succeeded, Some(true));

    let explained = client
        .send(
            &explain::Request::new("1", test.name())
                .body(explain::RequestBody::new().query(Query::r#match("title", "hnsw"))),
        )
        .await
        .unwrap();
    assert_eq!(explained.matched, Some(true));

    let caps = client
        .send(
            &field_caps::Request::new()
                .index(test.name())
                .fields("title,tag"),
        )
        .await
        .unwrap();
    assert!(caps.fields.contains_key("tag"));

    let shards = client
        .send(&search_shards::Request::new().index(test.name()))
        .await
        .unwrap();
    assert!(!shards.shards.is_empty());

    let terms = client
        .send(
            &terms_enum::Request::new(test.name())
                .body(terms_enum::RequestBody::new("tag").string("v")),
        )
        .await
        .unwrap();
    assert_eq!(terms.terms, ["vector"]);
    test.delete(&client).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn templates_and_ranking_evaluation() {
    let client = client();
    let test = articles_index(&client, "templates").await;

    let rendered = client
        .send(
            &render_search_template::Request::new().body(
                render_search_template::RequestBody::new()
                    .source(TEMPLATE)
                    .params(params(&[("tag", json!("ingest"))])),
            ),
        )
        .await
        .unwrap();
    assert_eq!(rendered.template_output["query"]["term"]["tag"], "ingest");

    let found: search_template::Response<Article> = client
        .send_as(
            &search_template::Request::new().index(test.name()).body(
                search_template::RequestBody::new()
                    .source(TEMPLATE)
                    .params(params(&[("tag", json!("vector"))])),
            ),
        )
        .await
        .unwrap();
    assert_eq!(found.results().len(), 2);

    let lines = vec![
        msearch_template::RequestItem::Header(msearch::MultisearchHeader::new().index(test.name())),
        msearch_template::RequestItem::Body(
            msearch_template::TemplateConfig::new()
                .source(TEMPLATE)
                .params(params(&[("tag", json!("ingest"))])),
        ),
    ];
    let many = client
        .send(&msearch_template::Request::new().body(lines))
        .await
        .unwrap();
    assert_eq!(many.responses.len(), 1);

    let evaluated = client
        .send(
            &rank_eval::Request::new().index(test.name()).body(
                rank_eval::RequestBody::new(vec![
                    rank_eval::RankEvalRequestItem::new(
                        "hnsw",
                        vec![rank_eval::DocumentRating::new("1", test.name(), 1)],
                    )
                    .request(rank_eval::RankEvalQuery::new(Query::r#match(
                        "title", "hnsw",
                    ))),
                ])
                .metric(rank_eval::RankEvalMetric::new().precision(
                    rank_eval::RankEvalMetricPrecision::new().relevant_rating_threshold(1),
                )),
            ),
        )
        .await
        .unwrap();
    assert!(evaluated.metric_score.unwrap() > 0.0);
    test.delete(&client).await;
}
