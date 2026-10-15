//! Placement: which crate each type goes in.
//!
//! The specification's namespaces can't be crates as they are: they depend
//! on each other in loops, which Cargo forbids. Placement follows three
//! rules, and moves no type by hand:
//!
//! 1. **Home.** Each type starts in the crate its namespace maps to
//!    ([`home`]).
//! 2. **Layers.** The foundation crates have a fixed order ([`FOUNDATION`]):
//!    each may depend only on the crates listed below it. Every other crate
//!    is an API area, which may depend on any foundation crate and on other
//!    areas, as long as the areas form no loop.
//! 3. **Moving down.** A reference from a crate to one it may not depend on
//!    moves the referenced type down: into the referring crate when the
//!    type's crate may depend on that one, and into the core otherwise.
//!    Moved types' own references are checked again, until nothing moves.
//!
//! A few references are seams instead ([`SEAMS`]): placement doesn't follow
//! them, and the generated code makes them open, so the crates stay apart.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::Error;
use crate::graph::{self, Graph};
use crate::spec::Pin;
use crate::spec::model::TypeName;

/// The placement report, relative to the workspace root.
pub const REPORT_FILE: &str = "spec/placement.md";

/// The foundation crates, lowest first, each with the crates it may depend
/// on directly. The core holds what every crate shares; a type two
/// foundation crates both need, when neither may depend on the other, moves
/// to the core.
pub const FOUNDATION: &[(&str, &[&str])] = &[
    ("core", &[]),
    ("analysis", &["core"]),
    ("mapping", &["analysis"]),
    ("tasks", &["core"]),
    ("inference", &["core"]),
    ("search", &["mapping"]),
    ("aggs", &["search"]),
    ("docs", &["mapping", "tasks"]),
];

/// A seam: references from one crate to a type in a crate it may not depend
/// on, which placement doesn't follow. The generated code gives the
/// referring property an open type instead.
pub struct Seam {
    /// The crate the references are made from.
    pub from: &'static str,
    /// The referenced types, as `namespace::name`, and what a reference to
    /// each becomes.
    pub to: &'static [(&'static str, Open)],
    /// Why it is a seam, and what the property's type becomes.
    pub why: &'static str,
}

/// What a reference through a seam becomes in the generated code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Open {
    /// Any JSON value.
    Json,
    /// `oxsearch_core::AggsBody`, which `json!` and oxsearch-aggs' types
    /// convert into, in place of the map of aggregations the reference is
    /// in.
    AggsBody,
    /// The type parameter `TAggregations`, `oxsearch_core::RawAggs` by
    /// default, in place of the map of aggregation results the reference is
    /// in. Every type that contains it takes the parameter too.
    AggsParam,
}

/// What a reference placement cut becomes: by (from type, to type).
pub fn open_form(placement: &Placement<'_>, from: usize, to: usize) -> Option<Open> {
    let &(seam, ..) = placement
        .cuts
        .iter()
        .find(|&&(_, f, _, t)| f == from && t == to)?;
    let name = placement.graph.name(to);
    let target = format!("{}::{}", name.namespace, name.name);
    SEAMS[seam]
        .to
        .iter()
        .find(|(t, _)| *t == target)
        .map(|&(_, open)| open)
}

/// The seams.
pub const SEAMS: &[Seam] = &[
    Seam {
        from: "core",
        to: &[("_global.search._types::SearchRequestBody", Open::Json)],
        why: "A stored script's source may be a whole search body. The core takes \
              any body that converts into one, so it doesn't depend on search.",
    },
    Seam {
        from: "search",
        to: &[
            ("_types.aggregations::AggregationContainer", Open::AggsBody),
            ("_types.aggregations::Aggregate", Open::AggsParam),
        ],
        why: "A search's aggregations. The request takes `impl Into<AggsBody>`, \
              which aggs and `json!` provide, and the response's aggregations are \
              a type parameter, raw JSON by default. So search doesn't depend on aggs.",
    },
    Seam {
        from: "core",
        to: &[("watcher._types::SearchInputRequestDefinition", Open::Json)],
        why: "A search transform's request, used only by Watcher. It is open \
              in the core, so the core doesn't depend on Watcher.",
    },
];

/// Endpoints in the `_global` namespace that belong to the search crate.
const SEARCH_ENDPOINTS: &[&str] = &[
    "clear_scroll",
    "close_point_in_time",
    "count",
    "explain",
    "field_caps",
    "knn_search",
    "msearch",
    "msearch_template",
    "open_point_in_time",
    "rank_eval",
    "render_search_template",
    "scroll",
    "search",
    "search_mvt",
    "search_shards",
    "search_template",
    "terms_enum",
];

/// Endpoints in the `_global` namespace that belong to the document crate.
const DOCS_ENDPOINTS: &[&str] = &[
    "bulk",
    "cancel_reindex",
    "create",
    "delete",
    "delete_by_query",
    "delete_by_query_rethrottle",
    "exists",
    "exists_source",
    "get",
    "get_reindex",
    "get_source",
    "index",
    "list_reindex",
    "mget",
    "mtermvectors",
    "reindex",
    "reindex_rethrottle",
    "termvectors",
    "update",
    "update_by_query",
    "update_by_query_rethrottle",
];

/// Endpoints in the `_global` namespace that every client needs, so they
/// live in the core.
const CORE_ENDPOINTS: &[&str] = &["info", "ping"];

/// Namespaces that share a crate with others, by their first part. Every
/// other namespace is a crate of its own, named after its first part.
const SHARED_AREAS: &[(&str, &str)] = &[
    ("_internal", "admin"),
    ("_spec_utils", "core"),
    ("cluster", "admin"),
    ("indices", "admin"),
    ("nodes", "admin"),
];

/// A type's home crate: where its namespace puts it, before placement moves
/// anything.
pub fn home(namespace: &str) -> &str {
    let mut parts = namespace.split('.');
    let first = parts.next().unwrap_or_default();
    let second = parts.next();
    match (first, second) {
        ("_types", Some("aggregations")) => "aggs",
        ("_types", Some("analysis")) => "analysis",
        ("_types", Some("mapping")) => "mapping",
        // The common types, including the query DSL, retrievers and kNN.
        ("_types", _) => "core",
        ("_global", Some(endpoint)) if SEARCH_ENDPOINTS.contains(&endpoint) => "search",
        ("_global", Some(endpoint)) if DOCS_ENDPOINTS.contains(&endpoint) => "docs",
        ("_global", Some(endpoint)) if CORE_ENDPOINTS.contains(&endpoint) => "core",
        // Scripts, health and capabilities: the cluster's.
        ("_global", _) => "admin",
        _ => SHARED_AREAS
            .iter()
            .find(|(area, _)| *area == first)
            .map_or(first, |(_, krate)| krate),
    }
}

/// Where every type goes, and why.
pub struct Placement<'g> {
    graph: &'g Graph<'g>,
    /// Each type's crate, by type index.
    pub krate: Vec<&'g str>,
    /// The types placement moved from their home crate, by type index, with
    /// the reference that moved each one last.
    pub moves: BTreeMap<usize, (usize, String)>,
    /// The references the seams cut, as (seam, from type, property, to type).
    pub cuts: Vec<(usize, usize, String, usize)>,
}

/// Places every type in the graph by the rules above.
pub fn place<'g>(graph: &'g Graph<'g>) -> Result<Placement<'g>, Error> {
    let layers = Layers::new()?;
    let seams = seam_targets(graph)?;
    let mut krate: Vec<&str> = (0..graph.types.len())
        .map(|i| home(&graph.name(i).namespace))
        .collect();
    let mut moves = BTreeMap::new();
    loop {
        let mut moved = false;
        for (from, edges) in graph.edges.iter().enumerate() {
            for edge in edges {
                let (a, b) = (krate[from], krate[edge.to]);
                if layers.sees(a, b) || seams.contains_key(&(a, edge.to)) {
                    continue;
                }
                krate[edge.to] = if layers.sees(b, a) { a } else { "core" };
                moves.insert(edge.to, (from, edge.via.clone()));
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    moves.retain(|&i, _| krate[i] != home(&graph.name(i).namespace));

    let mut cuts = Vec::new();
    for (from, edges) in graph.edges.iter().enumerate() {
        for edge in edges {
            if let Some(&seam) = seams.get(&(krate[from], edge.to)) {
                cuts.push((seam, from, edge.via.clone(), edge.to));
            }
        }
    }

    let placement = Placement {
        graph,
        krate,
        moves,
        cuts,
    };
    placement.check(&layers)?;
    Ok(placement)
}

impl Placement<'_> {
    /// The crates each crate depends on, from the references placement
    /// follows.
    pub fn dependencies(&self) -> BTreeMap<&str, BTreeSet<&str>> {
        let mut deps = BTreeMap::<&str, BTreeSet<&str>>::new();
        for (from, edges) in self.graph.edges.iter().enumerate() {
            let a = self.krate[from];
            let entry = deps.entry(a).or_default();
            for edge in edges {
                let b = self.krate[edge.to];
                if b != a && !self.is_cut(from, edge.to) {
                    entry.insert(b);
                }
            }
        }
        deps
    }

    /// How many types in each crate are part of a loop of references inside
    /// it. The generated code boxes somewhere in each loop.
    pub fn recursive(&self) -> BTreeMap<&str, usize> {
        let adjacency: Vec<Vec<usize>> = self
            .graph
            .edges
            .iter()
            .enumerate()
            .map(|(from, edges)| {
                edges
                    .iter()
                    .filter(|e| self.krate[e.to] == self.krate[from] && !self.is_cut(from, e.to))
                    .map(|e| e.to)
                    .collect()
            })
            .collect();
        let mut counts = BTreeMap::new();
        for component in graph::cycles(&adjacency) {
            *counts.entry(self.krate[component[0]]).or_default() += component.len();
        }
        counts
    }

    fn is_cut(&self, from: usize, to: usize) -> bool {
        self.cuts.iter().any(|&(_, f, _, t)| f == from && t == to)
    }

    /// Checks what placement promises: every reference it follows is one
    /// the referring crate may make, and the crates form no loop.
    fn check(&self, layers: &Layers) -> Result<(), Error> {
        for (from, edges) in self.graph.edges.iter().enumerate() {
            for edge in edges {
                let (a, b) = (self.krate[from], self.krate[edge.to]);
                if !layers.sees(a, b) && !self.is_cut(from, edge.to) {
                    return Err(format!(
                        "{} ({a}) still references {} ({b}) through {}",
                        self.graph.name(from),
                        self.graph.name(edge.to),
                        edge.via
                    )
                    .into());
                }
            }
        }
        let deps = self.dependencies();
        let names: Vec<&str> = deps.keys().copied().collect();
        let adjacency: Vec<Vec<usize>> = names
            .iter()
            .map(|name| {
                deps[name]
                    .iter()
                    .map(|dep| names.binary_search(dep).expect("every crate is a key"))
                    .collect()
            })
            .collect();
        let loops = graph::cycles(&adjacency);
        if loops.is_empty() {
            return Ok(());
        }
        let loops: Vec<String> = loops
            .iter()
            .map(|component| {
                let crates: Vec<&str> = component.iter().map(|&i| names[i]).collect();
                crates.join(", ")
            })
            .collect();
        Err(format!(
            "these crates depend on each other in a loop, which needs a new rule: {}",
            loops.join("; ")
        )
        .into())
    }

    /// The placement report, in Markdown. Committed as `spec/placement.md`;
    /// CI fails when the generator's placement and the report differ.
    pub fn report(&self, pin: &Pin) -> String {
        let graph = self.graph;
        let deps = self.dependencies();
        let recursive = self.recursive();
        let mut sizes = BTreeMap::<&str, (usize, usize)>::new();
        for (i, &krate) in self.krate.iter().enumerate() {
            let entry = sizes.entry(krate).or_default();
            entry.0 += 1;
            if self.moves.contains_key(&i) {
                entry.1 += 1;
            }
        }
        let foundation: Vec<&str> = FOUNDATION.iter().map(|(name, _)| *name).collect();
        let mut order: Vec<&str> = foundation.clone();
        order.extend(sizes.keys().filter(|name| !foundation.contains(name)));

        let mut out = String::new();
        let _ = writeln!(out, "# Crate placement");
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "Generated by `cargo run -p xtask -- placement` from {} {} at `{}`. Don't edit it: \
             CI regenerates it and fails if it differs. The rules are in `xtask/src/placement.rs`.",
            pin.repository, pin.branch, pin.commit
        );
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "{} types in {} crates; placement moved {} of them out of their namespace's crate, \
             and {} references are seams.",
            graph.types.len(),
            sizes.len(),
            self.moves.len(),
            self.cuts.len()
        );
        let _ = writeln!(out);
        let _ = writeln!(out, "## Crates");
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "Recursive types are those in a loop of references inside their crate."
        );
        let _ = writeln!(out);
        let _ = writeln!(out, "| Crate | Types | Moved in | Recursive | Depends on |");
        let _ = writeln!(out, "|---|--:|--:|--:|---|");
        for name in &order {
            let Some(&(count, moved)) = sizes.get(name) else {
                continue;
            };
            let on: Vec<&str> = deps
                .get(name)
                .map(|d| order.iter().copied().filter(|o| d.contains(o)).collect())
                .unwrap_or_default();
            let kind = if foundation.contains(name) {
                ""
            } else {
                " (area)"
            };
            let _ = writeln!(
                out,
                "| {name}{kind} | {count} | {moved} | {} | {} |",
                recursive.get(name).copied().unwrap_or(0),
                if on.is_empty() {
                    "–".to_owned()
                } else {
                    on.join(", ")
                }
            );
        }
        let _ = writeln!(out);
        let _ = writeln!(out, "## Seams");
        for (s, seam) in SEAMS.iter().enumerate() {
            let _ = writeln!(out);
            let _ = writeln!(
                out,
                "**From {} to {}.** {}",
                seam.from,
                seam.to
                    .iter()
                    .map(|(name, _)| *name)
                    .collect::<Vec<_>>()
                    .join(", "),
                seam.why
            );
            let _ = writeln!(out);
            let mut cut: Vec<String> = self
                .cuts
                .iter()
                .filter(|c| c.0 == s)
                .map(|&(_, from, ref via, to)| {
                    format!("- `{}` `{via}` → `{}`", graph.name(from), graph.name(to))
                })
                .collect();
            cut.sort();
            for line in cut {
                let _ = writeln!(out, "{line}");
            }
        }
        let _ = writeln!(out);
        let _ = writeln!(out, "## Moved types");
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "Each type with the reference that moved it last. A type moved to make room \
             for another is listed with the reference from the type that pulled it."
        );
        for name in &order {
            let mut lines: Vec<String> = self
                .moves
                .iter()
                .filter(|&(&i, _)| self.krate[i] == *name)
                .map(|(&i, (from, via))| {
                    format!(
                        "- `{}` from {}: `{}` `{via}`",
                        graph.name(i),
                        home(&graph.name(i).namespace),
                        graph.name(*from)
                    )
                })
                .collect();
            if lines.is_empty() {
                continue;
            }
            lines.sort();
            let _ = writeln!(out);
            let _ = writeln!(out, "### Into {name} ({})", lines.len());
            let _ = writeln!(out);
            for line in lines {
                let _ = writeln!(out, "{line}");
            }
        }
        out
    }
}

/// The foundation crates' order, closed over: which crates each may see.
struct Layers {
    below: BTreeMap<&'static str, BTreeSet<&'static str>>,
}

impl Layers {
    fn new() -> Result<Layers, Error> {
        let mut below = BTreeMap::<&str, BTreeSet<&str>>::new();
        for &(name, direct) in FOUNDATION {
            let mut seen = BTreeSet::new();
            for dep in direct {
                let Some(dep_below) = below.get(dep) else {
                    return Err(format!(
                        "{name} may depend on {dep}, which isn't listed before it"
                    )
                    .into());
                };
                seen.insert(*dep);
                seen.extend(dep_below.iter().copied());
            }
            below.insert(name, seen);
        }
        Ok(Layers { below })
    }

    /// Whether crate `a` may depend on crate `b`. An area may depend on
    /// every foundation crate, and on other areas while the areas form no
    /// loop (checked once placement is done).
    fn sees(&self, a: &str, b: &str) -> bool {
        a == b || self.below.get(a).is_none_or(|below| below.contains(b))
    }
}

/// The types each seam cuts references to, by (from crate, to type).
fn seam_targets<'g>(graph: &Graph<'g>) -> Result<BTreeMap<(&'static str, usize), usize>, Error> {
    let mut targets = BTreeMap::new();
    for (s, seam) in SEAMS.iter().enumerate() {
        for (to, _) in seam.to {
            let (namespace, name) = to
                .split_once("::")
                .expect("seam targets are namespace::name");
            let found = graph
                .find(&TypeName {
                    namespace: namespace.to_owned(),
                    name: name.to_owned(),
                })
                .ok_or_else(|| format!("seam target {to} isn't in the specification"))?;
            targets.insert((seam.from, found), s);
        }
    }
    Ok(targets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn homes() {
        assert_eq!(home("_types"), "core");
        assert_eq!(home("_types.query_dsl"), "core");
        assert_eq!(home("_types.aggregations"), "aggs");
        assert_eq!(home("_global.search._types"), "search");
        assert_eq!(home("_global.bulk"), "docs");
        assert_eq!(home("_global.info"), "core");
        assert_eq!(home("_global.put_script"), "admin");
        assert_eq!(home("indices.create"), "admin");
        assert_eq!(home("inference._types"), "inference");
        assert_eq!(home("ml._types"), "ml");
    }

    /// A schema of interfaces, each given as (namespace, name, properties),
    /// where each property references another interface by
    /// `namespace::name`.
    ///
    /// Every seam's targets are added, as a seam whose target is missing
    /// fails placement.
    fn schema(types: &[(&str, &str, &[&str])]) -> crate::spec::Schema {
        let mut types = types.to_vec();
        for (target, _) in SEAMS.iter().flat_map(|seam| seam.to.iter()) {
            let (namespace, name) = target.split_once("::").unwrap();
            if !types.iter().any(|t| t.0 == namespace && t.1 == name) {
                types.push((namespace, name, &[]));
            }
        }
        let types: Vec<serde_json::Value> = types
            .iter()
            .map(|(namespace, name, refs)| {
                let properties: Vec<serde_json::Value> = refs
                    .iter()
                    .enumerate()
                    .map(|(i, to)| {
                        let (ns, n) = to.split_once("::").unwrap();
                        serde_json::json!({
                            "name": format!("p{i}"),
                            "required": false,
                            "type": {"kind": "instance_of", "type": {"namespace": ns, "name": n}}
                        })
                    })
                    .collect();
                serde_json::json!({
                    "kind": "interface",
                    "name": {"namespace": namespace, "name": name},
                    "specLocation": "x.ts#L1-L1",
                    "properties": properties
                })
            })
            .collect();
        let json = serde_json::json!({
            "_info": {"title": "t", "license": {"name": "Apache 2.0", "url": "u"}},
            "endpoints": [],
            "types": types
        });
        crate::spec::load(json.to_string().as_bytes()).unwrap()
    }

    #[test]
    fn moves_types_down_and_respects_seams() {
        let schema = schema(&[
            // The core references a search type, which references another:
            // both move to the core.
            ("_types", "KnnSearch", &["_global.search._types::InnerHits"]),
            (
                "_global.search._types",
                "InnerHits",
                &["_global.search._types::Highlight"],
            ),
            ("_global.search._types", "Highlight", &[]),
            // Search references the core: nothing moves.
            (
                "_global.search",
                "Request",
                &[
                    "_types::KnnSearch",
                    "_global.search._types::SearchRequestBody",
                ],
            ),
            // A seam: the core's reference to the search body isn't followed.
            (
                "_types",
                "ScriptSource",
                &["_global.search._types::SearchRequestBody"],
            ),
            (
                "_global.search._types",
                "SearchRequestBody",
                &["_types.aggregations::AggregationContainer"],
            ),
            (
                "_types.aggregations",
                "AggregationContainer",
                &["ml._types::RegressionInferenceOptions"],
            ),
            // An area's type that a foundation crate references moves into it.
            ("ml._types", "RegressionInferenceOptions", &[]),
            // Two foundation crates that can't see each other share through
            // the core.
            (
                "_global.bulk",
                "UpdateAction",
                &["_global.search._types::SourceConfig"],
            ),
            ("_global.search._types", "SourceConfig", &[]),
            // Areas may depend on each other.
            (
                "cat._types",
                "MlRow",
                &["ml._types::RegressionInferenceOptions"],
            ),
        ]);
        let graph = Graph::new(&schema).unwrap();
        let placement = place(&graph).unwrap();
        let krate = |namespace: &str, name: &str| {
            let i = graph
                .find(&TypeName {
                    namespace: namespace.to_owned(),
                    name: name.to_owned(),
                })
                .unwrap();
            placement.krate[i]
        };
        assert_eq!(krate("_global.search._types", "InnerHits"), "core");
        assert_eq!(krate("_global.search._types", "Highlight"), "core");
        assert_eq!(
            krate("_global.search._types", "SearchRequestBody"),
            "search"
        );
        assert_eq!(krate("_types.aggregations", "AggregationContainer"), "aggs");
        assert_eq!(krate("ml._types", "RegressionInferenceOptions"), "aggs");
        assert_eq!(krate("_global.search._types", "SourceConfig"), "core");
        assert_eq!(krate("cat._types", "MlRow"), "cat");
        assert_eq!(placement.moves.len(), 4);
        assert_eq!(placement.cuts.len(), 2);
        let deps = placement.dependencies();
        assert!(!deps["core"].contains("search"));
        assert!(!deps["search"].contains("aggs"));
    }

    #[test]
    fn a_loop_between_areas_needs_a_rule() {
        let schema = schema(&[
            ("ml._types", "A", &["watcher._types::B"]),
            ("watcher._types", "B", &["ml._types::A"]),
        ]);
        let graph = Graph::new(&schema).unwrap();
        let error = place(&graph).err().unwrap().to_string();
        assert!(error.contains("ml, watcher"), "{error}");
    }

    #[test]
    fn the_foundation_is_in_order() {
        Layers::new().unwrap();
    }
}
