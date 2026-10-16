//! Administration against Elasticsearch: the cluster's health, settings and
//! templates, the node's information and statistics, and indices from
//! creation to deletion, with their aliases and data streams.

use oxsearch::admin::cluster;
use oxsearch::admin::indices::{
    self, IndexSettings, IndexSettingsNumberOfReplicas, IndexSettingsNumberOfShards,
    IndicesBlockOptions,
};
use oxsearch::admin::nodes;
use oxsearch::mapping::{KeywordProperty, Property, TextProperty, TypeMapping};
use oxsearch::shapes::IndexMap;
use oxsearch::{ClusterInfoTarget, Error, HealthStatus, OpType, Refresh};
use oxsearch_docs::index;
use oxsearch_integration::{TestClient, TestIndex, client};
use serde_json::{Value, json};

/// A mapping with a text and a keyword field.
fn mapping() -> TypeMapping {
    let mut properties = IndexMap::new();
    properties.insert("title".to_owned(), Property::Text(TextProperty::new()));
    properties.insert("tag".to_owned(), Property::Keyword(KeywordProperty::new()));
    TypeMapping::new().properties(properties)
}

/// One shard, and `replicas` replicas.
fn shards(replicas: i32) -> IndexSettings {
    IndexSettings::new()
        .number_of_shards(IndexSettingsNumberOfShards::Integer(1))
        .number_of_replicas(IndexSettingsNumberOfReplicas::Integer(replicas))
}

/// Whether a call failed with Elasticsearch's 404.
fn not_found<T>(result: &Result<T, Error>) -> bool {
    matches!(result, Err(Error::Elasticsearch(e)) if e.status == 404)
}

/// A setting as Elasticsearch reports it: a string, whatever its type.
fn setting(settings: &IndexMap<String, Value>, name: &str) -> Option<String> {
    settings
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

async fn write(client: &TestClient, index: &str, id: &str, body: Value) {
    client
        .send(
            &index::Request::new(index)
                .id(id)
                .refresh(Refresh::True)
                .body(body),
        )
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn cluster() {
    let client = client();

    // A single node: green, or yellow while an index waits for a replica
    // it can't place.
    let health = client.send(&cluster::health::Request::new()).await.unwrap();
    assert_eq!(health.number_of_nodes, Some(1));
    assert!(
        matches!(
            health.status,
            Some(HealthStatus::Green | HealthStatus::Yellow)
        ),
        "{health:?}"
    );

    // An index with a replica stays yellow, and allocation explains why.
    let test = TestIndex::named("health");
    client
        .send(
            &indices::create::Request::new(test.name())
                .body(indices::create::RequestBody::new().settings(shards(1))),
        )
        .await
        .unwrap();
    let health = client
        .send(
            &cluster::health::Request::new()
                .index(test.name())
                .wait_for_status(HealthStatus::Yellow)
                .level(oxsearch::Level::Indices),
        )
        .await
        .unwrap();
    assert_eq!(health.status, Some(HealthStatus::Yellow));
    assert_eq!(health.unassigned_shards, Some(1));
    let index = &health.indices.as_ref().expect("per-index health")[test.name()];
    assert_eq!(index.status, Some(HealthStatus::Yellow));
    let explained = client
        .send(
            &cluster::allocation_explain::Request::new().body(
                cluster::allocation_explain::RequestBody::new()
                    .index(test.name())
                    .shard(0)
                    .primary(false),
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        explained.can_allocate,
        Some(cluster::allocation_explain::Decision::No)
    );

    // The cluster's state and statistics.
    let state = client
        .send(
            &cluster::state::Request::new()
                .metric(cluster::state::ClusterStateMetric::Metadata)
                .index(test.name()),
        )
        .await
        .unwrap();
    assert!(
        state["metadata"]["indices"][test.name()].is_object(),
        "{state:?}"
    );
    let stats = client.send(&cluster::stats::Request::new()).await.unwrap();
    assert!(stats.cluster_name.is_some(), "{stats:?}");
    let info = client
        .send(&cluster::info::Request::new(ClusterInfoTarget::All))
        .await
        .unwrap();
    assert!(
        info.cluster_name.is_some() && info.http.is_some(),
        "{info:?}"
    );
    client
        .send(&cluster::pending_tasks::Request::new())
        .await
        .unwrap();
    let remotes = client
        .send(&cluster::remote_info::Request::new())
        .await
        .unwrap();
    assert!(remotes.is_empty(), "{remotes:?}");
    test.delete(&client).await;

    // A persistent setting, set, read back and cleared.
    const RECOVERY: &str = "indices.recovery.max_bytes_per_sec";
    let set = |value: Value| {
        let mut persistent = IndexMap::new();
        persistent.insert(RECOVERY.to_owned(), value);
        cluster::put_settings::Request::new()
            .body(cluster::put_settings::RequestBody::new().persistent(persistent))
    };
    let response = client.send(&set(json!("50mb"))).await.unwrap();
    assert_eq!(response.acknowledged, Some(true));
    let settings = client
        .send(&cluster::get_settings::Request::new().flat_settings(true))
        .await
        .unwrap();
    assert_eq!(
        setting(&settings.persistent, RECOVERY).as_deref(),
        Some("50mb")
    );
    client.send(&set(Value::Null)).await.unwrap();
    let settings = client
        .send(&cluster::get_settings::Request::new().flat_settings(true))
        .await
        .unwrap();
    assert_eq!(setting(&settings.persistent, RECOVERY), None);
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn nodes() {
    let client = client();
    let info = client.send(&nodes::info::Request::new()).await.unwrap();
    assert_eq!(info.nodes2.len(), 1);
    let node = info.nodes2.values().next().unwrap();
    assert!(node.name.is_some());
    assert!(
        node.version.as_deref().is_some_and(|v| v.starts_with("9.")),
        "{:?}",
        node.version
    );

    let stats = client
        .send(&nodes::stats::Request::new().metric(vec![
            nodes::stats::NodeStatsMetric::Jvm,
            nodes::stats::NodeStatsMetric::Indices,
        ]))
        .await
        .unwrap();
    let node = stats.nodes2.values().next().unwrap();
    let heap = node
        .jvm
        .as_ref()
        .and_then(|jvm| jvm.mem.as_ref())
        .and_then(|mem| mem.heap_used_in_bytes);
    assert!(heap.is_some_and(|bytes| bytes > 0), "{:?}", node.jvm);
    assert!(node.indices.is_some());

    let usage = client.send(&nodes::usage::Request::new()).await.unwrap();
    let node = usage.nodes2.values().next().unwrap();
    assert!(!node.rest_actions.is_empty(), "{node:?}");

    // Hot threads come back as text.
    let threads = client
        .send(&nodes::hot_threads::Request::new())
        .await
        .unwrap();
    assert!(threads.contains("Hot threads at"), "{}", threads.0);

    let reloaded = client
        .send(&nodes::reload_secure_settings::Request::new())
        .await
        .unwrap();
    assert_eq!(reloaded.nodes2.len(), 1);
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn index_life() {
    let client = client();
    let test = TestIndex::named("life");
    let name = test.name().to_owned();
    let name = name.as_str();
    let alias = format!("{name}-alias");

    // Created with its settings, mapping and an alias.
    let mut aliases = IndexMap::new();
    aliases.insert(alias.clone(), indices::Alias::new());
    let created = client
        .send(
            &indices::create::Request::new(name).body(
                indices::create::RequestBody::new()
                    .settings(shards(0))
                    .mappings(mapping())
                    .aliases(aliases),
            ),
        )
        .await
        .unwrap();
    assert_eq!(created.index.as_deref(), Some(name));
    client
        .send(&indices::exists::Request::new(name))
        .await
        .unwrap();
    assert!(not_found(
        &client
            .send(&indices::exists::Request::new(format!("{name}-missing")))
            .await
    ));

    // Read back whole, and in parts.
    let got = client
        .send(&indices::get::Request::new(name))
        .await
        .unwrap();
    let state = &got[name];
    assert!(state.aliases.as_ref().unwrap().contains_key(&alias));
    let properties = state
        .mappings
        .as_ref()
        .and_then(|m| m.properties.as_ref())
        .unwrap();
    assert!(matches!(properties.get("tag"), Some(Property::Keyword(_))));
    let settings = state
        .settings
        .as_ref()
        .and_then(|s| s.index.as_ref())
        .unwrap();
    assert_eq!(
        settings.number_of_shards,
        Some(IndexSettingsNumberOfShards::String("1".into()))
    );

    // Settings changed and read back.
    client
        .send(
            &indices::put_settings::Request::new()
                .index(name)
                .body(IndexSettings::new().refresh_interval("5s")),
        )
        .await
        .unwrap();
    let settings = client
        .send(&indices::get_settings::Request::new().index(name))
        .await
        .unwrap();
    let index = settings[name]
        .settings
        .as_ref()
        .and_then(|s| s.index.as_ref())
        .unwrap();
    assert_eq!(
        index.refresh_interval,
        Some(oxsearch::Duration::String("5s".into()))
    );

    // A field added to the mapping.
    let mut added = IndexMap::new();
    added.insert("body".to_owned(), Property::Text(TextProperty::new()));
    client
        .send(
            &indices::put_mapping::Request::new(name)
                .body(indices::put_mapping::RequestBody::new().properties(added)),
        )
        .await
        .unwrap();
    let fields = client
        .send(&indices::get_field_mapping::Request::new("body").index(name))
        .await
        .unwrap();
    assert!(fields[name].mappings.contains_key("body"), "{fields:?}");

    // Documents, and the calls that maintain the shards they're in.
    write(
        &client,
        name,
        "1",
        json!({"title": "hnsw graphs", "tag": "vectors"}),
    )
    .await;
    write(
        &client,
        name,
        "2",
        json!({"title": "bulk ingest", "tag": "ingest"}),
    )
    .await;
    client
        .send(&indices::refresh::Request::new().index(name))
        .await
        .unwrap();
    client
        .send(&indices::flush::Request::new().index(name))
        .await
        .unwrap();
    client
        .send(
            &indices::forcemerge::Request::new()
                .index(name)
                .max_num_segments(1),
        )
        .await
        .unwrap();
    client
        .send(&indices::clear_cache::Request::new().index(name))
        .await
        .unwrap();
    let stats = client
        .send(&indices::stats::Request::new().index(name))
        .await
        .unwrap();
    let docs = stats.indices.as_ref().unwrap()[name]
        .primaries
        .as_ref()
        .and_then(|p| p.docs.as_ref())
        .and_then(|d| d.count);
    assert_eq!(docs, Some(2));
    let segments = client
        .send(&indices::segments::Request::new().index(name))
        .await
        .unwrap();
    assert!(segments.indices.contains_key(name), "{segments:?}");
    let recovery = client
        .send(&indices::recovery::Request::new().index(name))
        .await
        .unwrap();
    assert!(recovery.contains_key(name), "{recovery:?}");
    client
        .send(&indices::shard_stores::Request::new().index(name))
        .await
        .unwrap();

    // Queries validated and text analyzed, against the index.
    let valid = client
        .send(
            &indices::validate_query::Request::new()
                .index(name)
                .q("title:hnsw"),
        )
        .await
        .unwrap();
    assert_eq!(valid.valid, Some(true));
    let analyzed = client
        .send(
            &indices::analyze::Request::new().index(name).body(
                indices::analyze::RequestBody::new()
                    .field("title")
                    .text("HNSW Graphs"),
            ),
        )
        .await
        .unwrap();
    let tokens: Vec<&str> = analyzed
        .tokens
        .as_ref()
        .unwrap()
        .iter()
        .filter_map(|t| t.token.as_deref())
        .collect();
    assert_eq!(tokens, ["hnsw", "graphs"]);
    let resolved = client
        .send(&indices::resolve_index::Request::new(name))
        .await
        .unwrap();
    assert_eq!(resolved.indices.len(), 1);

    // A write block, then closed and opened again.
    client
        .send(&indices::add_block::Request::new(
            name,
            IndicesBlockOptions::Write,
        ))
        .await
        .unwrap();
    let blocked = client
        .send(
            &index::Request::new(name)
                .id("3")
                .body(json!({"title": "x"})),
        )
        .await;
    assert!(
        matches!(&blocked, Err(Error::Elasticsearch(e)) if e.error_type() == Some("cluster_block_exception")),
        "{blocked:?}"
    );
    client
        .send(&indices::remove_block::Request::new(
            name,
            IndicesBlockOptions::Write,
        ))
        .await
        .unwrap();
    client
        .send(&indices::close::Request::new(name))
        .await
        .unwrap();
    client
        .send(&indices::open::Request::new(name))
        .await
        .unwrap();

    test.delete(&client).await;
    assert!(not_found(
        &client.send(&indices::exists::Request::new(name)).await
    ));
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn aliases() {
    let client = client();
    let mapping = mapping();
    let first = TestIndex::create(&client, "alias-a", &mapping).await;
    let second = TestIndex::create(&client, "alias-b", &mapping).await;
    let alias = format!("{}-current", first.name());

    client
        .send(&indices::put_alias::Request::new(
            first.name(),
            alias.as_str(),
        ))
        .await
        .unwrap();
    client
        .send(&indices::exists_alias::Request::new(alias.as_str()))
        .await
        .unwrap();
    let aliases = client
        .send(&indices::get_alias::Request::new().name(alias.as_str()))
        .await
        .unwrap();
    assert_eq!(aliases.keys().collect::<Vec<_>>(), [first.name()]);

    // Moved to the second index in one atomic change.
    client
        .send(&indices::update_aliases::Request::new().body(
            indices::update_aliases::RequestBody::new().actions(vec![
                    indices::update_aliases::Action::Remove(
                        indices::update_aliases::RemoveAction::new()
                            .index(first.name())
                            .alias(alias.as_str()),
                    ),
                    indices::update_aliases::Action::Add(
                        indices::update_aliases::AddAction::new()
                            .index(second.name())
                            .alias(alias.as_str()),
                    ),
                ]),
        ))
        .await
        .unwrap();
    let aliases = client
        .send(&indices::get_alias::Request::new().name(alias.as_str()))
        .await
        .unwrap();
    assert_eq!(aliases.keys().collect::<Vec<_>>(), [second.name()]);

    client
        .send(&indices::delete_alias::Request::new(
            second.name(),
            alias.as_str(),
        ))
        .await
        .unwrap();
    assert!(not_found(
        &client
            .send(&indices::exists_alias::Request::new(alias.as_str()))
            .await
    ));

    // A copy of a write-blocked index, under a new name.
    let copy = TestIndex::named("alias-copy");
    client
        .send(&indices::add_block::Request::new(
            first.name(),
            IndicesBlockOptions::Write,
        ))
        .await
        .unwrap();
    let cloned = client
        .send(&indices::clone::Request::new(first.name(), copy.name()))
        .await
        .unwrap();
    assert_eq!(cloned.index.as_deref(), Some(copy.name()));
    copy.delete(&client).await;
    first.delete(&client).await;
    second.delete(&client).await;

    // Rolled over to a new index behind a write alias.
    let rolled = TestIndex::named("rollover");
    let write_alias = format!("{}-writes", rolled.name());
    let mut aliases = IndexMap::new();
    aliases.insert(
        write_alias.clone(),
        indices::Alias::new().is_write_index(true),
    );
    let old = format!("{}-000001", rolled.name());
    client
        .send(
            &indices::create::Request::new(old.as_str())
                .body(indices::create::RequestBody::new().aliases(aliases)),
        )
        .await
        .unwrap();
    let dry = client
        .send(
            &indices::rollover::Request::new(write_alias.as_str())
                .dry_run(true)
                .body(
                    indices::rollover::RequestBody::new()
                        .conditions(indices::rollover::RolloverConditions::new().max_docs(1)),
                ),
        )
        .await
        .unwrap();
    assert_eq!(dry.dry_run, Some(true));
    assert_eq!(dry.rolled_over, Some(false));
    let over = client
        .send(&indices::rollover::Request::new(write_alias.as_str()))
        .await
        .unwrap();
    assert_eq!(over.rolled_over, Some(true));
    assert_eq!(over.old_index.as_deref(), Some(old.as_str()));
    let new = over.new_index.clone().unwrap();
    for index in [old, new] {
        client
            .send(&indices::delete::Request::new(index))
            .await
            .unwrap();
    }
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn templates_and_data_streams() {
    let client = client();
    let prefix = TestIndex::named("templated").name().to_owned();
    let component = format!("{prefix}-component");
    let template = format!("{prefix}-template");

    // A component template with the mapping, composed into an index template.
    client
        .send(
            &cluster::put_component_template::Request::new(component.as_str()).body(
                cluster::put_component_template::RequestBody::new(
                    indices::put_index_template::IndexTemplateMapping::new()
                        .mappings(mapping())
                        .settings(shards(0)),
                ),
            ),
        )
        .await
        .unwrap();
    client
        .send(&cluster::exists_component_template::Request::new(
            component.as_str(),
        ))
        .await
        .unwrap();
    let components = client
        .send(&cluster::get_component_template::Request::new().name(component.as_str()))
        .await
        .unwrap();
    assert_eq!(components.component_templates.len(), 1);
    assert_eq!(
        components.component_templates[0].name.as_deref(),
        Some(component.as_str())
    );

    client
        .send(
            &indices::put_index_template::Request::new(template.as_str()).body(
                indices::put_index_template::RequestBody::new()
                    .index_patterns(format!("{prefix}-*"))
                    .composed_of(vec![component.clone()])
                    .data_stream(indices::DataStreamVisibility::new())
                    .priority(500),
            ),
        )
        .await
        .unwrap();
    client
        .send(&indices::exists_index_template::Request::new(
            template.as_str(),
        ))
        .await
        .unwrap();
    let templates = client
        .send(&indices::get_index_template::Request::new().name(template.as_str()))
        .await
        .unwrap();
    assert_eq!(templates.index_templates.len(), 1);
    let simulated = client
        .send(&indices::simulate_index_template::Request::new(format!(
            "{prefix}-simulated"
        )))
        .await
        .unwrap();
    let properties = simulated
        .template
        .as_ref()
        .and_then(|t| t.mappings.as_ref())
        .and_then(|m| m.properties.as_ref())
        .unwrap();
    assert!(properties.contains_key("tag"), "{properties:?}");

    // A data stream the template shapes, written to and rolled over.
    let stream = format!("{prefix}-logs");
    client
        .send(&indices::create_data_stream::Request::new(stream.as_str()))
        .await
        .unwrap();
    client
        .send(
            &index::Request::new(stream.as_str())
                .op_type(OpType::Create)
                .refresh(Refresh::True)
                .body(json!({"@timestamp": "2026-10-16T00:00:00Z", "title": "started", "tag": "boot"})),
        )
        .await
        .unwrap();
    let streams = client
        .send(&indices::get_data_stream::Request::new().name(stream.as_str()))
        .await
        .unwrap();
    assert_eq!(streams.data_streams.len(), 1);
    assert_eq!(streams.data_streams[0].indices.len(), 1);
    let stats = client
        .send(&indices::data_streams_stats::Request::new().name(stream.as_str()))
        .await
        .unwrap();
    assert_eq!(stats.data_stream_count, Some(1));
    let over = client
        .send(&indices::rollover::Request::new(stream.as_str()))
        .await
        .unwrap();
    assert_eq!(over.rolled_over, Some(true));
    let streams = client
        .send(&indices::get_data_stream::Request::new().name(stream.as_str()))
        .await
        .unwrap();
    assert_eq!(streams.data_streams[0].indices.len(), 2);

    client
        .send(&indices::delete_data_stream::Request::new(stream.as_str()))
        .await
        .unwrap();
    client
        .send(&indices::delete_index_template::Request::new(
            template.as_str(),
        ))
        .await
        .unwrap();
    client
        .send(&cluster::delete_component_template::Request::new(
            component.as_str(),
        ))
        .await
        .unwrap();
    assert!(not_found(
        &client
            .send(&cluster::exists_component_template::Request::new(
                component.as_str()
            ))
            .await
    ));
}
