//! Document APIs: index, get, bulk and multi-get, with a typed document.

use oxsearch_core::Refresh;
use oxsearch_docs::{bulk, get, index, mget};
use oxsearch_integration::fixtures::{article_mapping, articles};
use oxsearch_integration::{Article, TestIndex, client};

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn index_and_get_a_typed_document() {
    let client = client();
    let test = TestIndex::create(&client, "docs", &article_mapping()).await;
    let (id, article) = articles().remove(0);

    let indexed = client
        .send(
            &index::Request::new(test.name())
                .id(id)
                .refresh(Refresh::True)
                .body(article.clone()),
        )
        .await
        .unwrap();
    assert_eq!(indexed.id.as_deref(), Some(id));

    let found: get::Response<Article> = client
        .send_as(&get::Request::new(id, test.name()))
        .await
        .unwrap();
    assert_eq!(found.found, Some(true));
    let source = found.source.clone().expect("the document's source");
    assert_eq!(
        (source.title.as_str(), source.tag.as_str()),
        (article.title.as_str(), article.tag.as_str())
    );
    // Elasticsearch 9 leaves vector fields out of returned sources.
    assert!(
        source.embedding.is_empty() && source.terms.is_empty(),
        "{source:?}"
    );
    test.delete(&client).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn bulk_then_multi_get() {
    let client = client();
    let test = TestIndex::create(&client, "bulk", &article_mapping()).await;
    let mut lines: Vec<bulk::RequestBody<Article>> = Vec::new();
    for (id, article) in articles() {
        lines.push(bulk::RequestBody::OperationContainer(
            bulk::OperationContainer::Index(bulk::IndexOperation::new().index(test.name()).id(id)),
        ));
        lines.push(bulk::RequestBody::TDocument(article));
    }
    let response = client
        .send(&bulk::Request::new().refresh(Refresh::True).body(lines))
        .await
        .unwrap();
    assert_eq!(response.errors, Some(false), "{response:?}");
    assert_eq!(response.items.len(), 3);

    let ids: Vec<String> = ["1", "3"].map(str::to_owned).to_vec();
    let found: mget::Response<Article> = client
        .send_as(
            &mget::Request::new()
                .index(test.name())
                .body(mget::RequestBody::new().ids(ids)),
        )
        .await
        .unwrap();
    let titles: Vec<String> = found
        .docs
        .iter()
        .filter_map(|doc| match doc {
            mget::ResponseItem::Result(result) => result.source.as_ref().map(|a| a.title.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(titles, ["Tuning HNSW graphs", "Quantizing vectors"]);
    test.delete(&client).await;
}
