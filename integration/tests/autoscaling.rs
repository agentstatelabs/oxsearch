//! Autoscaling against Elasticsearch: a policy with the fixed decider,
//! which asks for a set capacity whatever the cluster holds, put, read, the
//! capacity it calls for read, and deleted. Elasticsearch meant these APIs
//! for Elastic's orchestrators, and they need a license above Basic, so on
//! Basic the test checks that putting the policy is refused, and stops
//! there.

use oxsearch::Error;
use oxsearch::autoscaling::{
    AutoscalingPolicy, delete_autoscaling_policy, get_autoscaling_capacity, get_autoscaling_policy,
    put_autoscaling_policy,
};
use oxsearch::shapes::IndexMap;
use oxsearch_integration::{License, TestIndex, client};
use serde_json::json;

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn policies() {
    let client = client();
    let name = TestIndex::named("autoscaling").name().to_owned();
    // Two nodes of 2 GiB of storage each: 4 GiB in all, more than an
    // `integer` holds.
    let mut deciders = IndexMap::new();
    deciders.insert("fixed".to_owned(), json!({"storage": "2gb", "nodes": 2}));
    let put =
        client
            .send(&put_autoscaling_policy::Request::new(name.as_str()).body(
                AutoscalingPolicy::new(vec!["data_hot".to_owned()], deciders),
            ))
            .await;
    if License::from_env() == License::Basic {
        assert!(matches!(put, Err(Error::License(_))), "{put:?}");
        return;
    }
    assert_eq!(put.unwrap().0.acknowledged, Some(true));

    let read = client
        .send(&get_autoscaling_policy::Request::new(name.as_str()))
        .await
        .unwrap();
    assert_eq!(read.0.roles, vec!["data_hot".to_owned()]);
    assert!(read.0.deciders.contains_key("fixed"), "{read:?}");

    let capacity = client
        .send(&get_autoscaling_capacity::Request::new())
        .await
        .unwrap();
    let policy = &capacity.policies[name.as_str()];
    let required = policy
        .required_capacity
        .as_ref()
        .and_then(|c| c.total.as_ref())
        .expect("the total capacity");
    assert_eq!(required.storage, Some(4 << 30), "{policy:?}");
    assert!(policy.deciders.contains_key("fixed"), "{policy:?}");

    let deleted = client
        .send(&delete_autoscaling_policy::Request::new(name.as_str()))
        .await
        .unwrap();
    assert_eq!(deleted.0.acknowledged, Some(true));
}
