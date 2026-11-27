//! Analytics over an index: products counted and averaged by category,
//! once with aggregations read back as typed results, and once in ES|QL,
//! with the rows read into a struct of your own.
//!
//! It runs against Elasticsearch at `OXSEARCH_URL` (default
//! `http://localhost:9200`), such as the test fixture, on any license:
//!
//! ```text
//! scripts/es-fixture.sh up
//! cargo run -p oxsearch --example analytics --features admin,aggs,esql
//! ```
//!
//! Its dependencies, in an application of your own:
//!
//! ```toml
//! [dependencies]
//! oxsearch = { version = "0.3", features = ["admin", "aggs", "esql"] }
//! serde = { version = "1", features = ["derive"] }
//! serde_json = "1"
//! tokio = { version = "1", features = ["macros", "rt"] }
//! ```

use oxsearch::admin::indices::{create, delete};
use oxsearch::aggs::{Agg, Aggregates, Aggs, Results};
use oxsearch::docs::bulk;
use oxsearch::esql::{Esql, col};
use oxsearch::mapping::TypeMapping;
use oxsearch::search::search;
use oxsearch::transport_reqwest::ReqwestTransport;
use oxsearch::{Client, Refresh};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// A product, as the application stores it.
#[derive(Debug, Serialize)]
struct Product {
    name: String,
    category: String,
    price: f64,
}

// ANCHOR: row
/// A row of the ES|QL query's results, read by column name.
#[derive(Debug, Deserialize)]
struct Category {
    category: String,
    products: i64,
    average: f64,
}
// ANCHOR_END: row

const INDEX: &str = "oxsearch-example-analytics";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let url = std::env::var("OXSEARCH_URL").unwrap_or_else(|_| "http://localhost:9200".to_owned());
    let client = Client::new(ReqwestTransport::new(&url)?);

    // The index, its mapping written as JSON and read into the generated
    // type, which checks it.
    let mapping: TypeMapping = serde_json::from_value(json!({"properties": {
        "name": {"type": "text"},
        "category": {"type": "keyword"},
        "price": {"type": "double"},
    }}))?;
    let _ = client.send(&delete::Request::new(INDEX)).await;
    client
        .send(&create::Request::new(INDEX).body(create::RequestBody::new().mappings(mapping)))
        .await?;
    let products = [
        ("HNSW in practice", "books", 32.0),
        ("A vector search primer", "books", 24.0),
        ("Shard sizing cheat sheet", "books", 9.0),
        ("Cluster mug", "merch", 12.0),
        ("Replica socks", "merch", 8.0),
        ("Relevance tuning course", "courses", 140.0),
    ];
    let mut lines: Vec<bulk::RequestBody<Product>> = Vec::new();
    for (name, category, price) in products {
        lines.push(bulk::RequestBody::OperationContainer(
            bulk::OperationContainer::Index(bulk::IndexOperation::new().index(INDEX)),
        ));
        lines.push(bulk::RequestBody::TDocument(Product {
            name: name.to_owned(),
            category: category.to_owned(),
            price,
        }));
    }
    client
        .send(&bulk::Request::new().refresh(Refresh::True).body(lines))
        .await?;

    // ANCHOR: aggregations
    // Products by category, with each category's average price: a terms
    // aggregation with an average under it, sent with `typed_keys` so the
    // results read back typed.
    let request = search::Request::new().index(INDEX).typed_keys(true).body(
        search::RequestBody::new()
            .size(0)
            .aggregations(Aggs::new().add(
                "by_category",
                Agg::terms("category", 10).sub("avg_price", Agg::avg("price")),
            )),
    );
    let response: search::Response<Value, Aggregates> = client.send_as(&request).await?;
    let results = response.0.aggregations.expect("aggregation results");
    let averages = results
        .string_terms("by_category")
        .into_iter()
        .map(|bucket| bucket.aggregations.value("avg_price").unwrap_or_default());
    for ((category, count), average) in results.terms("by_category").into_iter().zip(averages) {
        println!("{category}: {count} products, {average:.2} on average");
    }
    // ANCHOR_END: aggregations

    // ANCHOR: esql
    // The same question in ES|QL. Values go as parameters, never into the
    // query's text, and the rows read into `Category` by column name.
    let rows: Vec<Category> = Esql::from_index(INDEX)
        .where_(col("price").gte(5.0))
        .stats(
            [("products", "COUNT(*)"), ("average", "AVG(price)")],
            ["category"],
        )
        .sort([col("products").desc(), col("category").asc()])
        .fetch(&client)
        .await?;
    for row in &rows {
        println!(
            "{}: {} products, {:.2} on average",
            row.category, row.products, row.average
        );
    }
    // ANCHOR_END: esql
    assert_eq!(rows.len(), 3);

    client.send(&delete::Request::new(INDEX)).await?;
    Ok(())
}
