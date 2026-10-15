# Compile time

DESIGN.md §2 says an application should compile only what it uses. The cost that matters is a clean build of what an application adding oxsearch compiles: the front crate's default features, which are search, docs, mapping and inference, and the crates they depend on. CI times that build on every change and fails when it goes over budget.

## Measured

These are clean dev builds of `oxsearch` with its default features, on the 9.5 specification (2026-10-15). The dependencies are fetched beforehand, so download time isn't counted.

| | M-series Mac, 14 cores | CI (`ubuntu-latest`, 4 cores) |
|---|--:|--:|
| Clean build | 13.8 s | 37 s |
| After a change to oxsearch-core | 3.1 s | |
| After a change to oxsearch-search | 0.7 s | |
| Adding `transport-reqwest` (reqwest with rustls) | +14.8 s | |

`cargo build --timings` gives each generated crate's share of the clean build, on the Mac:

| Crate | Time |
|---|--:|
| oxsearch-core | 4.2 s |
| oxsearch-inference | 2.6 s |
| oxsearch-search | 2.5 s |
| oxsearch-mapping | 2.2 s |
| oxsearch-analysis | 1.8 s |
| oxsearch-docs | 1.6 s |
| oxsearch-tasks | 0.2 s |

The rest of the build is serde, serde_json and indexmap.

`transport-reqwest` costs more than all the generated crates together. reqwest's `rustls` feature uses aws-lc-rs, which compiles C. That bears on DESIGN.md's open question 5, whether the transport is on by default, and on which TLS provider it uses.

## The budget

The CI job `compile-time` runs `cargo build -p oxsearch --timings` from a clean target directory. It fails if the build takes longer than `BUDGET_SECONDS`, and keeps `cargo-timing.html` as an artifact. The budget is 60 s, about 1.5 times the 37 s CI measured on 2026-10-15, which allows for the variance of shared runners. Raise it only on purpose, with the reason in the commit.
