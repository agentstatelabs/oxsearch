# Compile time

DESIGN.md §2 says an application should compile only what it uses. The cost that matters is a clean build of what an application adding oxsearch compiles: the front crate's default features, which are search, docs, mapping, inference and the HTTP transport, and the crates they depend on. CI times that build on every change and fails when it goes over budget.

## Measured

These are clean dev builds of `oxsearch` with the AI-search features, on the 9.5 specification (2026-10-15), before the transport was on by default. The dependencies are fetched beforehand, so download time isn't counted.

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

`transport-reqwest` costs more than all the generated crates together. reqwest's `rustls` feature uses aws-lc-rs, which compiles C.

### With the transport on by default

The transport has been a default feature since DESIGN.md's question 5 was decided (2026-10-29), so that the README's quick start builds as written; `default-features = false` leaves it out. Measured again on 2026-11-20, Rust 1.99, the 9.5 specification:

| | M-series Mac, 18 cores | CI (`ubuntu-latest`, 4 cores) |
|---|--:|--:|
| Clean build, AI-search features only | 20.6 s wall, 41 s of CPU | |
| Clean build, default features (with the transport) | 17.6 s wall, 80 s of CPU | to come |

On a machine with many cores the transport builds alongside the generated crates and costs no wall time; on CI's four it nearly doubles the work. aws-lc-sys's build script is 4 s of it; rustls, reqwest, hyper and tokio most of the rest.

## The budget

The CI job `compile-time` runs `cargo build -p oxsearch --timings` from a clean target directory, so it times the default features, transport included. It fails if the build takes longer than `BUDGET_SECONDS`, and keeps `cargo-timing.html` as an artifact. The budget is 120 s until CI has measured the build with the transport on by default, when it becomes about 1.5 times that figure, which allows for the variance of shared runners. It was 60 s, 1.5 times the 37 s CI measured on 2026-10-15 without the transport. Raise it only on purpose, with the reason in the commit.
