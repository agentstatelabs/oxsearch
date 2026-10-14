//! Development tasks:
//!
//! - `cargo run -p xtask -- spec check` gets the specification's
//!   `schema.json` at the commit pinned in `spec/pin.toml`, checks its
//!   SHA-256, reads it with the generator's model, and checks the test
//!   fixture runs the same Elasticsearch minor. The download is kept under
//!   `target/`, and reused while its checksum matches.
//! - `cargo run -p xtask -- spec fetch` only gets and checks the schema, and
//!   prints where it is.
//! - `cargo run -p xtask -- spec pin <branch> [<commit>]` moves the pin to a
//!   commit on a specification branch (its head by default), once the model
//!   reads that commit's schema, and records the new checksum.
//! - `cargo run -p xtask -- placement` places every type in a crate by the
//!   rules in `placement.rs`, and writes the report to `spec/placement.md`.
//! - `cargo run -p xtask -- shapes` counts the shapes the generator maps to
//!   serde, and writes the report to `spec/shapes.md`.

use std::process::ExitCode;

use xtask::graph::Graph;
use xtask::spec::{self, Pin};
use xtask::{Error, placement, workspace_root};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (name, result) = match args.as_slice() {
        ["spec", "check"] => ("spec check", spec_check()),
        ["spec", "fetch"] => ("spec fetch", spec_fetch()),
        ["spec", "pin", branch] => ("spec pin", spec_pin(branch, None)),
        ["spec", "pin", branch, commit] => ("spec pin", spec_pin(branch, Some(commit))),
        ["placement"] => ("placement", placement()),
        ["shapes"] => ("shapes", shapes()),
        _ => {
            eprintln!(
                "usage: cargo run -p xtask -- spec check | spec fetch | spec pin <branch> [<commit>] | placement | shapes"
            );
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{name} failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn spec_check() -> Result<(), Error> {
    let pin = Pin::read()?;
    let bytes = spec::fetch(&pin)?;
    let schema = spec::load(&bytes)?;
    let fixture = spec::check_fixture(&pin)?;
    println!(
        "{} {} at {}: schema.json ({} bytes) matches its SHA-256 and fits the model",
        pin.repository,
        pin.branch,
        pin.commit,
        bytes.len()
    );
    println!("{}", spec::summary(&schema));
    println!("the fixture runs Elasticsearch {fixture}");
    Ok(())
}

fn spec_fetch() -> Result<(), Error> {
    let pin = Pin::read()?;
    spec::fetch(&pin)?;
    println!("{}", pin.cache_path().display());
    Ok(())
}

fn spec_pin(branch: &str, commit: Option<&str>) -> Result<(), Error> {
    let pin = spec::repin(branch, commit)?;
    println!(
        "pinned {} {} at {} (schema.json SHA-256 {})",
        pin.repository, pin.branch, pin.commit, pin.schema_sha256
    );
    if let Err(e) = spec::check_fixture(&pin) {
        println!("now update the fixture: {e}");
    }
    Ok(())
}

fn placement() -> Result<(), Error> {
    let pin = Pin::read()?;
    let schema = spec::load(&spec::fetch(&pin)?)?;
    let graph = Graph::new(&schema)?;
    let placement = placement::place(&graph)?;
    let report = workspace_root().join(placement::REPORT_FILE);
    std::fs::write(&report, placement.report(&pin))?;
    println!(
        "{} types placed; {} moved, {} references cut by seams; written to {}",
        graph.types.len(),
        placement.moves.len(),
        placement.cuts.len(),
        placement::REPORT_FILE
    );
    Ok(())
}

fn shapes() -> Result<(), Error> {
    let pin = Pin::read()?;
    let schema = spec::load(&spec::fetch(&pin)?)?;
    let graph = Graph::new(&schema)?;
    let placement = placement::place(&graph)?;
    let inventory = xtask::shapes::inventory(&graph, &placement);
    let report = workspace_root().join(xtask::shapes::REPORT_FILE);
    std::fs::write(&report, inventory.report(&pin))?;
    println!(
        "{} shapes, {} unions; written to {}",
        inventory.shapes.len(),
        inventory.unions.len(),
        xtask::shapes::REPORT_FILE
    );
    Ok(())
}
