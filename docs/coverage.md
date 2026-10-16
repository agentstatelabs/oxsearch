# Endpoint coverage

The endpoints generated in each crate, and those the integration tests send to Elasticsearch on the Basic license and a trial. Written by `cargo run -p xtask -- coverage` from the requests the tests record (`OXSEARCH_COVERAGE`); each CI integration job prints its own.

| Crate | Generated | Integration-tested |
|---|---:|---:|
| `oxsearch-core` | 2 | 2 (100%) |
| `oxsearch-tasks` | 3 | 3 (100%) |
| `oxsearch-inference` | 41 | 6 (14%) |
| `oxsearch-search` | 17 | 15 (88%) |
| `oxsearch-docs` | 21 | 21 (100%) |
| `oxsearch-admin` | 102 | 58 (56%) |
| all | 186 | 105 (56%) |

## `oxsearch-core`

Tested: `info`, `ping`.


## `oxsearch-tasks`

Tested: `tasks.cancel`, `tasks.get`, `tasks.list`.


## `oxsearch-inference`

Tested: `inference.delete`, `inference.get`, `inference.inference`, `inference.put`, `inference.rerank`, `inference.sparse_embedding`.

Not tested yet: `inference.chat_completion_unified`, `inference.completion`, `inference.delete_region_policy`, `inference.embedding`, `inference.get_region_policy`, `inference.put_ai21`, `inference.put_alibabacloud`, `inference.put_amazonbedrock`, `inference.put_amazonsagemaker`, `inference.put_anthropic`, `inference.put_azureaistudio`, `inference.put_azureopenai`, `inference.put_cohere`, `inference.put_contextualai`, `inference.put_custom`, `inference.put_deepseek`, `inference.put_elasticsearch`, `inference.put_elser`, `inference.put_fireworksai`, `inference.put_googleaistudio`, `inference.put_googlevertexai`, `inference.put_groq`, `inference.put_hugging_face`, `inference.put_jinaai`, `inference.put_llama`, `inference.put_mistral`, `inference.put_nvidia`, `inference.put_openai`, `inference.put_openshift_ai`, `inference.put_region_policy`, `inference.put_voyageai`, `inference.put_watsonx`, `inference.stream_completion`, `inference.text_embedding`, `inference.update`.

## `oxsearch-search`

Tested: `clear_scroll`, `close_point_in_time`, `count`, `explain`, `field_caps`, `msearch`, `msearch_template`, `open_point_in_time`, `rank_eval`, `render_search_template`, `scroll`, `search`, `search_shards`, `search_template`, `terms_enum`.

Not tested yet: `knn_search`, `search_mvt`.

## `oxsearch-docs`

Tested: `bulk`, `cancel_reindex`, `create`, `delete`, `delete_by_query`, `delete_by_query_rethrottle`, `exists`, `exists_source`, `get`, `get_reindex`, `get_source`, `index`, `list_reindex`, `mget`, `mtermvectors`, `reindex`, `reindex_rethrottle`, `termvectors`, `update`, `update_by_query`, `update_by_query_rethrottle`.


## `oxsearch-admin`

Tested: `cluster.allocation_explain`, `cluster.delete_component_template`, `cluster.exists_component_template`, `cluster.get_component_template`, `cluster.get_settings`, `cluster.health`, `cluster.info`, `cluster.pending_tasks`, `cluster.put_component_template`, `cluster.put_settings`, `cluster.remote_info`, `cluster.state`, `cluster.stats`, `indices.add_block`, `indices.analyze`, `indices.clear_cache`, `indices.clone`, `indices.close`, `indices.create`, `indices.create_data_stream`, `indices.data_streams_stats`, `indices.delete`, `indices.delete_alias`, `indices.delete_data_stream`, `indices.delete_index_template`, `indices.exists`, `indices.exists_alias`, `indices.exists_index_template`, `indices.flush`, `indices.forcemerge`, `indices.get`, `indices.get_alias`, `indices.get_data_stream`, `indices.get_field_mapping`, `indices.get_index_template`, `indices.get_mapping`, `indices.get_settings`, `indices.open`, `indices.put_alias`, `indices.put_index_template`, `indices.put_mapping`, `indices.put_settings`, `indices.recovery`, `indices.refresh`, `indices.remove_block`, `indices.resolve_index`, `indices.rollover`, `indices.segments`, `indices.shard_stores`, `indices.simulate_index_template`, `indices.stats`, `indices.update_aliases`, `indices.validate_query`, `nodes.hot_threads`, `nodes.info`, `nodes.reload_secure_settings`, `nodes.stats`, `nodes.usage`.

Not tested yet: `capabilities`, `cluster.delete_voting_config_exclusions`, `cluster.post_voting_config_exclusions`, `cluster.reroute`, `delete_script`, `get_script`, `get_script_context`, `get_script_languages`, `health_report`, `indices.cancel_migrate_reindex`, `indices.create_from`, `indices.delete_data_lifecycle`, `indices.delete_data_stream_options`, `indices.delete_template`, `indices.disk_usage`, `indices.downsample`, `indices.exists_template`, `indices.explain_data_lifecycle`, `indices.field_usage_stats`, `indices.get_data_lifecycle`, `indices.get_data_lifecycle_stats`, `indices.get_data_stream_mappings`, `indices.get_data_stream_options`, `indices.get_data_stream_settings`, `indices.get_migrate_reindex_status`, `indices.get_template`, `indices.migrate_reindex`, `indices.migrate_to_data_stream`, `indices.modify_data_stream`, `indices.promote_data_stream`, `indices.put_data_lifecycle`, `indices.put_data_stream_mappings`, `indices.put_data_stream_options`, `indices.put_data_stream_settings`, `indices.put_template`, `indices.reload_search_analyzers`, `indices.resolve_cluster`, `indices.shrink`, `indices.simulate_template`, `indices.split`, `nodes.clear_repositories_metering_archive`, `nodes.get_repositories_metering_info`, `put_script`, `scripts_painless_execute`.

## APIs not generated yet

The tests also call these through the escape hatches: `license.get`, `query_rules.delete_ruleset`, `query_rules.put_ruleset`.
