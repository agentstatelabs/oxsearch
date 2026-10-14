# Contributing to oxsearch

Thank you for helping. Everyone taking part follows the [Code of Conduct](CODE_OF_CONDUCT.md). oxsearch is early, so the most useful contributions right now are issues: use cases, specification questions, and problems with the [design](DESIGN.md).

## Sign-off (DCO)

Every commit must be signed off under the [Developer Certificate of Origin 1.1](https://developercertificate.org/), which states that you have the right to submit the change under the project's license. Sign off with:

```bash
git commit -s
```

This adds a `Signed-off-by:` line with your name and email. There is no contributor license agreement.

## Code you contribute

- Write it for this project. Don't copy code from other projects, including other Elasticsearch clients, even when their license would allow it. The only other source of code here is the Apache-2.0 [Elasticsearch specification](https://github.com/elastic/elasticsearch-specification), which the generator reads. This keeps the project's provenance clean.
- Generated code is never edited by hand. Change the generator, or the specification upstream.
- No `unsafe` code.
- Run `cargo fmt`, `cargo clippy --all-targets` and `cargo test` before opening a pull request. Changes that touch requests or responses also need the integration tests (see the [README](README.md#development)).

## Reporting security issues

See [SECURITY.md](SECURITY.md). Please don't open public issues for vulnerabilities.
