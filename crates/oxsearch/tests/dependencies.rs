//! The examples' documentation lists the dependencies each needs in an
//! application of your own, and the site shows those lists. This keeps
//! them true: oxsearch at this release's version, with features that
//! exist and include those the example needs, and every other crate the
//! example uses, with the features it uses.

use std::collections::BTreeSet;

const MANIFEST: &str = include_str!("../Cargo.toml");

/// The examples, by name, with their source.
const EXAMPLES: &[(&str, &str)] = &[
    ("analytics", include_str!("../examples/analytics.rs")),
    ("documents", include_str!("../examples/documents.rs")),
    (
        "escape_hatches",
        include_str!("../examples/escape_hatches.rs"),
    ),
    ("quickstart", include_str!("../examples/quickstart.rs")),
    ("rag", include_str!("../examples/rag.rs")),
    ("transport", include_str!("../examples/transport.rs")),
];

/// The first ```toml block of a file's `//!` documentation.
fn dependencies(source: &str) -> Option<Vec<String>> {
    let mut block: Option<Vec<String>> = None;
    for line in source.lines().take_while(|line| line.starts_with("//!")) {
        let line = line.trim_start_matches("//!").trim();
        match &mut block {
            None if line == "```toml" => block = Some(Vec::new()),
            Some(_) if line == "```" => return block,
            Some(lines) => lines.push(line.to_owned()),
            None => {}
        }
    }
    None
}

/// The quoted strings in a TOML array, such as `features = ["a", "b"]`.
fn array(line: &str, key: &str) -> Vec<String> {
    let Some(at) = line.find(&format!("{key} = [")) else {
        return Vec::new();
    };
    let rest = &line[at + key.len() + 4..];
    let end = rest.find(']').expect("the array ends");
    rest[..end]
        .split(',')
        .map(|item| item.trim().trim_matches('"').to_owned())
        .filter(|item| !item.is_empty())
        .collect()
}

/// The front crate's features, and those on by default.
fn features() -> (BTreeSet<String>, BTreeSet<String>) {
    let section = MANIFEST
        .split("\n[features]\n")
        .nth(1)
        .and_then(|rest| rest.split("\n[").next())
        .expect("Cargo.toml has features");
    let mut all = BTreeSet::new();
    let mut default = BTreeSet::new();
    for line in section.lines().filter(|line| !line.starts_with('#')) {
        if let Some((name, _)) = line.split_once(" = [") {
            all.insert(name.to_owned());
            if name == "default" {
                default.extend(array(line, "default"));
            }
        }
    }
    (all, default)
}

/// An example's `required-features`.
fn required(example: &str) -> Vec<String> {
    let header = format!("[[example]]\nname = \"{example}\"\n");
    MANIFEST
        .split_once(&header)
        .and_then(|(_, rest)| rest.lines().next())
        .map(|line| array(line, "required-features"))
        .unwrap_or_default()
}

#[test]
fn the_examples_name_what_they_need() {
    let version = env!("CARGO_PKG_VERSION");
    let minor = version.rsplit_once('.').expect("a version").0;
    let (all, default) = features();
    let mut documented = 0;
    for (name, source) in EXAMPLES {
        let Some(lines) = dependencies(source) else {
            continue;
        };
        documented += 1;
        assert_eq!(lines.first().map(String::as_str), Some("[dependencies]"));
        let entry = |krate: &str| {
            lines
                .iter()
                .find(|line| line.starts_with(&format!("{krate} = ")))
                .unwrap_or_else(|| panic!("examples/{name}.rs doesn't list {krate}"))
        };

        let oxsearch = entry("oxsearch");
        assert!(
            oxsearch.contains(&format!("version = \"{minor}\"")),
            "examples/{name}.rs lists another oxsearch than {minor}: {oxsearch}"
        );
        let listed = array(oxsearch, "features");
        for feature in &listed {
            assert!(
                all.contains(feature),
                "examples/{name}.rs: no feature {feature}"
            );
        }
        for feature in required(name) {
            assert!(
                listed.contains(&feature) || default.contains(&feature),
                "examples/{name}.rs needs the {feature} feature but doesn't list it"
            );
        }

        // Every other crate it uses, and the features it uses of them.
        let mut used: BTreeSet<&str> = source
            .lines()
            .filter_map(|line| line.strip_prefix("use "))
            .filter_map(|path| path.split("::").next())
            .filter(|root| !["std", "core", "oxsearch", "crate", "self", "super"].contains(root))
            .collect();
        if source.contains("#[tokio::main") {
            used.insert("tokio");
        }
        for krate in used {
            entry(krate);
        }
        if source.contains("Serialize") || source.contains("Deserialize") {
            assert!(
                entry("serde").contains("\"derive\""),
                "examples/{name}.rs: serde needs derive"
            );
        }
        if source.contains("#[tokio::main") {
            let tokio = entry("tokio");
            assert!(
                tokio.contains("\"macros\""),
                "examples/{name}.rs: tokio needs macros"
            );
            let runtime = if source.contains("current_thread") {
                "\"rt\""
            } else {
                "\"rt-multi-thread\""
            };
            assert!(
                tokio.contains(runtime),
                "examples/{name}.rs: tokio needs {runtime}"
            );
        }
    }
    assert!(
        documented >= 4,
        "the examples' dependency lists went missing"
    );
}
