//! What `#[derive(Document)]` makes of a struct: the mapping, the id and
//! routing, and the field handles.

use oxsearch::document::{Document, kind};
use oxsearch::{Query, Retriever};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Document, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[index("articles")]
struct Article {
    #[id]
    id: u64,
    title: String,
    #[keyword]
    tags: Vec<String>,
    #[semantic_text(inference_id = "my-elser")]
    body: String,
    #[dense_vector(dims = 3, similarity = "cosine")]
    embedding: Vec<f32>,
    #[sparse_vector]
    terms: std::collections::HashMap<String, f32>,
    published_at: Option<std::time::SystemTime>,
    #[routing]
    #[serde(rename = "owner")]
    owner_id: Option<String>,
    views: Option<u32>,
    draft: bool,
    author: Author,
    #[serde(skip)]
    cached: Option<String>,
}

#[derive(Document, Serialize, Deserialize)]
struct Author {
    #[keyword]
    name: String,
    #[keyword]
    r#type: String,
}

#[test]
fn the_mapping_comes_from_the_struct() {
    assert_eq!(Article::INDEX, Some("articles"));
    assert_eq!(Author::INDEX, None);
    let mapping: Value = serde_json::to_value(Article::mapping()).unwrap();
    assert_eq!(
        mapping,
        json!({"properties": {
            "id": {"type": "unsigned_long"},
            "title": {"type": "text"},
            "tags": {"type": "keyword"},
            "body": {"type": "semantic_text", "inference_id": "my-elser"},
            "embedding": {"type": "dense_vector", "dims": 3, "similarity": "cosine"},
            "terms": {"type": "sparse_vector"},
            "publishedAt": {"type": "date"},
            "owner": {"type": "text"},
            "views": {"type": "long"},
            "draft": {"type": "boolean"},
            "author": {"type": "object", "properties": {
                "name": {"type": "keyword"},
                "type": {"type": "keyword"}
            }}
        }})
    );
}

#[test]
fn ids_and_routing() {
    let article = Article {
        id: 7,
        title: "Tuning HNSW".into(),
        tags: vec!["vector".into()],
        body: String::new(),
        embedding: vec![],
        terms: Default::default(),
        published_at: None,
        owner_id: Some("ada".into()),
        views: None,
        draft: false,
        author: Author {
            name: "Ada".into(),
            r#type: "person".into(),
        },
        cached: None,
    };
    assert_eq!(article.id(), Some("7".to_owned()));
    assert_eq!(article.routing(), Some("ada".to_owned()));
    assert_eq!(article.author.id(), None);
    assert!(article.cached.is_none());
}

#[test]
fn field_handles_offer_their_kinds_queries() {
    let _: oxsearch::document::Field<kind::Keyword> = Article::TAGS;
    let _: oxsearch::document::Field<kind::Date> = Article::PUBLISHED_AT;
    let _: oxsearch::document::Field<kind::Number> = Article::VIEWS;
    let json = |q| serde_json::to_value(q).unwrap();
    assert_eq!(
        json(Article::TAGS.term("vector")),
        json!({"term": {"tags": "vector"}})
    );
    assert_eq!(
        json(Article::PUBLISHED_AT.range("now-7d"..)),
        json!({"range": {"publishedAt": {"gte": "now-7d"}}})
    );
    assert_eq!(
        json(Article::AUTHOR.field(Author::NAME).term("Ada")),
        json!({"term": {"author.name": "Ada"}})
    );
    let hybrid = Retriever::rrf([
        Article::BODY.semantic("tuning").into(),
        Article::EMBEDDING.knn(vec![1.0, 0.0, 0.0], 5, 50),
        Retriever::standard(Query::bool().filter(Article::DRAFT.term(false))),
    ]);
    let hybrid = serde_json::to_value(hybrid).unwrap();
    assert_eq!(
        hybrid["rrf"]["retrievers"][0],
        json!({"standard": {"query": {"semantic": {"field": "body", "query": "tuning"}}}})
    );
}
