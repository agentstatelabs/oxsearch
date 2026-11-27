//! The code generator: Rust types for the specification's types, written
//! into the crates placement assigns them to.

mod emit;
mod examples;
pub mod ir;
mod names;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::Error;
use crate::graph::Graph;
use crate::placement::{self, Placement};
use crate::spec::Pin;
use crate::spec::model::{Availability, Endpoint, Stability, Visibility};

/// The crates generated so far: the first slice (DESIGN.md §8) and the
/// foundation crates it depends on.
pub const CRATES: &[&str] = &[
    "core",
    "analysis",
    "mapping",
    "tasks",
    "inference",
    "search",
    "docs",
    "aggs",
    "admin",
    "esql",
    "ingest",
    "license",
    "query_rules",
    "synonyms",
    "snapshot",
    "ilm",
    "async_search",
    "eql",
    "sql",
    "search_application",
    "enrich",
    "ml",
    "watcher",
    "slm",
    "xpack",
    "cat",
    "security",
    "transform",
    "rollup",
    "ccr",
    "searchable_snapshots",
    "dangling_indices",
    "autoscaling",
    "shutdown",
    "migration",
    "features",
    "simulate",
    "text_structure",
    "connector",
    "fleet",
    "logstash",
    "graph",
    "profiling",
    "project",
    "streams",
];

/// An area's package, and its directory under `crates`: `query_rules` is
/// `oxsearch-query-rules`.
pub fn package(krate: &str) -> String {
    format!("oxsearch-{}", krate.replace('_', "-"))
}

/// The directory generated code goes in, inside each crate's `src`.
pub const DIR: &str = "generated";

/// Lints the generated code doesn't meet: it follows the specification's
/// names and documentation, not Rust's style.
const ALLOW: &str = "#![allow(missing_docs, non_camel_case_types, non_snake_case, deprecated, dead_code, unused_imports, clippy::all, rustdoc::all)]";

/// What the generator wrote.
pub struct Generated {
    /// Every file, by its path relative to the workspace root: the types,
    /// and round-trip tests from the specification's examples.
    pub files: BTreeMap<PathBuf, String>,
    /// The endpoint modules behind a feature, as `crate::module (feature)`.
    pub gated: Vec<String>,
    /// Beta or experimental endpoints left ungated, and why.
    pub ungated: Vec<String>,
}

/// The core's constants for the specification's version: its minor and its
/// major.
fn versions(pin: &Pin) -> Result<String, Error> {
    let major: u8 = pin
        .branch
        .split('.')
        .next()
        .and_then(|major| major.parse().ok())
        .ok_or_else(|| format!("the pinned branch {} isn't a version", pin.branch))?;
    Ok(format!(
        "/// The Elasticsearch minor this code is generated for: the specification's\n\
         /// branch.\n\
         pub const SPECIFICATION_VERSION: &str = \"{}\";\n\n\
         /// Its major version, which a transport can ask a newer server to\n\
         /// answer as (`compatible-with`).\n\
         pub const MAJOR_VERSION: u8 = {major};\n\n",
        pin.branch
    ))
}

/// Adds each endpoint's availability, and a link to its documentation, to
/// its request's docs.
fn annotate(ir: &mut ir::Ir<'_>, endpoints: &[Endpoint]) {
    for endpoint in endpoints {
        let Some(&item) = endpoint
            .request
            .as_ref()
            .and_then(|r| ir.graph.find(r))
            .and_then(|t| ir.item_of.get(&t))
        else {
            continue;
        };
        let mut note = format!("**Availability:** {}", availability(endpoint));
        // The specification marks links it doesn't have yet with `TODO`.
        if let Some(url) = endpoint
            .doc_url
            .as_ref()
            .filter(|u| !u.is_empty() && !u.contains("TODO"))
        {
            note.push_str(&format!("\n\n[Elasticsearch's documentation]({url})"));
        }
        let doc = &mut ir.items[item].doc;
        *doc = Some(match doc.take() {
            Some(doc) if !doc.trim().is_empty() => format!("{}\n\n{note}", doc.trim_end()),
            _ => note,
        });
    }
}

/// Where an endpoint is: since which Elasticsearch version, and whether on
/// Elastic Cloud Serverless.
fn availability(endpoint: &Endpoint) -> String {
    let public = |a: &Availability| !matches!(a.visibility, Some(Visibility::Private));
    let serverless = endpoint.availability.serverless.as_ref();
    let Some(stack) = endpoint.availability.stack.as_ref() else {
        return "Elastic Cloud Serverless only; a self-managed cluster refuses it.".to_owned();
    };
    let since = stack.since.as_deref();
    let mut text = match since {
        Some(since) => format!("Elasticsearch {since} and later"),
        None => "Elasticsearch".to_owned(),
    };
    text.push_str(match (serverless, since) {
        (Some(s), Some(_)) if public(s) => ", and Elastic Cloud Serverless.",
        (Some(s), None) if public(s) => " and Elastic Cloud Serverless.",
        (Some(_), _) => "; on Elastic Cloud Serverless, for Elastic's own use only.",
        (None, _) => "; not on Elastic Cloud Serverless.",
    });
    if !public(stack) {
        text.push_str(" Elasticsearch meant it for its own components, not for direct use.");
    }
    text
}

/// Generates the crates in [`CRATES`].
pub fn generate(
    graph: &Graph<'_>,
    placement: &Placement<'_>,
    endpoints: &[Endpoint],
    pin: &Pin,
) -> Result<Generated, Error> {
    let text = endpoints
        .iter()
        .filter(|e| text_response(e))
        .filter_map(|e| e.response.as_ref().and_then(|r| graph.find(r)))
        .collect();
    let mut ir = ir::Ir::build(graph, placement, CRATES, text)?;
    annotate(&mut ir, endpoints);
    check_reserved(&ir)?;
    let mut modules = emit::emit(&ir);
    for endpoint in endpoints.iter().filter(|e| !e.codegen_exclude) {
        let Some(request) = endpoint.request.as_ref().and_then(|r| graph.find(r)) else {
            continue;
        };
        let Some(&item) = ir.item_of.get(&request) else {
            continue;
        };
        let response = endpoint
            .response
            .as_ref()
            .and_then(|r| graph.find(r))
            .filter(|r| ir.item_of.contains_key(r));
        let source = emit::endpoint(&ir, item, response, endpoint);
        let key = (ir.items[item].krate.clone(), ir.items[item].module.clone());
        modules.entry(key).or_default().push_str(&source);
    }
    let (gates, ungated) = gates(&ir, endpoints);
    let homes = homes(&ir);
    for ((krate, module), source) in moved(&ir, &homes, &gates)? {
        modules
            .entry((krate, module))
            .or_default()
            .push_str(&source);
    }
    let header = format!(
        "// Generated by `cargo run -p xtask -- generate` from {} {} at\n// {}. Don't edit it: change the generator.\n\n",
        pin.repository, pin.branch, pin.commit
    );
    let mut files = BTreeMap::new();
    for krate in CRATES {
        let mut paths: BTreeSet<Vec<String>> = BTreeSet::new();
        paths.insert(Vec::new());
        for (k, module) in modules.keys() {
            if k == krate {
                for n in 0..=module.len() {
                    paths.insert(module[..n].to_vec());
                }
            }
        }
        for path in &paths {
            let children: Vec<&String> = paths
                .iter()
                .filter(|p| p.len() == path.len() + 1 && p.starts_with(path))
                .map(|p| p.last().expect("a child has a name"))
                .collect();
            let mut file = PathBuf::from("crates")
                .join(package(krate))
                .join("src")
                .join(DIR);
            for part in path {
                file.push(names::bare(part));
            }
            let file = if path.is_empty() || !children.is_empty() {
                file.join("mod.rs")
            } else {
                file.with_extension("rs")
            };
            let mut source = header.clone();
            if path.is_empty() {
                source.push_str(ALLOW);
                source.push_str("\n\n");
                if *krate == "core" {
                    source.push_str(&versions(pin)?);
                }
            }
            for child in &children {
                let mut child_path = path.clone();
                child_path.push((*child).clone());
                if let Some(feature) = gates.get(&(krate.to_string(), child_path)) {
                    source.push_str(&format!(
                        "/// An endpoint Elasticsearch marks {feature}: it may change or go away in any\n/// release. Behind the `{feature}` feature.\n#[cfg(feature = \"{feature}\")]\n"
                    ));
                }
                source.push_str(&format!("pub mod {child};\n"));
            }
            if !children.is_empty() {
                source.push('\n');
            }
            if let Some(items) = modules.get(&(krate.to_string(), path.clone())) {
                source.push_str(items);
            }
            files.insert(file, source);
        }
    }
    files.insert(
        PathBuf::from("crates/oxsearch/src/generated.rs"),
        format!("{header}{}", front(&ir, &homes)),
    );
    files.extend(examples::tests(&ir, endpoints, CRATES, pin));
    let gated = gates
        .iter()
        .map(|((krate, module), feature)| format!("{krate}::{} ({feature})", module.join("::")))
        .collect();
    Ok(Generated {
        files,
        gated,
        ungated,
    })
}

/// Where each module's specification types belong: the home crate their
/// namespace gives them, and the module there, by the module placement
/// put them in.
type Homes = BTreeMap<(String, Vec<String>), BTreeSet<(String, Vec<String>)>>;

fn homes(ir: &ir::Ir<'_>) -> Homes {
    let mut homes = Homes::new();
    for (&ty, &id) in &ir.item_of {
        let item = &ir.items[id];
        let namespace = &ir.graph.name(ty).namespace;
        let home = placement::home(namespace);
        homes
            .entry((item.krate.clone(), item.module.clone()))
            .or_default()
            .insert((home.to_owned(), ir::module_path(namespace, home)));
    }
    homes
}

/// The types placement moved out of their home crate, re-exported there,
/// where their namespace puts them: `oxsearch_search::search::Highlight` is
/// `oxsearch_core::search::Highlight`. A module is re-exported whole, with
/// the types generated for its types' inline parts.
fn moved(
    ir: &ir::Ir<'_>,
    homes: &Homes,
    gates: &BTreeMap<(String, Vec<String>), &'static str>,
) -> Result<BTreeMap<(String, Vec<String>), String>, Error> {
    let mut out: BTreeMap<(String, Vec<String>), String> = BTreeMap::new();
    for ((krate, module), targets) in homes {
        let away: Vec<_> = targets.iter().filter(|(home, _)| home != krate).collect();
        let [(home, path)] = away.as_slice() else {
            if away.is_empty() {
                continue;
            }
            return Err(format!(
                "{}'s module `{}` holds types from several crates: {away:?}",
                package(krate),
                module.join("::")
            )
            .into());
        };
        if targets.len() > 1 {
            return Err(format!(
                "{}'s module `{}` holds its own types and {}'s",
                package(krate),
                module.join("::"),
                package(home)
            )
            .into());
        }
        if !CRATES.contains(&home.as_str()) {
            // Not generated yet: the area's crate re-exports them once it is.
            continue;
        }
        if gates
            .keys()
            .any(|(k, gated)| k == krate && module.starts_with(gated))
        {
            return Err(format!(
                "{}'s module `{}` is behind a feature",
                package(krate),
                module.join("::")
            )
            .into());
        }
        let names: Vec<&str> = ir
            .items
            .iter()
            .filter(|item| &item.krate == krate && &item.module == module)
            .map(|item| item.ident.as_str())
            .collect();
        for name in &names {
            if ir
                .items
                .iter()
                .any(|item| &item.krate == home && &item.module == path && item.ident == *name)
            {
                return Err(format!(
                    "{} has its own {name} where {}'s would be re-exported",
                    package(home),
                    package(krate)
                )
                .into());
            }
        }
        let mut from = format!("::oxsearch_{krate}");
        for part in module {
            from.push_str("::");
            from.push_str(part);
        }
        out.entry((home.clone(), path.clone()))
            .or_default()
            .push_str(&format!(
                "/// Types placement moved down to `{}`, which uses them.\npub use {from}::{{{}}};\n\n",
                package(krate),
                names.join(", ")
            ));
    }
    Ok(out)
}

/// The front crate's re-exports of oxsearch-core's generated types: those
/// at its root, and its modules whose types are its own.
fn front(ir: &ir::Ir<'_>, homes: &Homes) -> String {
    let root: Vec<&str> = ir
        .items
        .iter()
        .filter(|item| item.krate == "core" && item.module.is_empty())
        .map(|item| item.ident.as_str())
        .collect();
    let mut own: BTreeSet<&str> = BTreeSet::new();
    let mut away: BTreeSet<&str> = BTreeSet::new();
    for ((krate, module), targets) in homes {
        let Some(top) = module.first() else { continue };
        if krate != "core" {
            continue;
        }
        if targets.iter().all(|(home, _)| home == "core") {
            own.insert(top);
        } else {
            away.insert(top);
        }
    }
    let modules: Vec<&str> = own.difference(&away).copied().collect();
    format!(
        "//! oxsearch-core's generated types, re-exported at the root: the common\n//! types, retrievers and kNN, and the modules of oxsearch-core's own\n//! namespaces. Other namespaces' types are in their area's module, such as\n//! `search`.\n\npub use ::oxsearch_core::{{{}}};\n\npub use ::oxsearch_core::{{{}}};\n",
        root.join(", "),
        modules.join(", ")
    )
}

/// Names oxsearch-core declares at its root, which generated types there
/// would clash with.
const CORE_NAMES: &[&str] = &[
    "AggsBody",
    "auth",
    "Body",
    "Client",
    "ElasticsearchError",
    "Endpoint",
    "Error",
    "HttpRequest",
    "HttpResponse",
    "LicenseError",
    "Method",
    "Query",
    "RawAggs",
    "Retriever",
    "Route",
    "Transport",
    "TransportError",
    "shapes",
    "telemetry",
    "__private",
];

fn check_reserved(ir: &ir::Ir<'_>) -> Result<(), Error> {
    for item in &ir.items {
        if item.krate == "core"
            && item.module.is_empty()
            && CORE_NAMES.contains(&item.ident.as_str())
        {
            return Err(format!(
                "the generated type {} would clash with oxsearch-core's own",
                item.ident
            )
            .into());
        }
    }
    Ok(())
}

/// Whether an endpoint answers in text only, rather than JSON: plain text,
/// such as `nodes.hot_threads`, or server-sent events.
pub fn text_response(endpoint: &Endpoint) -> bool {
    let types = &endpoint.response_media_type;
    // Server-sent events too, as the streaming inference endpoints send:
    // the events, read as the text they are.
    types
        .iter()
        .any(|t| t == "text/plain" || t == "text/event-stream")
        && !types.iter().any(|t| t == "application/json")
}

/// The endpoint modules to put behind the `beta` or `experimental` feature:
/// those of endpoints Elasticsearch marks so, when nothing outside the
/// module uses their types. Also the endpoints left ungated, and why.
#[allow(clippy::type_complexity)]
fn gates(
    ir: &ir::Ir<'_>,
    endpoints: &[Endpoint],
) -> (BTreeMap<(String, Vec<String>), &'static str>, Vec<String>) {
    let mut stable_modules = BTreeSet::new();
    let mut candidates: BTreeMap<(String, Vec<String>), (&'static str, String)> = BTreeMap::new();
    for endpoint in endpoints {
        let Some(item) = endpoint
            .request
            .as_ref()
            .and_then(|r| ir.graph.find(r))
            .and_then(|t| ir.item_of.get(&t))
        else {
            continue;
        };
        let item = &ir.items[*item];
        let module = (item.krate.clone(), item.module.clone());
        // An endpoint only serverless has goes by its stability there.
        let feature = match endpoint
            .availability
            .stack
            .as_ref()
            .or(endpoint.availability.serverless.as_ref())
            .and_then(|s| s.stability)
        {
            Some(Stability::Beta) => "beta",
            Some(Stability::Experimental | Stability::TechPreview) => "experimental",
            _ => {
                stable_modules.insert(module);
                continue;
            }
        };
        candidates.insert(module, (feature, endpoint.name.clone()));
    }
    let mut gates = BTreeMap::new();
    let mut ungated = Vec::new();
    for (module, (feature, name)) in candidates {
        if module.1.is_empty() || stable_modules.contains(&module) {
            ungated.push(format!("{name}: its module holds other endpoints' types"));
            continue;
        }
        let inside = |id: usize| {
            ir.items[id].krate == module.0 && ir.items[id].module.starts_with(&module.1)
        };
        let used_outside = (0..ir.items.len())
            .filter(|&id| !inside(id))
            .find(|&id| ir.references(id).into_iter().any(inside));
        match used_outside {
            Some(id) => ungated.push(format!(
                "{name}: {}::{} uses its types",
                ir.items[id].module.join("::"),
                ir.items[id].ident
            )),
            None => {
                gates.insert(module, feature);
            }
        }
    }
    (gates, ungated)
}
