//! ES|QL against Elasticsearch: queries built with `Esql`, rows read into
//! structs by column name, FORK and FUSE, async queries, and queries on a
//! typed document index.

use oxsearch::Error;
use oxsearch::document::{Document, Indexes};
use oxsearch::esql::{Esql, EsqlResult, Expr, Fuse, async_query, async_query_delete, col};
use oxsearch::mapping::{
    DateProperty, IntegerNumberProperty, KeywordProperty, ObjectProperty, Property, TextProperty,
    TypeMapping,
};
use oxsearch::shapes::IndexMap;
use oxsearch::{Duration, Refresh};
use oxsearch_docs::index;
use oxsearch_integration::{License, TestClient, TestIndex, client};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Deserialize, PartialEq)]
struct Author {
    name: String,
}

/// A book as a query reads it, the author's name from `author.name`.
#[derive(Debug, Deserialize, PartialEq)]
struct Book {
    title: String,
    year: i32,
    tags: Vec<String>,
    author: Author,
}

/// Four books by two authors.
async fn books(client: &TestClient) -> TestIndex {
    let mut title_fields = IndexMap::new();
    title_fields.insert(
        "keyword".to_owned(),
        Property::Keyword(KeywordProperty::new()),
    );
    let mut author = IndexMap::new();
    author.insert("name".to_owned(), Property::Keyword(KeywordProperty::new()));
    let mut properties = IndexMap::new();
    properties.insert(
        "title".to_owned(),
        Property::Text(TextProperty::new().fields(title_fields)),
    );
    properties.insert(
        "author".to_owned(),
        Property::Object(ObjectProperty::new().properties(author)),
    );
    properties.insert(
        "year".to_owned(),
        Property::IntegerNumber(IntegerNumberProperty::new()),
    );
    properties.insert("tags".to_owned(), Property::Keyword(KeywordProperty::new()));
    properties.insert("published".to_owned(), Property::Date(DateProperty::new()));
    let test = TestIndex::create(client, "esql", &TypeMapping::new().properties(properties)).await;
    let rows = [
        (
            "HNSW graphs",
            "Ada",
            2024,
            json!(["vectors", "search"]),
            "2024-03-01",
        ),
        (
            "Bulk ingest at speed",
            "Grace",
            2021,
            json!("ingest"),
            "2021-06-01",
        ),
        (
            "Vector quantization",
            "Ada",
            2025,
            json!(["vectors"]),
            "2025-01-15",
        ),
        (
            "Search relevance",
            "Grace",
            2019,
            json!(["search"]),
            "2019-09-30",
        ),
    ];
    for (n, (title, author, year, tags, published)) in rows.into_iter().enumerate() {
        client
            .send(
                &index::Request::new(test.name())
                    .id(n.to_string())
                    .refresh(Refresh::True)
                    .body(json!({
                        "title": title,
                        "author": {"name": author},
                        "year": year,
                        "tags": tags,
                        "published": published
                    })),
            )
            .await
            .unwrap();
    }
    test
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn rows_into_structs() {
    let client = client();
    let test = books(&client).await;

    // Rows by column name: the author nested from `author.name`, and a
    // single tag read as a list of one.
    let recent: Vec<Book> = Esql::from_index(test.name())
        .where_(col("published").gte("2021-01-01"))
        .keep(["title", "year", "tags", "author.name"])
        .sort([col("year").asc()])
        .fetch(&client)
        .await
        .unwrap();
    let titles: Vec<&str> = recent.iter().map(|b| b.title.as_str()).collect();
    assert_eq!(
        titles,
        ["Bulk ingest at speed", "HNSW graphs", "Vector quantization"]
    );
    assert_eq!(recent[0].tags, ["ingest"]);
    // Multi-values come back sorted.
    assert_eq!(recent[1].tags, ["search", "vectors"]);
    assert_eq!(recent[1].author.name, "Ada");

    // A string value is a parameter, never part of the query.
    let tricky = Esql::from_index(test.name())
        .where_(col("author.name").eq("Ada\" OR true OR \""))
        .keep(["title"]);
    let none: Vec<serde_json::Value> = tricky.fetch(&client).await.unwrap();
    assert!(none.is_empty(), "{none:?}");

    // Statistics by group, read into their own struct.
    #[derive(Debug, Deserialize, PartialEq)]
    struct ByAuthor {
        books: i64,
        newest: i32,
        author: Author,
    }
    let by_author: Vec<ByAuthor> = Esql::from_index(test.name())
        .stats(
            [("books", "COUNT(*)"), ("newest", "MAX(year)")],
            ["author.name"],
        )
        .sort([col("author.name").asc()])
        .fetch(&client)
        .await
        .unwrap();
    assert_eq!(
        by_author,
        [
            ByAuthor {
                books: 2,
                newest: 2025,
                author: Author { name: "Ada".into() }
            },
            ByAuthor {
                books: 2,
                newest: 2021,
                author: Author {
                    name: "Grace".into()
                }
            },
        ]
    );

    // A full-text match, scored, with expressions the builder has no
    // method for written as text.
    #[derive(Debug, Deserialize)]
    struct Scored {
        title: String,
        #[serde(rename = "_score")]
        score: f64,
        shout: String,
    }
    let scored: Vec<Scored> = Esql::from_index(test.name())
        .metadata(["_score"])
        .where_(col("title").matches("vector graphs"))
        .eval("shout", Expr::new("CONCAT(TO_UPPER(title), ?)", ["!"]))
        .sort([col("_score").desc()])
        .keep(["title", "_score", "shout"])
        .fetch(&client)
        .await
        .unwrap();
    assert_eq!(scored.len(), 2, "{scored:?}");
    assert!(scored.iter().all(|s| s.score > 0.0));
    assert_eq!(
        scored[0].shout,
        format!("{}!", scored[0].title.to_uppercase())
    );

    // The whole result: columns with their types.
    let result: EsqlResult = client
        .send(
            &Esql::from_index(test.name())
                .keep(["title", "year"])
                .limit(1)
                .request(),
        )
        .await
        .unwrap()
        .0;
    let columns: Vec<(Option<&str>, Option<&str>)> = result
        .columns
        .iter()
        .map(|c| (c.name.as_deref(), c.r#type.as_deref()))
        .collect();
    assert_eq!(
        columns,
        [
            (Some("title"), Some("text")),
            (Some("year"), Some("integer"))
        ]
    );

    // A row that doesn't fit says which row and column.
    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct Mistyped {
        title: i64,
    }
    let wrong = Esql::from_index(test.name())
        .keep(["title"])
        .fetch::<Mistyped>(&client)
        .await;
    let Err(Error::Row(e)) = wrong else {
        panic!("{wrong:?}");
    };
    assert_eq!((e.row, e.column.as_deref()), (0, Some("title")));

    // A row without an index.
    #[derive(Debug, Deserialize, PartialEq)]
    struct Sum {
        a: i64,
        b: i64,
    }
    let sum: Vec<Sum> = Esql::row([("a", 40)])
        .eval("b", Expr::new("a + ?", [2]))
        .fetch(&client)
        .await
        .unwrap();
    assert_eq!(sum, [Sum { a: 40, b: 42 }]);
    test.delete(&client).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn fork_and_fuse() {
    let client = client();
    let test = books(&client).await;
    let branches = || {
        [
            Esql::branch()
                .where_(col("title").matches("vector search"))
                .sort([col("_score").desc()])
                .limit(3),
            Esql::branch()
                .where_(col("year").gte(2024))
                .sort([col("year").desc()])
                .limit(3),
        ]
    };

    // FORK alone: each branch's rows, named by `_fork`.
    #[derive(Debug, Deserialize)]
    struct Forked {
        title: String,
        #[serde(rename = "_fork")]
        fork: String,
    }
    let forked: Vec<Forked> = Esql::from_index(test.name())
        .metadata(["_score"])
        .fork(branches())
        .keep(["title", "_fork"])
        .fetch(&client)
        .await
        .unwrap();
    let in_second: Vec<&str> = forked
        .iter()
        .filter(|r| r.fork == "fork2")
        .map(|r| r.title.as_str())
        .collect();
    assert_eq!(in_second, ["Vector quantization", "HNSW graphs"]);

    // FUSE merges the branches' rows by reciprocal rank fusion, on a
    // license that includes it.
    #[derive(Debug, Deserialize)]
    struct Fused {
        title: String,
        #[serde(rename = "_score")]
        score: f64,
    }
    let fused = Esql::from_index(test.name())
        .metadata(["_id", "_index", "_score"])
        .fork(branches())
        .fuse()
        .sort([col("_score").desc()])
        .keep(["title", "_score"])
        .fetch::<Fused>(&client)
        .await;
    match License::from_env() {
        License::Basic => {
            let Err(Error::License(e)) = fused else {
                panic!("{fused:?}");
            };
            assert_eq!(e.feature, "FUSE");
        }
        License::Trial => {
            let fused = fused.unwrap();
            // In both branches, so first.
            assert_eq!(fused[0].title, "Vector quantization", "{fused:?}");
            let titles: std::collections::BTreeSet<&str> =
                fused.iter().map(|f| f.title.as_str()).collect();
            assert_eq!(titles.len(), fused.len(), "merged: {fused:?}");
            assert!(fused.windows(2).all(|w| w[0].score >= w[1].score));
            let linear: Vec<Fused> = Esql::from_index(test.name())
                .metadata(["_id", "_index", "_score"])
                .fork(branches())
                .fuse_with(Fuse::linear().with(json!({"normalizer": "minmax"})))
                .sort([col("_score").desc()])
                .keep(["title", "_score"])
                .fetch(&client)
                .await
                .unwrap();
            assert_eq!(linear.len(), fused.len(), "{linear:?}");
        }
    }
    test.delete(&client).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn async_queries() {
    let client = client();
    let test = books(&client).await;
    let query = Esql::from_index(test.name())
        // A comparison with a multi-valued field is null, so on the author.
        .where_(col("author.name").eq("Ada"))
        .keep(["title"])
        .sort([col("title").asc()]);
    let started = client
        .send(
            &async_query::Request::new().body(
                query
                    .async_body()
                    .wait_for_completion_timeout(Duration::String("10s".into()))
                    .keep_on_completion(true),
            ),
        )
        .await
        .unwrap();
    assert_eq!(started.is_running, Some(false));
    #[derive(Debug, Deserialize)]
    struct Title {
        title: String,
    }
    let titles: Vec<Title> = started.rows().unwrap();
    let titles: Vec<&str> = titles.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(titles, ["HNSW graphs", "Vector quantization"]);

    // Kept, it reads again by its id, until it's deleted.
    let id = started.id.clone().expect("a kept query's id");
    let again = client
        .send(&oxsearch::esql::async_query_get::Request::new(id.as_str()))
        .await
        .unwrap();
    assert_eq!(again.rows::<Title>().unwrap().len(), 2);
    client
        .send(&async_query_delete::Request::new(id.as_str()))
        .await
        .unwrap();
    test.delete(&client).await;
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn arrow_output() {
    use oxsearch::esql::arrow_array::{Array, Int32Array, ListArray, StringArray};

    let client = client();
    let test = books(&client).await;
    let batches = Esql::from_index(test.name())
        .keep(["title", "year", "author.name", "tags"])
        .sort([col("year").asc()])
        .fetch_arrow(&client)
        .await
        .unwrap();
    assert_eq!(batches.iter().map(|b| b.num_rows()).sum::<usize>(), 4);
    let batch = &batches[0];
    let schema = batch.schema();
    // Each ES|QL column is a field, a multi-valued one a list.
    let fields: Vec<(&str, String)> = schema
        .fields()
        .iter()
        .map(|f| (f.name().as_str(), f.data_type().to_string()))
        .collect();
    assert_eq!(
        fields[..3],
        [
            ("title", "Utf8".to_owned()),
            ("year", "Int32".to_owned()),
            ("author.name", "Utf8".to_owned()),
        ]
    );
    assert!(fields[3].1.starts_with("List("), "{fields:?}");
    let column = |name: &str| batch.column(schema.index_of(name).unwrap()).clone();
    let years = column("year");
    let years = years.as_any().downcast_ref::<Int32Array>().unwrap();
    assert_eq!(years.values(), &[2019, 2021, 2024, 2025]);
    let titles = column("title");
    let titles = titles.as_any().downcast_ref::<StringArray>().unwrap();
    assert_eq!(titles.value(0), "Search relevance");
    let tags = column("tags");
    let tags = tags.as_any().downcast_ref::<ListArray>().unwrap();
    let hnsw = tags.value(2);
    let hnsw = hnsw.as_any().downcast_ref::<StringArray>().unwrap();
    assert_eq!(
        hnsw.iter().flatten().collect::<Vec<_>>(),
        ["search", "vectors"]
    );
    test.delete(&client).await;
}

#[derive(Document, Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Note {
    #[id]
    id: String,
    title: String,
    #[keyword]
    tags: Vec<String>,
    views: i32,
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn on_a_document_index() {
    let client = client();
    let test = TestIndex::named("esql-notes");
    let notes = client
        .index_named::<Note>(test.name())
        .refresh(Refresh::WaitFor);
    let _ = notes.ensure_mapping().await.unwrap();
    let written = [
        ("1", "Tuning HNSW graphs", vec!["vector", "tuning"], 40),
        ("2", "Bulk loading at speed", vec!["ingest"], 10),
        ("3", "Quantizing vectors", vec!["vector"], 25),
    ];
    for (id, title, tags, views) in written {
        notes
            .put(&Note {
                id: id.into(),
                title: title.into(),
                tags: tags.into_iter().map(str::to_owned).collect(),
                views,
            })
            .await
            .unwrap();
    }

    // Rows read back as the documents, the index filled in.
    let popular: Vec<Note> = notes
        .esql()
        .where_(col("views").gte(20))
        .sort([col("views").desc()])
        .fetch(notes.client())
        .await
        .unwrap();
    assert_eq!(
        popular,
        [
            Note {
                id: "1".into(),
                title: "Tuning HNSW graphs".into(),
                tags: vec!["tuning".into(), "vector".into()],
                views: 40
            },
            Note {
                id: "3".into(),
                title: "Quantizing vectors".into(),
                tags: vec!["vector".into()],
                views: 25
            },
        ]
    );
    test.delete(&client).await;
}
