# Endpoint coverage

The endpoints generated in each crate, and those the integration tests send to Elasticsearch on the Basic license and a trial. Written by `cargo run -p xtask -- coverage` from the requests the tests record (`OXSEARCH_COVERAGE`); each CI integration job prints its own.

| Crate | Generated | Integration-tested |
|---|---:|---:|
| `oxsearch-core` | 2 | 1 (50%) |
| `oxsearch-tasks` | 3 | 0 (0%) |
| `oxsearch-inference` | 41 | 1 (2%) |
| `oxsearch-search` | 17 | 4 (23%) |
| `oxsearch-docs` | 21 | 5 (23%) |
| all | 84 | 11 (13%) |

## `oxsearch-core`

Tested: `info`.

Not tested yet: `ping`.

## `oxsearch-tasks`

Not tested yet: `tasks.cancel`, `tasks.get`, `tasks.list`.

## `oxsearch-inference`

Tested: `inference.get`.

Not tested yet: `inference.chat_completion_unified`, `inference.completion`, `inference.delete`, `inference.delete_region_policy`, `inference.embedding`, `inference.get_region_policy`, `inference.inference`, `inference.put`, `inference.put_ai21`, `inference.put_alibabacloud`, `inference.put_amazonbedrock`, `inference.put_amazonsagemaker`, `inference.put_anthropic`, `inference.put_azureaistudio`, `inference.put_azureopenai`, `inference.put_cohere`, `inference.put_contextualai`, `inference.put_custom`, `inference.put_deepseek`, `inference.put_elasticsearch`, `inference.put_elser`, `inference.put_fireworksai`, `inference.put_googleaistudio`, `inference.put_googlevertexai`, `inference.put_groq`, `inference.put_hugging_face`, `inference.put_jinaai`, `inference.put_llama`, `inference.put_mistral`, `inference.put_nvidia`, `inference.put_openai`, `inference.put_openshift_ai`, `inference.put_region_policy`, `inference.put_voyageai`, `inference.put_watsonx`, `inference.rerank`, `inference.sparse_embedding`, `inference.stream_completion`, `inference.text_embedding`, `inference.update`.

## `oxsearch-search`

Tested: `close_point_in_time`, `msearch`, `open_point_in_time`, `search`.

Not tested yet: `clear_scroll`, `count`, `explain`, `field_caps`, `knn_search`, `msearch_template`, `rank_eval`, `render_search_template`, `scroll`, `search_mvt`, `search_shards`, `search_template`, `terms_enum`.

## `oxsearch-docs`

Tested: `bulk`, `delete`, `get`, `index`, `mget`.

Not tested yet: `cancel_reindex`, `create`, `delete_by_query`, `delete_by_query_rethrottle`, `exists`, `exists_source`, `get_reindex`, `get_source`, `list_reindex`, `mtermvectors`, `reindex`, `reindex_rethrottle`, `termvectors`, `update`, `update_by_query`, `update_by_query_rethrottle`.

## APIs not generated yet

The tests also call these through the escape hatches: `indices.create`, `indices.delete`, `indices.get_mapping`, `indices.put_mapping`, `indices.refresh`, `license.get`, `query_rules.delete_ruleset`, `query_rules.put_ruleset`.
