//! Query rules: rulesets whose rules pin documents for queries that match
//! their criteria. They need a license above Basic.

use oxsearch::Error;
use oxsearch::query_rules::{
    QueryRule, QueryRuleActions, QueryRuleCriteria, QueryRuleCriteriaType, QueryRuleType,
    delete_rule, delete_ruleset, get_rule, get_ruleset, list_rulesets, put_rule, put_ruleset, test,
};
use oxsearch::shapes::IndexMap;
use oxsearch_integration::{License, TestIndex, client};
use serde_json::json;

/// Pins `id` when the `topic` is one of `topics`.
fn pin(rule: &str, topics: &[&str], id: &str) -> QueryRule {
    QueryRule::new(
        rule,
        QueryRuleType::Pinned,
        QueryRuleCriteria::new(QueryRuleCriteriaType::Exact)
            .metadata("topic")
            .values(topics.iter().map(|t| json!(t)).collect::<Vec<_>>()),
        QueryRuleActions::new().ids(vec![id.into()]),
    )
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn rulesets_and_rules() {
    let client = client();
    let ruleset = TestIndex::named("rules").name().to_owned();
    let created = client
        .send(
            &put_ruleset::Request::new(ruleset.as_str()).body(put_ruleset::RequestBody::new(vec![
                pin("pin-ingest", &["ingest", "bulk"], "2"),
            ])),
        )
        .await;
    if License::from_env() == License::Basic {
        let Err(Error::License(e)) = created else {
            panic!("{created:?}");
        };
        assert_eq!(e.feature, "query rules");
        return;
    }
    created.unwrap();

    // A rule added to the ruleset, and read back with it.
    client
        .send(
            &put_rule::Request::new(ruleset.as_str(), "pin-vectors").body(
                put_rule::RequestBody::new(
                    QueryRuleType::Pinned,
                    QueryRuleCriteria::new(QueryRuleCriteriaType::Exact)
                        .metadata("topic")
                        .values(vec![json!("vectors")]),
                    QueryRuleActions::new().ids(vec!["1".into(), "3".into()]),
                ),
            ),
        )
        .await
        .unwrap();
    let read = client
        .send(&get_ruleset::Request::new(ruleset.as_str()))
        .await
        .unwrap();
    let rules: Vec<&str> = read
        .rules
        .iter()
        .filter_map(|r| r.rule_id.as_deref())
        .collect();
    assert_eq!(rules, ["pin-ingest", "pin-vectors"]);
    let rule = client
        .send(&get_rule::Request::new(ruleset.as_str(), "pin-vectors"))
        .await
        .unwrap();
    let ids = rule.actions.as_ref().and_then(|a| a.ids.as_ref()).unwrap();
    assert_eq!(ids.len(), 2);
    let listed = client.send(&list_rulesets::Request::new()).await.unwrap();
    assert!(
        listed
            .results
            .iter()
            .any(|r| r.ruleset_id.as_deref() == Some(ruleset.as_str())),
        "{listed:?}"
    );

    // Which rules a query's metadata matches.
    let mut criteria = IndexMap::new();
    criteria.insert("topic".to_owned(), json!("bulk"));
    let matched = client
        .send(&test::Request::new(ruleset.as_str()).body(test::RequestBody::new(criteria)))
        .await
        .unwrap();
    assert_eq!(matched.total_matched_rules, Some(1));
    assert_eq!(
        matched.matched_rules[0].rule_id.as_deref(),
        Some("pin-ingest")
    );

    client
        .send(&delete_rule::Request::new(ruleset.as_str(), "pin-vectors"))
        .await
        .unwrap();
    client
        .send(&delete_ruleset::Request::new(ruleset.as_str()))
        .await
        .unwrap();
    let gone = client
        .send(&get_ruleset::Request::new(ruleset.as_str()))
        .await;
    assert!(
        matches!(&gone, Err(Error::Elasticsearch(e)) if e.status == 404),
        "{gone:?}"
    );
}
