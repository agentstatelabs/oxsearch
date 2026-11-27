# oxsearch-cli

`oxsearch`, a command for moving Elasticsearch documents in bulk: export an index to NDJSON, import NDJSON into an index, and transfer documents between indices or clusters. It's built on [oxsearch](https://github.com/agentstatelabs/oxsearch)'s point-in-time scan and bulk loader, and earns its place where reindex can't go: files, samples, sources a cluster can't reach.

```bash
cargo install oxsearch-cli
```

## Export

```bash
oxsearch export --url https://es.example:9200 --index notes --slices 4 -o notes.ndjson.gz
```

Each line is a record: the document's index, id and routing, and its source as the JSON it was, unchanged.

```json
{"_index":"notes","_id":"7","_routing":"ada","_source":{"title":"Tuning HNSW","views":41}}
```

- The index is read over a point in time, `--slices` slices at once, in pages of `--page-size`.
- A page that a shard couldn't answer is an error, not a page with documents missing.
- `--query` takes a query as JSON. `--every N` keeps one document in N, from the first, and `--limit N` keeps at most N.
- `--plain` writes the documents themselves, without their ids and routing.
- A name ending in `.gz`, or `--gzip`, compresses the output.
- An export never overwrites a file; `-o -`, the default, writes to standard output.

## Import

```bash
oxsearch import --url https://es.example:9200 -i notes.ndjson.gz
oxsearch import --index products --plain --id-field sku -i products.ndjson
```

- Records go back to the index, id and routing they came from, or into `--index`.
- With `--plain`, each line is a document, and `--id-field` names the field holding its id; without it, Elasticsearch gives each document one.
- Gzip input is recognized by its first bytes.
- Documents are written by the bulk loader, `--concurrency` requests of `--batch-size` at a time, with backoff when Elasticsearch is busy.
- A line that isn't a record (or, with `--plain`, a JSON object) stops the import at that line. What was read before it stays written.

## Transfer

```bash
OXSEARCH_TO_API_KEY=… oxsearch transfer --index notes --to-url https://other.example:9200
oxsearch transfer --index notes --to-index notes-v2 --every 10
```

Documents go with their ids and routing: to `--to-index`, or to indices of the same names in the other cluster. A transfer within one cluster needs `--to-index`. Create the destination index with its mapping first: otherwise Elasticsearch creates it with a dynamic one.

## What a run says

Every run ends with a line like `25 read, 0 skipped, 25 written, 0 refused` on standard error. Documents Elasticsearch refused are counted and the first ten are listed with the reason. The exit status is 1 if any document was refused or the run stopped, and 2 for arguments that don't make a run.

## Credentials and telemetry

Credentials come from the environment, never from arguments, so they stay out of process listings and shell history:

- `OXSEARCH_API_KEY` (encoded, as Kibana shows it), or `OXSEARCH_USERNAME` and `OXSEARCH_PASSWORD`;
- `OXSEARCH_TO_API_KEY`, `OXSEARCH_TO_USERNAME` and `OXSEARCH_TO_PASSWORD` for a transfer's destination.

`OXSEARCH_URL` and `OXSEARCH_TO_URL` give the clusters' URLs. Logs never show a password in a URL.

Telemetry goes through [oxtel](https://github.com/agentstatelabs/oxtel): progress is logged to standard error every ten seconds, and with `OTEL_EXPORTER_OTLP_ENDPOINT` set, the metrics go to an OpenTelemetry collector. They are `oxsearch.cli.documents` (by operation and result), and the scan's and bulk loader's own: pages, documents and bytes per second, batches in flight and actions queued.

## License

Apache-2.0. oxsearch is not affiliated with or endorsed by Elastic. Elasticsearch is a trademark of Elasticsearch B.V.
