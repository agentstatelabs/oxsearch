# oxsearch

A typed Rust client for Elasticsearch, generated from the Elasticsearch specification, with typed AI search: retrievers, `semantic`, `knn` and `sparse_vector` queries, `semantic_text` mappings and the inference API.

**Status: early.** Nothing is generated yet; the generator is being built (Phase 0 in [DESIGN.md](DESIGN.md)). The crates exist so the layout and CI can be built and checked; they contain no API.

## Design

[DESIGN.md](DESIGN.md) describes the architecture: the crates and why they're split the way they are, how the generator places types and maps the specification's shapes to serde, the escape hatches, the typed document layer and the telemetry.

## Development

```bash
cargo test --workspace
```

The generator reads the specification's `schema.json` from the commit pinned in [spec/pin.toml](spec/pin.toml), one Elasticsearch minor at a time. This downloads it, checks its SHA-256 and reads it:

```bash
cargo run -p xtask -- spec check
```

The integration tests run against Elasticsearch in Docker, on the Basic license or a trial license:

```bash
scripts/es-fixture.sh up trial
OXSEARCH_TEST_LICENSE=trial cargo test -p oxsearch-integration -- --ignored
scripts/es-fixture.sh down
```

## License

Apache-2.0. See [LICENSE](LICENSE) and [CONTRIBUTING.md](CONTRIBUTING.md); every commit is signed off under the Developer Certificate of Origin.

oxsearch is not affiliated with or endorsed by Elastic. Elasticsearch is a trademark of Elasticsearch B.V., used here only to say what oxsearch works with.
