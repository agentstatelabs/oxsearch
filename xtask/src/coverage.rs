//! The coverage report: the endpoints generated in each crate, and those
//! the integration tests send to Elasticsearch.
//!
//! The integration tests record each request's method and path in the
//! files `OXSEARCH_COVERAGE` names, one line each, followed by the
//! endpoint's name when the client knew it. A request without a name is
//! matched against the specification's URL templates.

use std::collections::{BTreeMap, BTreeSet};

use crate::codegen::CRATES;
use crate::graph::Graph;
use crate::placement::Placement;
use crate::spec::model::{Endpoint, Visibility};

/// Whether Elastic Cloud Serverless offers an endpoint to everyone.
fn on_serverless(endpoint: &Endpoint) -> bool {
    endpoint
        .availability
        .serverless
        .as_ref()
        .is_some_and(|a| !matches!(a.visibility, Some(Visibility::Private)))
}

/// What a single-node fixture can't give an area's tests, by crate.
const LIMITS: &[(&str, &str)] = &[
    (
        "ccr",
        "Tested on the trial only. The fixture follows its own indices, through a remote \
         cluster that points back at its node; following another cluster isn't tested.",
    ),
    (
        "searchable_snapshots",
        "Mounting, the mounted index's statistics and clearing its cache are tested on the \
         trial only, and the mount as a full copy: the fixture's node has no shared cache, \
         so its cache statistics count no regions.",
    ),
    (
        "dangling_indices",
        "A single node can't leave an index dangling, so the import and the delete are \
         tested on a UUID no node has, which Elasticsearch refuses.",
    ),
    ("autoscaling", "Tested on the trial only."),
    (
        "fleet",
        "The specification gives the multi-search a `wait_for_checkpoints` URL parameter, \
         which Elasticsearch refuses.",
    ),
    ("graph", "Tested on the trial only."),
    (
        "profiling",
        "The queries need a license above Basic and indices the fixture doesn't have, so \
         they're refused on both licenses.",
    ),
    (
        "project",
        "Only Elastic Cloud Serverless has these, so the fixture can't test them.",
    ),
    (
        "inference",
        "Calls to a provider go to a mock service on the CI runner, which the fixture reaches \
         as `host.docker.internal`: OpenAI, and the providers that take a URL. Providers that \
         only call their own cloud (Amazon Bedrock and SageMaker, Anthropic, Azure, Cohere, \
         Google, Mistral, watsonx and others) aren't tested, since that needs an account with \
         each and sends the requests to them. Creating an endpoint needs a license above Basic, \
         so on Basic the tests check the refusal.",
    ),
    (
        "esql",
        "Data sources and datasets point at storage the fixture can't reach, so those calls \
         are checked for an answer or a refusal Elasticsearch understood. Stopping an async \
         query is tested on one that has finished, and getting a running query by an id no \
         query has.",
    ),
    (
        "migration",
        "A cluster on the current version has no system features to upgrade, so the \
         upgrade asked for is declined.",
    ),
];

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
        let mut fields = line.split_whitespace();
        let (Some(method), Some(path)) = (fields.next(), fields.next()) else {
            continue;
        };
        let named = fields
            .next()
            .and_then(|name| every.iter().find(|g| g.name == name))
            .map(|g| g.name);
        let Some(name) = named.or_else(|| best_match(&every, method, path)) else {
            continue;
        };
        if generated.iter().any(|g| g.name == name) {
            tested.insert(name);
        } else {
            elsewhere.insert(name);
        }
    }
    let mut by_crate: BTreeMap<&str, (Vec<&str>, Vec<&str>)> = BTreeMap::new();
    // By crate: the endpoints serverless lacks.
    let mut not_serverless: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for g in &generated {
        let entry = by_crate.entry(g.krate).or_default();
        if tested.contains(g.name) {
            entry.0.push(g.name);
        } else {
            entry.1.push(g.name);
        }
        if !on_serverless(g.endpoint) && g.endpoint.availability.stack.is_some() {
            not_serverless.entry(g.krate).or_default().push(g.name);
        }
    }
    let mut out = String::from(
        "# Endpoint coverage\n\n\
         The endpoints generated in each crate, and those the integration tests send to \
         Elasticsearch on the Basic license and a trial. Written by \
         `cargo run -p xtask -- coverage` from the requests the tests record \
         (`OXSEARCH_COVERAGE`); each CI integration job prints its own. Each endpoint's \
         documentation also says where it's available: since which Elasticsearch \
         version, and whether on Elastic Cloud Serverless.\n\n\
         | Crate | Generated | On Serverless | Integration-tested |\n|---|---:|---:|---:|\n",
    );
    let (mut all, mut hit, mut on) = (0, 0, 0);
    for krate in CRATES {
        let Some((yes, no)) = by_crate.get(krate) else {
            continue;
        };
        let total = yes.len() + no.len();
        let serverless = generated
            .iter()
            .filter(|g| g.krate == *krate && on_serverless(g.endpoint))
            .count();
        all += total;
        hit += yes.len();
        on += serverless;
        out.push_str(&format!(
            "| `{}` | {total} | {serverless} | {} ({}%) |\n",
            crate::codegen::package(krate),
            yes.len(),
            yes.len() * 100 / total.max(1)
        ));
    }
    out.push_str(&format!(
        "| all | {all} | {on} | {hit} ({}%) |\n",
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
            out.push_str(&format!("Not tested yet: {}.\n\n", list(no)));
        }
        if let Some(names) = not_serverless.get(krate) {
            out.push_str(&format!(
                "Not on Elastic Cloud Serverless: {}.\n\n",
                list(names)
            ));
        }
        if let Some((_, note)) = LIMITS.iter().find(|(k, _)| k == krate) {
            out.push_str(&format!("{note}\n\n"));
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

/// The endpoint a request without a name was for: of the URL templates
/// its method and path fit, the one with the most fixed segments.
/// Endpoints that share a template, such as the inference API's general
/// `put` and its provider-specific ones, go to the first in the
/// specification's order, the general one.
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
