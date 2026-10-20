//! Enrich policies against Elasticsearch: a policy over a source index,
//! executed, and used by the enrich processor to add the source's data to
//! a document as it's ingested.

use oxsearch::enrich::execute_policy::EnrichPolicyPhase;
use oxsearch::enrich::{Policy, delete_policy, execute_policy, get_policy, put_policy, stats};
use oxsearch::ingest::{Document, EnrichProcessor, Pipeline, simulate};
use oxsearch::mapping::{KeywordProperty, Property, TypeMapping};
use oxsearch::shapes::IndexMap;
use oxsearch::{Error, Refresh};
use oxsearch_docs::index;
use oxsearch_integration::{TestIndex, client};
use serde_json::json;

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn policies() {
    let client = client();
    // The match field must be a keyword field of its own.
    let mut properties = IndexMap::new();
    properties.insert(
        "email".to_owned(),
        Property::Keyword(KeywordProperty::new()),
    );
    let users = TestIndex::create(
        &client,
        "enrich-users",
        &TypeMapping::new().properties(properties),
    )
    .await;
    for (n, (email, name, city)) in [
        ("ada@example.com", "Ada", "London"),
        ("grace@example.com", "Grace", "Arlington"),
    ]
    .into_iter()
    .enumerate()
    {
        client
            .send(
                &index::Request::new(users.name())
                    .id(n.to_string())
                    .refresh(Refresh::True)
                    .body(json!({"email": email, "name": name, "city": city})),
            )
            .await
            .unwrap();
    }

    // Users looked up by email.
    let policy = format!("{}-policy", users.name());
    client
        .send(&put_policy::Request::new(policy.as_str()).body(
            put_policy::RequestBody::new().r#match(Policy::new(
                vec!["name".to_owned(), "city".to_owned()],
                users.name(),
                "email",
            )),
        ))
        .await
        .unwrap();
    let read = client
        .send(&get_policy::Request::new().name(policy.as_str()))
        .await
        .unwrap();
    assert_eq!(read.policies.len(), 1);
    let executed = client
        .send(&execute_policy::Request::new(policy.as_str()).wait_for_completion(true))
        .await
        .unwrap();
    assert_eq!(
        executed.status.and_then(|s| s.phase),
        Some(EnrichPolicyPhase::Complete)
    );
    client.send(&stats::Request::new()).await.unwrap();

    // A document that names a user gets the user's name and city.
    let enriched = client
        .send(
            &simulate::Request::new().body(
                simulate::RequestBody::new(vec![Document::new(
                    json!({"order": 7, "email": "ada@example.com"}),
                )])
                .pipeline(Pipeline::new().processors(vec![
                    EnrichProcessor::new("email", policy.as_str(), "user").into(),
                ])),
            ),
        )
        .await
        .unwrap();
    let source = &enriched.docs[0].doc.as_ref().unwrap().source;
    assert_eq!(source["user"]["name"], "Ada");
    assert_eq!(source["user"]["city"], "London");

    client
        .send(&delete_policy::Request::new(policy.as_str()))
        .await
        .unwrap();
    let gone = client
        .send(&get_policy::Request::new().name(policy.as_str()))
        .await;
    match gone {
        Ok(read) => assert!(read.policies.is_empty(), "{read:?}"),
        Err(Error::Elasticsearch(e)) => assert_eq!(e.status, 404),
        Err(e) => panic!("{e}"),
    }
    users.delete(&client).await;
}
