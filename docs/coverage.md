# Endpoint coverage

The endpoints generated in each crate, and those the integration tests send to Elasticsearch on the Basic license and a trial. Written by `cargo run -p xtask -- coverage` from the requests the tests record (`OXSEARCH_COVERAGE`); each CI integration job prints its own. Each endpoint's documentation also says where it's available: since which Elasticsearch version, and whether on Elastic Cloud Serverless.

| Crate | Generated | On Serverless | Integration-tested |
|---|---:|---:|---:|
| `oxsearch-core` | 2 | 2 | 2 (100%) |
| `oxsearch-tasks` | 3 | 1 | 3 (100%) |
| `oxsearch-inference` | 41 | 41 | 6 (14%) |
| `oxsearch-search` | 17 | 15 | 15 (88%) |
| `oxsearch-docs` | 21 | 19 | 21 (100%) |
| `oxsearch-admin` | 102 | 52 | 58 (56%) |
| `oxsearch-esql` | 16 | 6 | 4 (25%) |
| `oxsearch-ingest` | 12 | 5 | 8 (66%) |
| `oxsearch-license` | 7 | 1 | 3 (42%) |
| `oxsearch-query-rules` | 8 | 8 | 8 (100%) |
| `oxsearch-synonyms` | 7 | 7 | 7 (100%) |
| `oxsearch-snapshot` | 13 | 0 | 12 (92%) |
| `oxsearch-ilm` | 11 | 0 | 11 (100%) |
| `oxsearch-async-search` | 4 | 4 | 4 (100%) |
| `oxsearch-eql` | 4 | 4 | 4 (100%) |
| `oxsearch-sql` | 6 | 6 | 6 (100%) |
| `oxsearch-search-application` | 10 | 8 | 6 (60%) |
| `oxsearch-enrich` | 5 | 4 | 5 (100%) |
| `oxsearch-ml` | 73 | 53 | 73 (100%) |
| `oxsearch-watcher` | 13 | 0 | 13 (100%) |
| `oxsearch-slm` | 9 | 0 | 9 (100%) |
| `oxsearch-xpack` | 3 | 0 | 3 (100%) |
| `oxsearch-cat` | 27 | 10 | 27 (100%) |
| `oxsearch-security` | 68 | 14 | 68 (100%) |
| `oxsearch-transform` | 13 | 11 | 13 (100%) |
| `oxsearch-rollup` | 8 | 0 | 8 (100%) |
| `oxsearch-ccr` | 13 | 0 | 13 (100%) |
| `oxsearch-searchable-snapshots` | 4 | 0 | 4 (100%) |
| `oxsearch-dangling-indices` | 3 | 0 | 3 (100%) |
| `oxsearch-autoscaling` | 4 | 0 | 4 (100%) |
| `oxsearch-shutdown` | 3 | 0 | 3 (100%) |
| `oxsearch-migration` | 3 | 0 | 3 (100%) |
| `oxsearch-features` | 2 | 0 | 2 (100%) |
| `oxsearch-simulate` | 1 | 0 | 1 (100%) |
| `oxsearch-text-structure` | 4 | 0 | 4 (100%) |
| `oxsearch-connector` | 34 | 24 | 34 (100%) |
| `oxsearch-fleet` | 6 | 0 | 6 (100%) |
| `oxsearch-logstash` | 3 | 3 | 3 (100%) |
| `oxsearch-graph` | 1 | 0 | 1 (100%) |
| `oxsearch-profiling` | 4 | 0 | 4 (100%) |
| `oxsearch-project` | 6 | 6 | 0 (0%) |
| `oxsearch-streams` | 3 | 0 | 3 (100%) |
| all | 597 | 304 | 485 (81%) |

## `oxsearch-core`

Tested: `info`, `ping`.


## `oxsearch-tasks`

Tested: `tasks.cancel`, `tasks.get`, `tasks.list`.

Not on Elastic Cloud Serverless: `tasks.cancel`, `tasks.list`.


## `oxsearch-inference`

Tested: `inference.delete`, `inference.get`, `inference.inference`, `inference.put`, `inference.rerank`, `inference.sparse_embedding`.

Not tested yet: `inference.chat_completion_unified`, `inference.completion`, `inference.delete_region_policy`, `inference.embedding`, `inference.get_region_policy`, `inference.put_ai21`, `inference.put_alibabacloud`, `inference.put_amazonbedrock`, `inference.put_amazonsagemaker`, `inference.put_anthropic`, `inference.put_azureaistudio`, `inference.put_azureopenai`, `inference.put_cohere`, `inference.put_contextualai`, `inference.put_custom`, `inference.put_deepseek`, `inference.put_elasticsearch`, `inference.put_elser`, `inference.put_fireworksai`, `inference.put_googleaistudio`, `inference.put_googlevertexai`, `inference.put_groq`, `inference.put_hugging_face`, `inference.put_jinaai`, `inference.put_llama`, `inference.put_mistral`, `inference.put_nvidia`, `inference.put_openai`, `inference.put_openshift_ai`, `inference.put_region_policy`, `inference.put_voyageai`, `inference.put_watsonx`, `inference.stream_completion`, `inference.text_embedding`, `inference.update`.

Newer than Elasticsearch 8.19: `inference.delete_region_policy`, `inference.embedding`, `inference.get_region_policy`, `inference.put_ai21`, `inference.put_amazonsagemaker`, `inference.put_contextualai`, `inference.put_deepseek`, `inference.put_fireworksai`, `inference.put_groq`, `inference.put_llama`, `inference.put_nvidia`, `inference.put_openshift_ai`, `inference.put_region_policy`.


## `oxsearch-search`

Tested: `clear_scroll`, `close_point_in_time`, `count`, `explain`, `field_caps`, `msearch`, `msearch_template`, `open_point_in_time`, `rank_eval`, `render_search_template`, `scroll`, `search`, `search_shards`, `search_template`, `terms_enum`.

Not tested yet: `knn_search`, `search_mvt`.

Not on Elastic Cloud Serverless: `knn_search`, `search_shards`.


## `oxsearch-docs`

Tested: `bulk`, `cancel_reindex`, `create`, `delete`, `delete_by_query`, `delete_by_query_rethrottle`, `exists`, `exists_source`, `get`, `get_reindex`, `get_source`, `index`, `list_reindex`, `mget`, `mtermvectors`, `reindex`, `reindex_rethrottle`, `termvectors`, `update`, `update_by_query`, `update_by_query_rethrottle`.

Newer than Elasticsearch 8.19: `cancel_reindex`, `get_reindex`, `list_reindex`.

Not on Elastic Cloud Serverless: `delete_by_query_rethrottle`, `update_by_query_rethrottle`.


## `oxsearch-admin`

Tested: `cluster.allocation_explain`, `cluster.delete_component_template`, `cluster.exists_component_template`, `cluster.get_component_template`, `cluster.get_settings`, `cluster.health`, `cluster.info`, `cluster.pending_tasks`, `cluster.put_component_template`, `cluster.put_settings`, `cluster.remote_info`, `cluster.state`, `cluster.stats`, `indices.add_block`, `indices.analyze`, `indices.clear_cache`, `indices.clone`, `indices.close`, `indices.create`, `indices.create_data_stream`, `indices.data_streams_stats`, `indices.delete`, `indices.delete_alias`, `indices.delete_data_stream`, `indices.delete_index_template`, `indices.exists`, `indices.exists_alias`, `indices.exists_index_template`, `indices.flush`, `indices.forcemerge`, `indices.get`, `indices.get_alias`, `indices.get_data_stream`, `indices.get_field_mapping`, `indices.get_index_template`, `indices.get_mapping`, `indices.get_settings`, `indices.open`, `indices.put_alias`, `indices.put_index_template`, `indices.put_mapping`, `indices.put_settings`, `indices.recovery`, `indices.refresh`, `indices.remove_block`, `indices.resolve_index`, `indices.rollover`, `indices.segments`, `indices.shard_stores`, `indices.simulate_index_template`, `indices.stats`, `indices.update_aliases`, `indices.validate_query`, `nodes.hot_threads`, `nodes.info`, `nodes.reload_secure_settings`, `nodes.stats`, `nodes.usage`.

Not tested yet: `capabilities`, `cluster.delete_voting_config_exclusions`, `cluster.post_voting_config_exclusions`, `cluster.reroute`, `delete_script`, `get_script`, `get_script_context`, `get_script_languages`, `health_report`, `indices.cancel_migrate_reindex`, `indices.create_from`, `indices.delete_data_lifecycle`, `indices.delete_data_stream_options`, `indices.delete_template`, `indices.disk_usage`, `indices.downsample`, `indices.exists_template`, `indices.explain_data_lifecycle`, `indices.field_usage_stats`, `indices.get_data_lifecycle`, `indices.get_data_lifecycle_stats`, `indices.get_data_stream_mappings`, `indices.get_data_stream_options`, `indices.get_data_stream_settings`, `indices.get_migrate_reindex_status`, `indices.get_template`, `indices.migrate_reindex`, `indices.migrate_to_data_stream`, `indices.modify_data_stream`, `indices.promote_data_stream`, `indices.put_data_lifecycle`, `indices.put_data_stream_mappings`, `indices.put_data_stream_options`, `indices.put_data_stream_settings`, `indices.put_template`, `indices.reload_search_analyzers`, `indices.resolve_cluster`, `indices.shrink`, `indices.simulate_template`, `indices.split`, `nodes.clear_repositories_metering_archive`, `nodes.get_repositories_metering_info`, `put_script`, `scripts_painless_execute`.

Newer than Elasticsearch 8.19: `indices.get_data_stream_mappings`, `indices.get_data_stream_settings`, `indices.put_data_stream_mappings`, `indices.put_data_stream_settings`, `indices.remove_block`.

Not on Elastic Cloud Serverless: `capabilities`, `cluster.allocation_explain`, `cluster.delete_voting_config_exclusions`, `cluster.get_settings`, `cluster.health`, `cluster.pending_tasks`, `cluster.post_voting_config_exclusions`, `cluster.put_settings`, `cluster.remote_info`, `cluster.reroute`, `cluster.state`, `cluster.stats`, `get_script_context`, `get_script_languages`, `health_report`, `indices.clear_cache`, `indices.clone`, `indices.close`, `indices.data_streams_stats`, `indices.delete_data_lifecycle`, `indices.delete_data_stream_options`, `indices.delete_template`, `indices.disk_usage`, `indices.downsample`, `indices.exists_template`, `indices.field_usage_stats`, `indices.flush`, `indices.forcemerge`, `indices.get_data_lifecycle_stats`, `indices.get_field_mapping`, `indices.get_template`, `indices.migrate_reindex`, `indices.open`, `indices.promote_data_stream`, `indices.put_template`, `indices.recovery`, `indices.reload_search_analyzers`, `indices.resolve_cluster`, `indices.segments`, `indices.shard_stores`, `indices.shrink`, `indices.split`, `indices.stats`, `nodes.clear_repositories_metering_archive`, `nodes.get_repositories_metering_info`, `nodes.hot_threads`, `nodes.info`, `nodes.reload_secure_settings`, `nodes.stats`, `nodes.usage`.


## `oxsearch-esql`

Tested: `esql.async_query`, `esql.async_query_delete`, `esql.async_query_get`, `esql.query`.

Not tested yet: `esql.async_query_stop`, `esql.delete_data_source`, `esql.delete_dataset`, `esql.delete_view`, `esql.get_data_source`, `esql.get_dataset`, `esql.get_query`, `esql.get_view`, `esql.list_queries`, `esql.put_data_source`, `esql.put_dataset`, `esql.put_view`.

Newer than Elasticsearch 8.19: `esql.delete_view`, `esql.get_query`, `esql.get_view`, `esql.list_queries`, `esql.put_view`.

Not on Elastic Cloud Serverless: `esql.async_query`, `esql.async_query_delete`, `esql.async_query_get`, `esql.async_query_stop`, `esql.delete_data_source`, `esql.delete_dataset`, `esql.get_data_source`, `esql.get_dataset`, `esql.put_data_source`, `esql.put_dataset`.


## `oxsearch-ingest`

Tested: `ingest.delete_pipeline`, `ingest.geo_ip_stats`, `ingest.get_geoip_database`, `ingest.get_ip_location_database`, `ingest.get_pipeline`, `ingest.processor_grok`, `ingest.put_pipeline`, `ingest.simulate`.

Not tested yet: `ingest.delete_geoip_database`, `ingest.delete_ip_location_database`, `ingest.put_geoip_database`, `ingest.put_ip_location_database`.

Not on Elastic Cloud Serverless: `ingest.delete_geoip_database`, `ingest.delete_ip_location_database`, `ingest.geo_ip_stats`, `ingest.get_geoip_database`, `ingest.get_ip_location_database`, `ingest.put_geoip_database`, `ingest.put_ip_location_database`.


## `oxsearch-license`

Tested: `license.get`, `license.get_basic_status`, `license.get_trial_status`.

Not tested yet: `license.delete`, `license.post`, `license.post_start_basic`, `license.post_start_trial`.

Not on Elastic Cloud Serverless: `license.delete`, `license.get_basic_status`, `license.get_trial_status`, `license.post`, `license.post_start_basic`, `license.post_start_trial`.


## `oxsearch-query-rules`

Tested: `query_rules.delete_rule`, `query_rules.delete_ruleset`, `query_rules.get_rule`, `query_rules.get_ruleset`, `query_rules.list_rulesets`, `query_rules.put_rule`, `query_rules.put_ruleset`, `query_rules.test`.


## `oxsearch-synonyms`

Tested: `synonyms.delete_synonym`, `synonyms.delete_synonym_rule`, `synonyms.get_synonym`, `synonyms.get_synonym_rule`, `synonyms.get_synonyms_sets`, `synonyms.put_synonym`, `synonyms.put_synonym_rule`.


## `oxsearch-snapshot`

Tested: `snapshot.cleanup_repository`, `snapshot.clone`, `snapshot.create`, `snapshot.create_repository`, `snapshot.delete`, `snapshot.delete_repository`, `snapshot.get`, `snapshot.get_repository`, `snapshot.repository_analyze`, `snapshot.restore`, `snapshot.status`, `snapshot.verify_repository`.

Not tested yet: `snapshot.repository_verify_integrity`.

Not on Elastic Cloud Serverless: `snapshot.cleanup_repository`, `snapshot.clone`, `snapshot.create`, `snapshot.create_repository`, `snapshot.delete`, `snapshot.delete_repository`, `snapshot.get`, `snapshot.get_repository`, `snapshot.repository_analyze`, `snapshot.repository_verify_integrity`, `snapshot.restore`, `snapshot.status`, `snapshot.verify_repository`.


## `oxsearch-ilm`

Tested: `ilm.delete_lifecycle`, `ilm.explain_lifecycle`, `ilm.get_lifecycle`, `ilm.get_status`, `ilm.migrate_to_data_tiers`, `ilm.move_to_step`, `ilm.put_lifecycle`, `ilm.remove_policy`, `ilm.retry`, `ilm.start`, `ilm.stop`.

Not on Elastic Cloud Serverless: `ilm.delete_lifecycle`, `ilm.explain_lifecycle`, `ilm.get_lifecycle`, `ilm.get_status`, `ilm.migrate_to_data_tiers`, `ilm.move_to_step`, `ilm.put_lifecycle`, `ilm.remove_policy`, `ilm.retry`, `ilm.start`, `ilm.stop`.


## `oxsearch-async-search`

Tested: `async_search.delete`, `async_search.get`, `async_search.status`, `async_search.submit`.


## `oxsearch-eql`

Tested: `eql.delete`, `eql.get`, `eql.get_status`, `eql.search`.


## `oxsearch-sql`

Tested: `sql.clear_cursor`, `sql.delete_async`, `sql.get_async`, `sql.get_async_status`, `sql.query`, `sql.translate`.


## `oxsearch-search-application`

Tested: `search_application.delete`, `search_application.get`, `search_application.list`, `search_application.put`, `search_application.render_query`, `search_application.search`.

Not tested yet: `search_application.delete_behavioral_analytics`, `search_application.get_behavioral_analytics`, `search_application.post_behavioral_analytics_event`, `search_application.put_behavioral_analytics`.

Not on Elastic Cloud Serverless: `search_application.post_behavioral_analytics_event`, `search_application.render_query`.


## `oxsearch-enrich`

Tested: `enrich.delete_policy`, `enrich.execute_policy`, `enrich.get_policy`, `enrich.put_policy`, `enrich.stats`.

Not on Elastic Cloud Serverless: `enrich.stats`.


## `oxsearch-ml`

Tested: `ml.clear_trained_model_deployment_cache`, `ml.close_job`, `ml.delete_calendar`, `ml.delete_calendar_event`, `ml.delete_calendar_job`, `ml.delete_data_frame_analytics`, `ml.delete_datafeed`, `ml.delete_expired_data`, `ml.delete_filter`, `ml.delete_forecast`, `ml.delete_job`, `ml.delete_model_snapshot`, `ml.delete_trained_model`, `ml.delete_trained_model_alias`, `ml.estimate_model_memory`, `ml.evaluate_data_frame`, `ml.explain_data_frame_analytics`, `ml.flush_job`, `ml.forecast`, `ml.get_buckets`, `ml.get_calendar_events`, `ml.get_calendars`, `ml.get_categories`, `ml.get_data_frame_analytics`, `ml.get_data_frame_analytics_stats`, `ml.get_datafeed_stats`, `ml.get_datafeeds`, `ml.get_filters`, `ml.get_influencers`, `ml.get_job_stats`, `ml.get_jobs`, `ml.get_memory_stats`, `ml.get_model_snapshot_upgrade_stats`, `ml.get_model_snapshots`, `ml.get_overall_buckets`, `ml.get_records`, `ml.get_trained_models`, `ml.get_trained_models_stats`, `ml.infer_trained_model`, `ml.info`, `ml.open_job`, `ml.post_calendar_events`, `ml.post_data`, `ml.preview_data_frame_analytics`, `ml.preview_datafeed`, `ml.put_calendar`, `ml.put_calendar_job`, `ml.put_data_frame_analytics`, `ml.put_datafeed`, `ml.put_filter`, `ml.put_job`, `ml.put_trained_model`, `ml.put_trained_model_alias`, `ml.put_trained_model_definition_part`, `ml.put_trained_model_vocabulary`, `ml.reset_job`, `ml.revert_model_snapshot`, `ml.set_upgrade_mode`, `ml.start_data_frame_analytics`, `ml.start_datafeed`, `ml.start_trained_model_deployment`, `ml.stop_data_frame_analytics`, `ml.stop_datafeed`, `ml.stop_trained_model_deployment`, `ml.update_data_frame_analytics`, `ml.update_datafeed`, `ml.update_filter`, `ml.update_job`, `ml.update_model_snapshot`, `ml.update_trained_model_deployment`, `ml.upgrade_job_snapshot`, `ml.validate`, `ml.validate_detector`.

Not on Elastic Cloud Serverless: `ml.clear_trained_model_deployment_cache`, `ml.delete_expired_data`, `ml.delete_forecast`, `ml.delete_model_snapshot`, `ml.explain_data_frame_analytics`, `ml.get_buckets`, `ml.get_categories`, `ml.get_influencers`, `ml.get_memory_stats`, `ml.get_model_snapshot_upgrade_stats`, `ml.get_model_snapshots`, `ml.get_records`, `ml.info`, `ml.post_data`, `ml.revert_model_snapshot`, `ml.set_upgrade_mode`, `ml.update_model_snapshot`, `ml.upgrade_job_snapshot`, `ml.validate`, `ml.validate_detector`.


## `oxsearch-watcher`

Tested: `watcher.ack_watch`, `watcher.activate_watch`, `watcher.deactivate_watch`, `watcher.delete_watch`, `watcher.execute_watch`, `watcher.get_settings`, `watcher.get_watch`, `watcher.put_watch`, `watcher.query_watches`, `watcher.start`, `watcher.stats`, `watcher.stop`, `watcher.update_settings`.

Not on Elastic Cloud Serverless: `watcher.ack_watch`, `watcher.activate_watch`, `watcher.deactivate_watch`, `watcher.delete_watch`, `watcher.execute_watch`, `watcher.get_settings`, `watcher.get_watch`, `watcher.put_watch`, `watcher.query_watches`, `watcher.start`, `watcher.stats`, `watcher.stop`, `watcher.update_settings`.


## `oxsearch-slm`

Tested: `slm.delete_lifecycle`, `slm.execute_lifecycle`, `slm.execute_retention`, `slm.get_lifecycle`, `slm.get_stats`, `slm.get_status`, `slm.put_lifecycle`, `slm.start`, `slm.stop`.

Not on Elastic Cloud Serverless: `slm.delete_lifecycle`, `slm.execute_lifecycle`, `slm.execute_retention`, `slm.get_lifecycle`, `slm.get_stats`, `slm.get_status`, `slm.put_lifecycle`, `slm.start`, `slm.stop`.


## `oxsearch-xpack`

Tested: `monitoring.bulk`, `xpack.info`, `xpack.usage`.

Not on Elastic Cloud Serverless: `monitoring.bulk`, `xpack.info`, `xpack.usage`.


## `oxsearch-cat`

Tested: `cat.aliases`, `cat.allocation`, `cat.circuit_breaker`, `cat.component_templates`, `cat.count`, `cat.fielddata`, `cat.health`, `cat.help`, `cat.indices`, `cat.master`, `cat.ml_data_frame_analytics`, `cat.ml_datafeeds`, `cat.ml_jobs`, `cat.ml_trained_models`, `cat.nodeattrs`, `cat.nodes`, `cat.pending_tasks`, `cat.plugins`, `cat.recovery`, `cat.repositories`, `cat.segments`, `cat.shards`, `cat.snapshots`, `cat.tasks`, `cat.templates`, `cat.thread_pool`, `cat.transforms`.

Not on Elastic Cloud Serverless: `cat.allocation`, `cat.circuit_breaker`, `cat.fielddata`, `cat.health`, `cat.master`, `cat.nodeattrs`, `cat.nodes`, `cat.pending_tasks`, `cat.plugins`, `cat.recovery`, `cat.repositories`, `cat.segments`, `cat.shards`, `cat.snapshots`, `cat.tasks`, `cat.templates`, `cat.thread_pool`.


## `oxsearch-security`

Tested: `encryption.reset`, `security.activate_user_profile`, `security.authenticate`, `security.bulk_delete_role`, `security.bulk_put_role`, `security.bulk_update_api_keys`, `security.change_password`, `security.clear_api_key_cache`, `security.clear_cached_privileges`, `security.clear_cached_realms`, `security.clear_cached_roles`, `security.clear_cached_service_tokens`, `security.clone_api_key`, `security.create_api_key`, `security.create_cross_cluster_api_key`, `security.create_service_token`, `security.delegate_pki`, `security.delete_privileges`, `security.delete_role`, `security.delete_role_mapping`, `security.delete_service_token`, `security.delete_user`, `security.disable_user`, `security.disable_user_profile`, `security.enable_user`, `security.enable_user_profile`, `security.enroll_kibana`, `security.enroll_node`, `security.get_api_key`, `security.get_builtin_privileges`, `security.get_privileges`, `security.get_role`, `security.get_role_mapping`, `security.get_service_accounts`, `security.get_service_credentials`, `security.get_settings`, `security.get_stats`, `security.get_token`, `security.get_user`, `security.get_user_privileges`, `security.get_user_profile`, `security.grant_api_key`, `security.has_privileges`, `security.has_privileges_user_profile`, `security.invalidate_api_key`, `security.invalidate_token`, `security.oidc_authenticate`, `security.oidc_logout`, `security.oidc_prepare_authentication`, `security.put_privileges`, `security.put_role`, `security.put_role_mapping`, `security.put_user`, `security.query_api_keys`, `security.query_role`, `security.query_user`, `security.saml_authenticate`, `security.saml_complete_logout`, `security.saml_invalidate`, `security.saml_logout`, `security.saml_prepare_authentication`, `security.saml_service_provider_metadata`, `security.suggest_user_profiles`, `security.update_api_key`, `security.update_cross_cluster_api_key`, `security.update_settings`, `security.update_user_profile_data`, `ssl.certificates`.

Newer than Elasticsearch 8.19: `encryption.reset`, `security.clone_api_key`, `security.get_stats`.

Not on Elastic Cloud Serverless: `encryption.reset`, `security.activate_user_profile`, `security.bulk_delete_role`, `security.bulk_put_role`, `security.bulk_update_api_keys`, `security.change_password`, `security.clear_api_key_cache`, `security.clear_cached_privileges`, `security.clear_cached_realms`, `security.clear_cached_roles`, `security.clear_cached_service_tokens`, `security.clone_api_key`, `security.create_cross_cluster_api_key`, `security.delegate_pki`, `security.delete_privileges`, `security.delete_role_mapping`, `security.delete_user`, `security.disable_user`, `security.disable_user_profile`, `security.enable_user`, `security.enable_user_profile`, `security.enroll_kibana`, `security.enroll_node`, `security.get_privileges`, `security.get_role_mapping`, `security.get_service_accounts`, `security.get_service_credentials`, `security.get_settings`, `security.get_stats`, `security.get_token`, `security.get_user`, `security.get_user_privileges`, `security.get_user_profile`, `security.grant_api_key`, `security.has_privileges_user_profile`, `security.invalidate_token`, `security.oidc_authenticate`, `security.oidc_logout`, `security.oidc_prepare_authentication`, `security.put_privileges`, `security.put_role_mapping`, `security.put_user`, `security.query_user`, `security.saml_authenticate`, `security.saml_complete_logout`, `security.saml_invalidate`, `security.saml_logout`, `security.saml_prepare_authentication`, `security.saml_service_provider_metadata`, `security.suggest_user_profiles`, `security.update_cross_cluster_api_key`, `security.update_settings`, `security.update_user_profile_data`, `ssl.certificates`.


## `oxsearch-transform`

Tested: `transform.delete_transform`, `transform.get_node_stats`, `transform.get_transform`, `transform.get_transform_stats`, `transform.preview_transform`, `transform.put_transform`, `transform.reset_transform`, `transform.schedule_now_transform`, `transform.set_upgrade_mode`, `transform.start_transform`, `transform.stop_transform`, `transform.update_transform`, `transform.upgrade_transforms`.

Not on Elastic Cloud Serverless: `transform.set_upgrade_mode`, `transform.upgrade_transforms`.


## `oxsearch-rollup`

Tested: `rollup.delete_job`, `rollup.get_jobs`, `rollup.get_rollup_caps`, `rollup.get_rollup_index_caps`, `rollup.put_job`, `rollup.rollup_search`, `rollup.start_job`, `rollup.stop_job`.

Not on Elastic Cloud Serverless: `rollup.delete_job`, `rollup.get_jobs`, `rollup.get_rollup_caps`, `rollup.get_rollup_index_caps`, `rollup.put_job`, `rollup.rollup_search`, `rollup.start_job`, `rollup.stop_job`.


## `oxsearch-ccr`

Tested: `ccr.delete_auto_follow_pattern`, `ccr.follow`, `ccr.follow_info`, `ccr.follow_stats`, `ccr.forget_follower`, `ccr.get_auto_follow_pattern`, `ccr.pause_auto_follow_pattern`, `ccr.pause_follow`, `ccr.put_auto_follow_pattern`, `ccr.resume_auto_follow_pattern`, `ccr.resume_follow`, `ccr.stats`, `ccr.unfollow`.

Not on Elastic Cloud Serverless: `ccr.delete_auto_follow_pattern`, `ccr.follow`, `ccr.follow_info`, `ccr.follow_stats`, `ccr.forget_follower`, `ccr.get_auto_follow_pattern`, `ccr.pause_auto_follow_pattern`, `ccr.pause_follow`, `ccr.put_auto_follow_pattern`, `ccr.resume_auto_follow_pattern`, `ccr.resume_follow`, `ccr.stats`, `ccr.unfollow`.

Tested on the trial only. The fixture follows its own indices, through a remote cluster that points back at its node; following another cluster isn't tested.


## `oxsearch-searchable-snapshots`

Tested: `searchable_snapshots.cache_stats`, `searchable_snapshots.clear_cache`, `searchable_snapshots.mount`, `searchable_snapshots.stats`.

Not on Elastic Cloud Serverless: `searchable_snapshots.cache_stats`, `searchable_snapshots.clear_cache`, `searchable_snapshots.mount`, `searchable_snapshots.stats`.

Mounting, the mounted index's statistics and clearing its cache are tested on the trial only, and the mount as a full copy: the fixture's node has no shared cache, so its cache statistics count no regions.


## `oxsearch-dangling-indices`

Tested: `dangling_indices.delete_dangling_index`, `dangling_indices.import_dangling_index`, `dangling_indices.list_dangling_indices`.

Not on Elastic Cloud Serverless: `dangling_indices.delete_dangling_index`, `dangling_indices.import_dangling_index`, `dangling_indices.list_dangling_indices`.

A single node can't leave an index dangling, so the import and the delete are tested on a UUID no node has, which Elasticsearch refuses.


## `oxsearch-autoscaling`

Tested: `autoscaling.delete_autoscaling_policy`, `autoscaling.get_autoscaling_capacity`, `autoscaling.get_autoscaling_policy`, `autoscaling.put_autoscaling_policy`.

Not on Elastic Cloud Serverless: `autoscaling.delete_autoscaling_policy`, `autoscaling.get_autoscaling_capacity`, `autoscaling.get_autoscaling_policy`, `autoscaling.put_autoscaling_policy`.

Tested on the trial only.


## `oxsearch-shutdown`

Tested: `shutdown.delete_node`, `shutdown.get_node`, `shutdown.put_node`.

Not on Elastic Cloud Serverless: `shutdown.delete_node`, `shutdown.get_node`, `shutdown.put_node`.


## `oxsearch-migration`

Tested: `migration.deprecations`, `migration.get_feature_upgrade_status`, `migration.post_feature_upgrade`.

Not on Elastic Cloud Serverless: `migration.deprecations`, `migration.get_feature_upgrade_status`, `migration.post_feature_upgrade`.

A cluster on the current version has no system features to upgrade, so the upgrade asked for is declined.


## `oxsearch-features`

Tested: `features.get_features`, `features.reset_features`.

Not on Elastic Cloud Serverless: `features.get_features`, `features.reset_features`.


## `oxsearch-simulate`

Tested: `simulate.ingest`.

Not on Elastic Cloud Serverless: `simulate.ingest`.


## `oxsearch-text-structure`

Tested: `text_structure.find_field_structure`, `text_structure.find_message_structure`, `text_structure.find_structure`, `text_structure.test_grok_pattern`.

Not on Elastic Cloud Serverless: `text_structure.find_field_structure`, `text_structure.find_message_structure`, `text_structure.find_structure`, `text_structure.test_grok_pattern`.


## `oxsearch-connector`

Tested: `connector.check_in`, `connector.delete`, `connector.get`, `connector.last_sync`, `connector.list`, `connector.post`, `connector.put`, `connector.secret_delete`, `connector.secret_get`, `connector.secret_post`, `connector.secret_put`, `connector.sync_job_cancel`, `connector.sync_job_check_in`, `connector.sync_job_claim`, `connector.sync_job_delete`, `connector.sync_job_error`, `connector.sync_job_get`, `connector.sync_job_list`, `connector.sync_job_post`, `connector.sync_job_update_stats`, `connector.update_active_filtering`, `connector.update_api_key_id`, `connector.update_configuration`, `connector.update_error`, `connector.update_features`, `connector.update_filtering`, `connector.update_filtering_validation`, `connector.update_index_name`, `connector.update_name`, `connector.update_native`, `connector.update_pipeline`, `connector.update_scheduling`, `connector.update_service_type`, `connector.update_status`.

Not on Elastic Cloud Serverless: `connector.last_sync`, `connector.secret_delete`, `connector.secret_get`, `connector.secret_post`, `connector.secret_put`, `connector.sync_job_check_in`, `connector.sync_job_claim`, `connector.sync_job_error`, `connector.sync_job_update_stats`, `connector.update_features`.


## `oxsearch-fleet`

Tested: `fleet.delete_secret`, `fleet.get_secret`, `fleet.global_checkpoints`, `fleet.msearch`, `fleet.post_secret`, `fleet.search`.

Not on Elastic Cloud Serverless: `fleet.delete_secret`, `fleet.get_secret`, `fleet.global_checkpoints`, `fleet.msearch`, `fleet.post_secret`, `fleet.search`.

The specification gives the multi-search a `wait_for_checkpoints` URL parameter, which Elasticsearch refuses.


## `oxsearch-logstash`

Tested: `logstash.delete_pipeline`, `logstash.get_pipeline`, `logstash.put_pipeline`.


## `oxsearch-graph`

Tested: `graph.explore`.

Not on Elastic Cloud Serverless: `graph.explore`.

Tested on the trial only.


## `oxsearch-profiling`

Tested: `profiling.flamegraph`, `profiling.stacktraces`, `profiling.status`, `profiling.topn_functions`.

Not on Elastic Cloud Serverless: `profiling.flamegraph`, `profiling.stacktraces`, `profiling.status`, `profiling.topn_functions`.

The queries need a license above Basic and indices the fixture doesn't have, so they're refused on both licenses.


## `oxsearch-project`

Not tested yet: `project.create_many_routing`, `project.create_routing`, `project.delete_routing`, `project.get_many_routing`, `project.get_routing`, `project.tags`.

Only Elastic Cloud Serverless has these, so the fixture can't test them.


## `oxsearch-streams`

Tested: `streams.logs_disable`, `streams.logs_enable`, `streams.status`.

Newer than Elasticsearch 8.19: `streams.logs_disable`, `streams.logs_enable`, `streams.status`.

Not on Elastic Cloud Serverless: `streams.logs_disable`, `streams.logs_enable`, `streams.status`.

