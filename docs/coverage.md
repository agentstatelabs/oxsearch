# Endpoint coverage

The endpoints generated in each crate, and those the integration tests send to Elasticsearch on the Basic license and a trial. Written by `cargo run -p xtask -- coverage` from the requests the tests record (`OXSEARCH_COVERAGE`); each CI integration job prints its own.

| Crate | Generated | Integration-tested |
|---|---:|---:|
| `oxsearch-core` | 2 | 2 (100%) |
| `oxsearch-tasks` | 3 | 3 (100%) |
| `oxsearch-inference` | 41 | 6 (14%) |
| `oxsearch-search` | 17 | 15 (88%) |
| `oxsearch-docs` | 21 | 21 (100%) |
| all | 84 | 47 (55%) |

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


## APIs not generated yet

The tests also call these through the escape hatches: `indices.create`, `indices.delete`, `indices.get_mapping`, `indices.put_mapping`, `indices.refresh`, `license.get`, `query_rules.delete_ruleset`, `query_rules.put_ruleset`.
