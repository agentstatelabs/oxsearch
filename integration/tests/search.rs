//! Search with every retriever and the AI-search queries, built from the
//! generated types. Retrievers the free license doesn't include must fail
//! there with a typed license error, and work on a trial.

use oxsearch_core::query_dsl::{
    IntervalsMatch, IntervalsQuery, IntervalsQueryVariant, MatchQuery, MatchQueryQuery,
    QueryContainer, SparseVectorQuery, SparseVectorQueryVariant,
};
use oxsearch_core::search::{Rescore, RescoreQuery, RescoreVariant};
use oxsearch_core::shapes::{IndexMap, SingleKey};
use oxsearch_core::{
    DiversifyRetriever, DiversifyRetrieverTypes, Error, InnerRetriever, KnnQuery, KnnRetriever,
    LinearRetriever, Method, PinnedRetriever, RRFRetriever, RRFRetrieverEntry, Refresh,
    RescorerRetriever, RetrieverContainer, RuleRetriever, StandardRetriever,
    TextSimilarityReranker,
};
use oxsearch_docs::index;
use oxsearch_integration::fixtures::{article_mapping, articles};
use oxsearch_integration::{Article, License, TestClient, TestIndex, client, understood};
use oxsearch_search::search;
use serde_json::json;

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

async fn search(
    client: &TestClient,
    test: &TestIndex,
    body: search::RequestBody,
) -> Result<Vec<String>, Error> {
    let request = search::Request::new().index(test.name()).body(body);
    let response: search::Response<Article> = client.send_as(&request).await?;
    let hits = response.hits.as_ref().expect("a search response has hits");
    Ok(hits.hits.iter().filter_map(|hit| hit.id.clone()).collect())
}

fn retrieve(retriever: RetrieverContainer) -> search::RequestBody {
    search::RequestBody::new().retriever(retriever)
}

fn match_title(text: &str) -> QueryContainer {
    QueryContainer::Match(SingleKey::new(
        "title",
        MatchQuery::new(MatchQueryQuery::String(text.to_owned())),
    ))
}

fn standard(query: QueryContainer) -> RetrieverContainer {
    RetrieverContainer::Standard(StandardRetriever::new().query(query))
}

fn knn(vector: [f32; 3]) -> RetrieverContainer {
    RetrieverContainer::Knn(
        KnnRetriever::new("embedding", 2)
            .query_vector(vector.to_vec())
            .num_candidates(10),
    )
}

/// What a retriever the Basic license doesn't include does: fails with a
/// license error there, and works on a trial.
fn paid(result: Result<Vec<String>, Error>, expected_first: &str) {
    match (License::from_env(), result) {
        (License::Basic, Err(Error::License(e))) => assert!(!e.feature.is_empty()),
        (License::Basic, other) => panic!("expected a license error on Basic, got {other:?}"),
        (License::Trial, Ok(ids)) => assert_eq!(
            ids.first().map(String::as_str),
            Some(expected_first),
            "{ids:?}"
        ),
        (License::Trial, Err(e)) => panic!("{e}"),
    }
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn retrievers_on_every_license() {
    let client = client();
    let test = articles_index(&client, "free-retrievers").await;

    let ids = search(&client, &test, retrieve(standard(match_title("hnsw"))))
        .await
        .unwrap();
    assert_eq!(ids, ["1"]);

    let ids = search(&client, &test, retrieve(knn([0.9, 0.1, 0.0])))
        .await
        .unwrap();
    assert_eq!(ids.first().map(String::as_str), Some("1"));

    let pinned = PinnedRetriever::new(standard(match_title("vectors"))).ids(vec!["2".to_owned()]);
    let ids = search(
        &client,
        &test,
        retrieve(RetrieverContainer::Pinned(Box::new(pinned))),
    )
    .await
    .unwrap();
    assert_eq!(ids.first().map(String::as_str), Some("2"), "{ids:?}");

    let rescore = Rescore::new()
        .window_size(10)
        .variant(RescoreVariant::Query(RescoreQuery::new(match_title(
            "quantizing",
        ))));
    let rescorer = RescorerRetriever::new(knn([0.9, 0.1, 0.0]), rescore);
    let ids = search(
        &client,
        &test,
        retrieve(RetrieverContainer::Rescorer(Box::new(rescorer))),
    )
    .await
    .unwrap();
    assert_eq!(ids.first().map(String::as_str), Some("3"), "{ids:?}");

    let diversify = DiversifyRetriever::new(
        DiversifyRetrieverTypes::Mmr,
        "embedding",
        knn([0.9, 0.1, 0.0]),
    )
    .query_vector(vec![0.9, 0.1, 0.0])
    .lambda(0.5)
    .size(2);
    let result = search(
        &client,
        &test,
        retrieve(RetrieverContainer::Diversify(Box::new(diversify))),
    )
    .await;
    match result {
        Ok(ids) => assert_eq!(ids.len(), 2, "{ids:?}"),
        Err(e) => assert!(understood(&e), "{e}"),
    }
    test.delete(&client).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn fusion_retrievers_need_a_paid_license() {
    let client = client();
    let test = articles_index(&client, "fusion").await;

    let rrf = RRFRetriever::new(vec![
        RRFRetrieverEntry::Retriever(standard(match_title("hnsw"))),
        RRFRetrieverEntry::Retriever(knn([1.0, 0.0, 0.0])),
    ]);
    paid(
        search(&client, &test, retrieve(RetrieverContainer::Rrf(rrf))).await,
        "1",
    );

    let linear = LinearRetriever::new().retrievers(vec![
        InnerRetriever::new(standard(match_title("hnsw"))).weight(2.0),
        InnerRetriever::new(knn([1.0, 0.0, 0.0])).weight(1.0),
    ]);
    paid(
        search(&client, &test, retrieve(RetrieverContainer::Linear(linear))).await,
        "1",
    );
    test.delete(&client).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn rule_retriever_pins_by_ruleset() {
    let client = client();
    let test = articles_index(&client, "rules").await;
    // The query rules API isn't in the first slice's crates: the raw call.
    let ruleset = format!("{}-rules", test.name());
    let created = client
        .send_raw(
            Method::Put,
            &format!("/_query_rules/{ruleset}"),
            Some(&json!({"rules": [{
                "rule_id": "pin-bulk",
                "type": "pinned",
                "criteria": [{"type": "exact", "metadata": "topic", "values": ["ingest"]}],
                "actions": {"ids": ["2"]}
            }]})),
        )
        .await;
    let rule = RuleRetriever::new(
        ruleset.clone(),
        json!({"topic": "ingest"}),
        standard(match_title("hnsw")),
    );
    let result = search(
        &client,
        &test,
        retrieve(RetrieverContainer::Rule(Box::new(rule))),
    )
    .await;
    match (License::from_env(), created) {
        (License::Trial, Ok(_)) => {
            assert_eq!(result.unwrap().first().map(String::as_str), Some("2"))
        }
        (License::Trial, Err(e)) => panic!("creating the ruleset: {e}"),
        // On Basic, the query rules API or the retriever refuses with a
        // license error.
        (License::Basic, Err(e)) => assert!(matches!(e, Error::License(_)), "{e}"),
        (License::Basic, Ok(_)) => assert!(matches!(result, Err(Error::License(_))), "{result:?}"),
    }
    let _ = client
        .send_raw(Method::Delete, &format!("/_query_rules/{ruleset}"), None)
        .await;
    test.delete(&client).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn text_similarity_reranker_is_understood() {
    // Reranking needs a deployed model, which the fixture doesn't download:
    // the request must be one Elasticsearch reads, even if it can't run it.
    let client = client();
    let test = articles_index(&client, "rerank").await;
    let reranker =
        TextSimilarityReranker::new(standard(match_title("hnsw")), "tuning graphs", "title")
            .inference_id(".rerank-v1-elasticsearch");
    let result = search(
        &client,
        &test,
        retrieve(RetrieverContainer::TextSimilarityReranker(Box::new(
            reranker,
        ))),
    )
    .await;
    if let Err(e) = result {
        eprintln!("the reranker didn't run: {e}");
        assert!(understood(&e), "{e}");
    }
    test.delete(&client).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn ai_search_queries() {
    let client = client();
    let test = articles_index(&client, "queries").await;

    let knn_query = KnnQuery::new("embedding")
        .query_vector(vec![0.0, 1.0, 0.0])
        .k(1)
        .num_candidates(10);
    let ids = search(
        &client,
        &test,
        search::RequestBody::new().query(QueryContainer::Knn(Box::new(knn_query))),
    )
    .await
    .unwrap();
    assert_eq!(ids, ["2"]);

    let mut weights = IndexMap::new();
    weights.insert("quantization".to_owned(), 1.0);
    let sparse =
        SparseVectorQuery::new("terms").variant(SparseVectorQueryVariant::QueryVector(weights));
    let ids = search(
        &client,
        &test,
        search::RequestBody::new().query(QueryContainer::SparseVector(sparse)),
    )
    .await
    .unwrap();
    assert_eq!(ids, ["3"]);

    let intervals = IntervalsQuery::new().variant(IntervalsQueryVariant::Match(
        IntervalsMatch::new("bulk loading"),
    ));
    let ids = search(
        &client,
        &test,
        search::RequestBody::new().query(QueryContainer::Intervals(SingleKey::new(
            "title", intervals,
        ))),
    )
    .await
    .unwrap();
    assert_eq!(ids, ["2"]);
    test.delete(&client).await;
}
