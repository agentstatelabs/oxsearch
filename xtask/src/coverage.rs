//! The coverage report: the endpoints generated in each crate, and those
//! the integration tests send to Elasticsearch.
//!
//! The integration tests record each request's method and path in the
//! files `OXSEARCH_COVERAGE` names, one line each; this matches them
//! against the specification's URL templates.

use std::collections::{BTreeMap, BTreeSet};

use crate::codegen::CRATES;
use crate::graph::Graph;
use crate::placement::Placement;
use crate::spec::model::Endpoint;

/// An endpoint the generator wrote, and where.
struct Generated<'a> {
    name: &'a str,
    krate: &'a str,
    endpoint: &'a Endpoint,
}

/// The report, as Markdown, from the recorded requests.
pub fn report(
    graph: &Graph<'_>,
    placement: &Placement<'_>,
    endpoints: &[Endpoint],
    recorded: &[String],
) -> String {
    let generated: Vec<Generated<'_>> = endpoints
        .iter()
        .filter(|e| !e.codegen_exclude)
        .filter_map(|endpoint| {
            let request = graph.find(endpoint.request.as_ref()?)?;
            let krate = placement.krate[request];
            CRATES.contains(&krate).then_some(Generated {
                name: &endpoint.name,
                krate,
                endpoint,
            })
        })
        .collect();
    // Every endpoint, generated or not, to name what a request was for.
    let every: Vec<Generated<'_>> = endpoints
        .iter()
        .map(|endpoint| Generated {
            name: &endpoint.name,
            krate: "",
            endpoint,
        })
        .collect();
    let mut tested: BTreeSet<&str> = BTreeSet::new();
    let mut elsewhere: BTreeSet<&str> = BTreeSet::new();
    for line in recorded {
        let Some((method, path)) = line.trim().split_once(' ') else {
            continue;
        };
        let Some(name) = best_match(&every, method, path) else {
            continue;
        };
        if generated.iter().any(|g| g.name == name) {
            tested.insert(name);
        } else {
            elsewhere.insert(name);
        }
    }
    let mut by_crate: BTreeMap<&str, (Vec<&str>, Vec<&str>)> = BTreeMap::new();
    for g in &generated {
        let entry = by_crate.entry(g.krate).or_default();
        if tested.contains(g.name) {
            entry.0.push(g.name);
        } else {
            entry.1.push(g.name);
        }
    }
    let mut out = String::from(
        "# Endpoint coverage\n\n\
         The endpoints generated in each crate, and those the integration tests send to \
         Elasticsearch on the Basic license and a trial. Written by \
         `cargo run -p xtask -- coverage` from the requests the tests record \
         (`OXSEARCH_COVERAGE`); each CI integration job prints its own.\n\n\
         | Crate | Generated | Integration-tested |\n|---|---:|---:|\n",
    );
    let (mut all, mut hit) = (0, 0);
    for krate in CRATES {
        let Some((yes, no)) = by_crate.get(krate) else {
            continue;
        };
        let total = yes.len() + no.len();
        all += total;
        hit += yes.len();
        out.push_str(&format!(
            "| `{}` | {total} | {} ({}%) |\n",
            crate::codegen::package(krate),
            yes.len(),
            yes.len() * 100 / total.max(1)
        ));
    }
    out.push_str(&format!(
        "| all | {all} | {hit} ({}%) |\n",
        hit * 100 / all.max(1)
    ));
    for krate in CRATES {
        let Some((yes, no)) = by_crate.get(krate) else {
            continue;
        };
        out.push_str(&format!("\n## `{}`\n\n", crate::codegen::package(krate)));
        if !yes.is_empty() {
            out.push_str(&format!("Tested: {}.\n\n", list(yes)));
        }
        if !no.is_empty() {
            out.push_str(&format!("Not tested yet: {}.\n", list(no)));
        }
    }
    if !elsewhere.is_empty() {
        let names: Vec<&str> = elsewhere.into_iter().collect();
        out.push_str(&format!(
            "\n## APIs not generated yet\n\nThe tests also call these through the escape hatches: {}.\n",
            list(&names)
        ));
    }
    out
}

fn list(names: &[&str]) -> String {
    let mut names = names.to_vec();
    names.sort();
    names
        .iter()
        .map(|n| format!("`{n}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The endpoint a request was for: of the URL templates its method and
/// path fit, the one with the most fixed segments. Endpoints that share a
/// template, such as the inference API's general `put` and its
/// provider-specific ones, go to the first in the specification's order,
/// the general one.
fn best_match<'a>(generated: &[Generated<'a>], method: &str, path: &str) -> Option<&'a str> {
    let path = path.split('?').next().unwrap_or(path);
    let mut best: Option<(usize, &'a str)> = None;
    for (name, url) in generated
        .iter()
        .flat_map(|g| g.endpoint.urls.iter().map(move |url| (g.name, url)))
    {
        if !url.methods.iter().any(|m| m == method) {
            continue;
        }
        if let Some(fixed) = fits(&url.path, path)
            && best.is_none_or(|(most, _)| fixed > most)
        {
            best = Some((fixed, name));
        }
    }
    best.map(|(_, name)| name)
}

/// How many fixed segments a template has, if the path fits it: a
/// `{parameter}` takes one segment, which isn't an API's (`_search`) but may
/// be `_all`.
fn fits(template: &str, path: &str) -> Option<usize> {
    let template: Vec<&str> = template.trim_matches('/').split('/').collect();
    let path: Vec<&str> = path.trim_matches('/').split('/').collect();
    if template.len() != path.len() {
        return None;
    }
    let mut fixed = 0;
    for (t, p) in template.iter().zip(&path) {
        if t.starts_with('{') {
            if p.is_empty() || (p.starts_with('_') && *p != "_all") {
                return None;
            }
        } else if t == p {
            fixed += 1;
        } else {
            return None;
        }
    }
    Some(fixed)
}

#[cfg(test)]
mod tests {
    use super::fits;

    #[test]
    fn templates() {
        assert_eq!(fits("/{index}/_search", "/articles/_search"), Some(1));
        assert_eq!(fits("/_search", "/_search"), Some(1));
        assert_eq!(fits("/{index}/_doc/{id}", "/_inference/rerank/x"), None);
        assert_eq!(fits("/{index}", "/_bulk"), None);
        assert_eq!(fits("/_info/{target}", "/_info/_all"), Some(1));
        assert_eq!(fits("/", "/"), Some(1));
        assert_eq!(fits("/{index}/_doc/{id}", "/a/_doc/1"), Some(1));
    }
}
