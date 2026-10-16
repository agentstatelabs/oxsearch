//! Round-trip tests from the specification's own request and response
//! examples: each example is read into its generated type and written back.
//!
//! Each generated crate gets `tests/examples.rs`, which records every
//! example's outcome and compares the record with `tests/examples.expected`:
//! `exact` (written back as it was read), `normalized` (written back in
//! another form, such as a shorthand expanded, and read again to the same
//! value), or `fails` with the reason. A changed outcome fails the test;
//! `OXSEARCH_BLESS=1` writes the new record.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use super::emit::outside;
use super::ir::{Ir, ItemKind, Ty};
use crate::spec::Pin;
use crate::spec::model::{Endpoint, TypeDefinition};

/// A request body's aggregations, as JSON text, if it has any.
fn request_aggregations(body: &str) -> Option<String> {
    let body: serde_json::Value = serde_json::from_str(body).ok()?;
    let aggregations = body.get("aggregations").or_else(|| body.get("aggs"))?;
    aggregations.is_object().then(|| aggregations.to_string())
}

/// The test files, by path relative to the workspace root.
pub fn tests(
    ir: &Ir<'_>,
    endpoints: &[Endpoint],
    crates: &[&str],
    pin: &Pin,
) -> BTreeMap<PathBuf, String> {
    let graph = ir.graph;
    let mut by_crate: BTreeMap<&str, Vec<(String, String, String, bool)>> = BTreeMap::new();
    // Every request's aggregations, generated endpoint or not, are checked
    // in the aggregations crate as its own types: requests read them as
    // JSON, through the seam. Responses' can't be: their examples are
    // written without `typed_keys`, which names each result's type.
    if crates.contains(&"aggs") {
        for (i, definition) in graph.types.iter().enumerate() {
            let TypeDefinition::Request(t) = definition else {
                continue;
            };
            for (example_name, example) in &t.examples {
                let Some(aggregations) = example.value.as_deref().and_then(request_aggregations)
                else {
                    continue;
                };
                by_crate.entry("aggs").or_default().push((
                    format!("{} request {example_name} aggregations", graph.name(i).namespace),
                    "::indexmap::IndexMap<::std::string::String, ::oxsearch_aggs::AggregationContainer>"
                        .to_owned(),
                    aggregations,
                    false,
                ));
            }
        }
    }
    for (i, definition) in graph.types.iter().enumerate() {
        let Some(&item) = ir.item_of.get(&i) else {
            continue;
        };
        let krate = ir.items[item].krate.as_str();
        let Some(krate) = crates.iter().find(|c| **c == krate) else {
            continue;
        };
        let name = graph.name(i);
        let endpoint = endpoints
            .iter()
            .find(|e| e.request.as_ref() == Some(name) || e.response.as_ref() == Some(name));
        if endpoint.is_some_and(|e| e.codegen_exclude) {
            continue;
        }
        let ndjson = endpoint.is_some_and(|e| {
            e.request_media_type
                .iter()
                .any(|m| m == "application/x-ndjson")
        });
        let json = |types: &[String]| {
            types.is_empty()
                || types
                    .iter()
                    .any(|m| m == "application/json" || m == "application/x-ndjson")
        };
        let json_request = endpoint.is_none_or(|e| json(&e.request_media_type));
        let json_response = endpoint.is_none_or(|e| json(&e.response_media_type));
        let (examples, ty, is_request) = match definition {
            TypeDefinition::Request(_) if !json_request => continue,
            TypeDefinition::Response(_) if !json_response => continue,
            TypeDefinition::Request(t) => {
                let ItemKind::Request(request) = &ir.items[item].kind else {
                    continue;
                };
                let Some(body) = &request.body else {
                    continue;
                };
                (&t.examples, outside(ir, &concrete(body)), true)
            }
            TypeDefinition::Response(t) => (
                &t.examples,
                outside(
                    ir,
                    &Ty::Named {
                        target: i,
                        args: Vec::new(),
                    },
                ),
                false,
            ),
            _ => continue,
        };
        for (example_name, example) in examples {
            let Some(value) = &example.value else {
                continue;
            };
            if example
                .response_code
                .is_some_and(|code| !(200..300).contains(&code))
            {
                continue;
            }

            let label = format!(
                "{} {} {example_name}",
                name.namespace,
                if is_request { "request" } else { "response" }
            );
            by_crate.entry(krate).or_default().push((
                label,
                ty.clone(),
                value.clone(),
                is_request && ndjson,
            ));
        }
    }

    let mut files = BTreeMap::new();
    for (krate, mut examples) in by_crate {
        examples.sort();
        let mut source = format!(
            "// Generated by `cargo run -p xtask -- generate` from {} {} at\n// {}. Don't edit it: change the generator.\n\n",
            pin.repository, pin.branch, pin.commit
        );
        source.push_str(HARNESS);
        source.push_str("\nfn examples() -> ::std::vec::Vec<Example> {\n    vec![\n");
        for (label, ty, value, ndjson) in &examples {
            let check = if *ndjson { "check_ndjson" } else { "check" };
            let _ = writeln!(
                source,
                "        ({}, {check}::<{ty}>, {}),",
                rust_string(label),
                raw_string(value)
            );
        }
        source.push_str("    ]\n}\n");
        files.insert(
            PathBuf::from(format!("crates/oxsearch-{krate}/tests/examples.rs")),
            source,
        );
    }
    files
}

/// A type with its generic parameters as JSON, as the examples use them.
fn concrete(ty: &Ty) -> Ty {
    let boxed = |t: &Ty| Box::new(concrete(t));
    match ty {
        Ty::Param(_) => Ty::Json,
        Ty::Named { target, args } => Ty::Named {
            target: *target,
            args: args.iter().map(concrete).collect(),
        },
        Ty::Inline { item, args } => Ty::Inline {
            item: *item,
            args: args.iter().map(concrete).collect(),
        },
        Ty::Vec(t) => Ty::Vec(boxed(t)),
        Ty::Map(k, v) => Ty::Map(boxed(k), boxed(v)),
        Ty::SingleKey(k, v) => Ty::SingleKey(boxed(k), boxed(v)),
        Ty::OneOrMany(t) => Ty::OneOrMany(boxed(t)),
        Ty::Stringified(t) => Ty::Stringified(boxed(t)),
        Ty::WithNull(t) => Ty::WithNull(boxed(t)),
        Ty::Flags(t) => Ty::Flags(boxed(t)),
        Ty::Opt(t) => Ty::Opt(boxed(t)),
        Ty::TypedKeys(t) => Ty::TypedKeys(boxed(t)),
        Ty::Prim(_) | Ty::Unit | Ty::Json => ty.clone(),
    }
}

/// A raw string literal holding `text`, with enough `#`s.
fn raw_string(text: &str) -> String {
    let mut hashes = 1;
    while text.contains(&format!("\"{}", "#".repeat(hashes))) {
        hashes += 1;
    }
    let fence = "#".repeat(hashes);
    format!("r{fence}\"{text}\"{fence}")
}

fn rust_string(text: &str) -> String {
    format!("{text:?}")
}

/// The test itself, the same in every crate.
const HARNESS: &str = r#"//! Every example the specification gives for this crate's requests and
//! responses, read into its type and written back. The outcomes are in
//! `tests/examples.expected`; run with `OXSEARCH_BLESS=1` to record new
//! ones.

// Not every crate has NDJSON examples.
#![allow(dead_code)]

type Check = fn(&str) -> ::core::result::Result<Outcome, ::std::string::String>;
type Example = (&'static str, Check, &'static str);

enum Outcome {
    /// Written back exactly as read.
    Exact,
    /// Written back in another form that reads to the same value; the
    /// first place the forms differ.
    Normalized(::std::string::String),
}

fn check<T>(text: &str) -> ::core::result::Result<Outcome, ::std::string::String>
where
    T: ::serde::Serialize + ::serde::de::DeserializeOwned + ::core::cmp::PartialEq + ::core::fmt::Debug,
{
    let input: ::serde_json::Value = ::serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    round_trip::<T>(input)
}

/// For NDJSON bodies: each line is one element of the body's array.
fn check_ndjson<T>(text: &str) -> ::core::result::Result<Outcome, ::std::string::String>
where
    T: ::serde::Serialize + ::serde::de::DeserializeOwned + ::core::cmp::PartialEq + ::core::fmt::Debug,
{
    let lines = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(::serde_json::from_str)
        .collect::<::core::result::Result<::std::vec::Vec<::serde_json::Value>, _>>()
        .map_err(|e| format!("not NDJSON: {e}"))?;
    round_trip::<T>(::serde_json::Value::Array(lines))
}

/// The value with every object's keys in sorted order, so what's read first,
/// and so which error is reported, doesn't depend on whether serde_json
/// keeps keys in order (its `preserve_order` feature).
fn sorted(value: ::serde_json::Value) -> ::serde_json::Value {
    use ::serde_json::Value;
    match value {
        Value::Object(map) => {
            let mut entries: ::std::vec::Vec<(::std::string::String, Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(entries.into_iter().map(|(key, value)| (key, sorted(value))).collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

fn round_trip<T>(input: ::serde_json::Value) -> ::core::result::Result<Outcome, ::std::string::String>
where
    T: ::serde::Serialize + ::serde::de::DeserializeOwned + ::core::cmp::PartialEq + ::core::fmt::Debug,
{
    let input = sorted(input);
    let read: T = ::serde_path_to_error::deserialize(input.clone()).map_err(|e| format!("reading {}: {}", e.path(), e.inner()))?;
    let output = ::serde_json::to_value(&read).map_err(|e| format!("writing: {e}"))?;
    if output == input {
        return Ok(Outcome::Exact);
    }
    let again: T = ::serde_json::from_value(output.clone()).map_err(|e| format!("reading back: {e}"))?;
    if again != read {
        return Err(format!("reads back differently at {}", first_difference(&input, &output, "")));
    }
    Ok(Outcome::Normalized(first_difference(&input, &output, "")))
}

/// The JSON pointer of the first place two values differ, taking keys in
/// sorted order, whatever order serde_json keeps them in.
fn first_difference(a: &::serde_json::Value, b: &::serde_json::Value, at: &str) -> ::std::string::String {
    use ::serde_json::Value;
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            let mut keys: ::std::vec::Vec<&::std::string::String> = a.keys().collect();
            keys.sort();
            for key in keys {
                let value = &a[key];
                match b.get(key) {
                    Some(other) if other == value => {}
                    Some(other) => return first_difference(value, other, &format!("{at}/{key}")),
                    None => return format!("{at}/{key} (dropped)"),
                }
            }
            let mut added: ::std::vec::Vec<&::std::string::String> =
                b.keys().filter(|key| !a.contains_key(*key)).collect();
            added.sort();
            if let Some(key) = added.first() {
                return format!("{at}/{key} (added)");
            }
            at.to_owned()
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (n, (x, y)) in a.iter().zip(b).enumerate() {
                if x != y {
                    return first_difference(x, y, &format!("{at}/{n}"));
                }
            }
            at.to_owned()
        }
        _ => if at.is_empty() { "/".to_owned() } else { at.to_owned() },
    }
}

#[test]
fn examples_round_trip() {
    let mut record = ::std::string::String::new();
    let mut counts = [0usize; 3];
    for (name, check, text) in examples() {
        let outcome = match check(text) {
            Ok(Outcome::Exact) => {
                counts[0] += 1;
                "exact".to_owned()
            }
            Ok(Outcome::Normalized(at)) => {
                counts[1] += 1;
                format!("normalized at {at}")
            }
            Err(reason) => {
                counts[2] += 1;
                let reason: ::std::string::String = reason.chars().take(160).collect();
                format!("fails: {reason}")
            }
        };
        record.push_str(&format!("{name}: {outcome}\n"));
    }
    let summary = format!(
        "{} exact, {} normalized, {} failing\n",
        counts[0], counts[1], counts[2]
    );
    let record = format!("{summary}{record}");
    let path = ::std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/examples.expected");
    if ::std::env::var_os("OXSEARCH_BLESS").is_some() {
        ::std::fs::write(&path, &record).unwrap();
        return;
    }
    let expected = ::std::fs::read_to_string(&path).unwrap_or_default();
    if expected != record {
        let old: ::std::collections::BTreeSet<&str> = expected.lines().collect();
        let new: ::std::collections::BTreeSet<&str> = record.lines().collect();
        let mut diff = ::std::string::String::new();
        for line in old.difference(&new) {
            diff.push_str(&format!("- {line}\n"));
        }
        for line in new.difference(&old) {
            diff.push_str(&format!("+ {line}\n"));
        }
        panic!("the examples' outcomes changed (OXSEARCH_BLESS=1 records them):\n{diff}");
    }
}
"#;
