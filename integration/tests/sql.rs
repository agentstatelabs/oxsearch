//! SQL against Elasticsearch: columns and rows, a cursor through a result
//! a page at a time, the search a query translates into, and async queries.

use oxsearch::sql::query::SqlFormat;
use oxsearch::sql::{clear_cursor, delete_async, get_async, get_async_status, query, translate};
use oxsearch::{Duration, Refresh};
use oxsearch_docs::index;
use oxsearch_integration::{TestIndex, client};
use serde_json::json;

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn queries_and_cursors() {
    let client = client();
    let test = TestIndex::named("sql");
    let books = [("HNSW graphs", 30), ("Bulk ingest", 10), ("Relevance", 20)];
    for (n, (title, price)) in books.into_iter().enumerate() {
        client
            .send(
                &index::Request::new(test.name())
                    .id(n.to_string())
                    .refresh(Refresh::True)
                    .body(json!({"title": title, "price": price})),
            )
            .await
            .unwrap();
    }
    let sql = format!(
        r#"SELECT title, price FROM "{}" ORDER BY price"#,
        test.name()
    );

    // Two rows a page, the rest through the cursor.
    let first = client
        .send(
            &query::Request::new()
                .format(SqlFormat::Json)
                .body(query::RequestBody::new().query(sql.as_str()).fetch_size(2)),
        )
        .await
        .unwrap();
    let columns: Vec<Option<&str>> = first
        .columns
        .as_ref()
        .unwrap()
        .iter()
        .map(|c| c.name.as_deref())
        .collect();
    assert_eq!(columns, [Some("title"), Some("price")]);
    assert_eq!(
        first.rows,
        [
            vec![json!("Bulk ingest"), json!(10)],
            vec![json!("Relevance"), json!(20)]
        ]
    );
    let cursor = first.cursor.clone().expect("a cursor to the rest");
    let rest = client
        .send(
            &query::Request::new()
                .format(SqlFormat::Json)
                .body(query::RequestBody::new().cursor(cursor.as_str())),
        )
        .await
        .unwrap();
    assert_eq!(rest.rows, [vec![json!("HNSW graphs"), json!(30)]]);
    assert_eq!(rest.cursor, None, "the last page has no cursor");

    // A cursor closed before its rows are read.
    let paged = client
        .send(
            &query::Request::new()
                .format(SqlFormat::Json)
                .body(query::RequestBody::new().query(sql.as_str()).fetch_size(1)),
        )
        .await
        .unwrap();
    let closed = client
        .send(
            &clear_cursor::Request::new().body(clear_cursor::RequestBody::new(
                paged.cursor.expect("a cursor"),
            )),
        )
        .await
        .unwrap();
    assert_eq!(closed.succeeded, Some(true));

    // The search request a query becomes.
    let translated = client
        .send(&translate::Request::new().body(translate::RequestBody::new(sql.as_str())))
        .await
        .unwrap();
    assert!(translated.sort.is_some(), "{translated:?}");

    // Kept after it completes, to be read again by its id.
    let kept = client
        .send(
            &query::Request::new().format(SqlFormat::Json).body(
                query::RequestBody::new()
                    .query(sql.as_str())
                    .keep_on_completion(true)
                    .wait_for_completion_timeout(Duration::String("10s".into())),
            ),
        )
        .await
        .unwrap();
    let id = kept.id.clone().expect("a kept query's id");
    let again = client
        .send(&get_async::Request::new(id.as_str()).format("json"))
        .await
        .unwrap();
    assert_eq!(again.rows.len(), 3);
    let status = client
        .send(&get_async_status::Request::new(id.as_str()))
        .await
        .unwrap();
    assert_eq!(status.is_running, Some(false));
    client
        .send(&delete_async::Request::new(id.as_str()))
        .await
        .unwrap();
    test.delete(&client).await;
}
