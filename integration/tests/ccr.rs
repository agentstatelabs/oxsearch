//! Cross-cluster replication against Elasticsearch, with the fixture as
//! both clusters: a remote cluster that points back at its own node, so it
//! follows its own indices. A follower index copies a leader index, is
//! read, paused, resumed and made an ordinary index, and the leader forgets
//! it; an auto-follow pattern is put, read, paused, resumed and deleted;
//! and the replication statistics. Replication needs a license above
//! Basic, so on Basic the test checks that the statistics are refused, and
//! stops there.

use std::time::{Duration, Instant};

use oxsearch::admin::cluster;
use oxsearch::admin::nodes;
use oxsearch::ccr::follow_info::FollowerIndexStatus;
use oxsearch::ccr::{
    delete_auto_follow_pattern, follow, follow_info, follow_stats, forget_follower,
    get_auto_follow_pattern, pause_auto_follow_pattern, pause_follow, put_auto_follow_pattern,
    resume_auto_follow_pattern, resume_follow, stats, unfollow,
};
use oxsearch::search::count;
use oxsearch::shapes::IndexMap;
use oxsearch::{Error, Method, Refresh};
use oxsearch_docs::index;
use oxsearch_integration::{License, TestClient, TestIndex, client};
use serde_json::{Value, json};

/// Points the remote cluster `alias` at `seeds`, or removes it.
async fn remote(client: &TestClient, alias: &str, seeds: Value) {
    let mut persistent = IndexMap::new();
    persistent.insert(format!("cluster.remote.{alias}.seeds"), seeds);
    client
        .send(
            &cluster::put_settings::Request::new()
                .body(cluster::put_settings::RequestBody::new().persistent(persistent)),
        )
        .await
        .unwrap();
}

/// Waits until the index holds `n` documents.
async fn holds(client: &TestClient, index: &str, n: i64) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        client
            .send_raw(Method::Post, &format!("/{index}/_refresh"), None)
            .await
            .unwrap();
        let counted = client
            .send(&count::Request::new().index(index))
            .await
            .unwrap();
        if counted.count == Some(n) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{index} holds {:?}, not {n}",
            counted.count
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// The follower's status, as `follow_info` gives it.
async fn status(client: &TestClient, follower: &str) -> Option<FollowerIndexStatus> {
    let info = client
        .send(&follow_info::Request::new(follower))
        .await
        .unwrap();
    info.follower_indices[0].status.clone()
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn replication() {
    let client = client();
    let refused = client.send(&stats::Request::new()).await;
    if License::from_env() == License::Basic {
        assert!(matches!(refused, Err(Error::License(_))), "{refused:?}");
        return;
    }
    refused.unwrap();

    // The fixture as its own remote cluster, through its transport address.
    let leader = TestIndex::named("ccr");
    let alias = leader.name().to_owned();
    let info = client.send(&nodes::info::Request::new()).await.unwrap();
    let node = info.nodes.values().next().expect("a node");
    let address = node.transport_address.clone().expect("a transport address");
    remote(&client, &alias, json!([address])).await;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let remotes = client
            .send(&cluster::remote_info::Request::new())
            .await
            .unwrap();
        if matches!(
            remotes.0.get(&alias),
            Some(cluster::remote_info::ClusterRemoteInfo::Sniff(s)) if s.connected == Some(true)
        ) {
            break;
        }
        assert!(Instant::now() < deadline, "not connected: {remotes:?}");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    // A leader index with three notes, and its follower.
    for n in 0..3 {
        client
            .send(
                &index::Request::new(leader.name())
                    .id(n.to_string())
                    .refresh(Refresh::True)
                    .body(json!({"title": format!("note {n}")})),
            )
            .await
            .unwrap();
    }
    let follower = format!("{}-follower", leader.name());
    let followed = client
        .send(
            &follow::Request::new(follower.as_str())
                .wait_for_active_shards(1)
                .body(follow::RequestBody::new(leader.name(), alias.as_str())),
        )
        .await
        .unwrap();
    assert_eq!(followed.follow_index_created, Some(true), "{followed:?}");
    assert_eq!(followed.follow_index_shards_acked, Some(true));
    assert_eq!(followed.index_following_started, Some(true));
    holds(&client, &follower, 3).await;

    let info = client
        .send(&follow_info::Request::new(follower.as_str()))
        .await
        .unwrap();
    let following = &info.follower_indices[0];
    assert_eq!(following.leader_index.as_deref(), Some(leader.name()));
    assert_eq!(following.remote_cluster.as_deref(), Some(alias.as_str()));
    assert_eq!(following.status, Some(FollowerIndexStatus::Active));
    let shards = client
        .send(&follow_stats::Request::new(follower.as_str()))
        .await
        .unwrap();
    assert_eq!(shards.indices[0].index.as_deref(), Some(follower.as_str()));
    assert!(!shards.indices[0].shards.is_empty(), "{shards:?}");
    let all = client.send(&stats::Request::new()).await.unwrap();
    let following = all.follow_stats.expect("the follow statistics");
    assert!(
        following
            .indices
            .iter()
            .any(|i| i.index.as_deref() == Some(follower.as_str())),
        "{following:?}"
    );

    // Paused, and resumed to copy a fourth note.
    client
        .send(&pause_follow::Request::new(follower.as_str()))
        .await
        .unwrap();
    assert_eq!(
        status(&client, &follower).await,
        Some(FollowerIndexStatus::Paused)
    );
    client
        .send(
            &resume_follow::Request::new(follower.as_str())
                .body(resume_follow::RequestBody::new().read_poll_timeout("1s")),
        )
        .await
        .unwrap();
    client
        .send(
            &index::Request::new(leader.name())
                .id("3")
                .refresh(Refresh::True)
                .body(json!({"title": "note 3"})),
        )
        .await
        .unwrap();
    holds(&client, &follower, 4).await;

    // Made an ordinary index: paused, closed and unfollowed. Then the
    // leader forgets it, which unfollowing has mostly done already.
    client
        .send(&pause_follow::Request::new(follower.as_str()))
        .await
        .unwrap();
    client
        .send_raw(Method::Post, &format!("/{follower}/_close"), None)
        .await
        .unwrap();
    client
        .send(&unfollow::Request::new(follower.as_str()))
        .await
        .unwrap();
    let settings = client
        .send_raw(
            Method::Get,
            &format!("/{follower}/_settings?flat_settings=true"),
            None,
        )
        .await
        .unwrap();
    let uuid = settings[&follower]["settings"]["index.uuid"]
        .as_str()
        .expect("the follower's UUID")
        .to_owned();
    let root = client.send_raw(Method::Get, "/", None).await.unwrap();
    let cluster_name = root["cluster_name"].as_str().expect("the cluster's name");
    let forgotten = client
        .send(
            &forget_follower::Request::new(leader.name()).body(
                forget_follower::RequestBody::new()
                    .follower_cluster(cluster_name)
                    .follower_index(follower.as_str())
                    .follower_index_uuid(uuid)
                    .leader_remote_cluster(alias.as_str()),
            ),
        )
        .await
        .unwrap();
    let shards = forgotten.shards.expect("the shards");
    assert_eq!(shards.total, Some(1), "{shards:?}");

    // An auto-follow pattern for the leader's later namesakes: put, read,
    // paused, resumed and deleted.
    let pattern = format!("{}-pattern", leader.name());
    client
        .send(
            &put_auto_follow_pattern::Request::new(pattern.as_str()).body(
                put_auto_follow_pattern::RequestBody::new(alias.as_str())
                    .leader_index_patterns(vec![format!("{}-later-*", leader.name())])
                    .follow_index_pattern("{{leader_index}}-copy"),
            ),
        )
        .await
        .unwrap();
    let active = || async {
        let read = client
            .send(&get_auto_follow_pattern::Request::new().name(pattern.as_str()))
            .await
            .unwrap();
        read.patterns[0].pattern.as_ref().and_then(|p| p.active)
    };
    assert_eq!(active().await, Some(true));
    client
        .send(&pause_auto_follow_pattern::Request::new(pattern.as_str()))
        .await
        .unwrap();
    assert_eq!(active().await, Some(false));
    client
        .send(&resume_auto_follow_pattern::Request::new(pattern.as_str()))
        .await
        .unwrap();
    assert_eq!(active().await, Some(true));
    client
        .send(&delete_auto_follow_pattern::Request::new(pattern.as_str()))
        .await
        .unwrap();

    for index in [follower.as_str(), leader.name()] {
        client
            .send_raw(Method::Delete, &format!("/{index}"), None)
            .await
            .unwrap();
    }
    remote(&client, &alias, Value::Null).await;
}
