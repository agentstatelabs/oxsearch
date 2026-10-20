//! Rust names for the specification's names, and doc comments from its
//! descriptions.

/// Rust's keywords, strict and reserved, in every edition the generated code
/// might be read in.
const KEYWORDS: &[&str] = &[
    "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "crate",
    "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl",
    "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref",
    "return", "self", "Self", "static", "struct", "super", "trait", "true", "try", "type",
    "typeof", "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
];

/// Keywords that can't be raw identifiers.
const NOT_RAW: &[&str] = &["crate", "self", "Self", "super"];

/// A field, function or module name: `snake_case`, a raw identifier when it
/// is a keyword.
pub fn snake(wire: &str) -> String {
    let mut out = String::new();
    let mut previous_lower = false;
    for c in wire.chars() {
        if c.is_ascii_alphanumeric() {
            if c.is_ascii_uppercase() && previous_lower && !out.ends_with('_') {
                out.push('_');
            }
            previous_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
            previous_lower = false;
        }
    }
    let trimmed = out.trim_end_matches('_');
    let mut name = if trimmed.is_empty() {
        "value".to_owned()
    } else {
        trimmed.to_owned()
    };
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        name.insert(0, 'n');
    }
    keyword(name)
}

/// A type or variant name: `PascalCase`.
pub fn pascal(wire: &str) -> String {
    // A word in capitals is one word, as an acronym is in Rust's names:
    // `RUNNING` is `Running` and `BM25` is `Bm25`.
    let wire: String = wire
        .split_inclusive(|c: char| !c.is_ascii_alphanumeric())
        .map(|word| {
            if word.chars().any(|c| c.is_ascii_lowercase()) {
                return word.to_owned();
            }
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                std::iter::once(first)
                    .chain(chars.map(|c| c.to_ascii_lowercase()))
                    .collect()
            })
        })
        .collect();
    let mut out = String::new();
    let mut upper = true;
    for c in wire.chars() {
        if c.is_ascii_alphanumeric() {
            if upper {
                out.push(c.to_ascii_uppercase());
                upper = false;
            } else {
                out.push(c);
            }
            if c.is_ascii_digit() {
                upper = true;
            }
        } else {
            upper = true;
        }
    }
    if out.is_empty() {
        return "Empty".to_owned();
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, 'V');
    }
    if out == "Self" {
        out.push('_');
    }
    out
}

/// A name for a numeric or boolean literal, as a variant.
pub fn literal_variant(text: &str) -> String {
    match text {
        "-1" => "MinusOne".to_owned(),
        "0" => "Zero".to_owned(),
        "1" => "One".to_owned(),
        "true" => "True".to_owned(),
        "false" => "False".to_owned(),
        other => pascal(other),
    }
}

fn keyword(name: String) -> String {
    if NOT_RAW.contains(&name.as_str()) {
        format!("{name}_")
    } else if KEYWORDS.contains(&name.as_str()) {
        format!("r#{name}")
    } else {
        name
    }
}

/// The name without a raw identifier's `r#`.
pub fn bare(ident: &str) -> &str {
    ident.strip_prefix("r#").unwrap_or(ident)
}

/// A doc comment from a description: Markdown, with fenced code marked as
/// text so rustdoc doesn't run it, and indentation outside code removed so
/// rustdoc doesn't read indented text as code.
pub fn doc(description: Option<&str>, indent: &str) -> String {
    let Some(description) = description.map(str::trim).filter(|d| !d.is_empty()) else {
        return String::new();
    };
    let mut out = String::new();
    let mut fenced = false;
    for line in description.lines() {
        let trimmed = line.trim_start();
        let text = if let Some(rest) = trimmed.strip_prefix("```") {
            if fenced {
                fenced = false;
                "```".to_owned()
            } else {
                fenced = true;
                let _ = rest;
                "```text".to_owned()
            }
        } else if fenced {
            line.trim_end().to_owned()
        } else {
            trimmed.trim_end().to_owned()
        };
        out.push_str(indent);
        if text.is_empty() {
            out.push_str("///\n");
        } else {
            out.push_str("/// ");
            out.push_str(&text);
            out.push('\n');
        }
    }
    if fenced {
        out.push_str(indent);
        out.push_str("/// ```\n");
    }
    out
}

/// A Rust string literal.
pub fn string(text: &str) -> String {
    format!("{text:?}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_names() {
        assert_eq!(snake("_source"), "source");
        assert_eq!(snake("type"), "r#type");
        assert_eq!(snake("self"), "self_");
        assert_eq!(snake("minimumShouldMatch"), "minimum_should_match");
        assert_eq!(snake("index.number_of_shards"), "index_number_of_shards");
        assert_eq!(snake("@timestamp"), "timestamp");
        assert_eq!(snake("1d"), "n1d");
        assert_eq!(snake("gen"), "r#gen");
    }

    #[test]
    fn pascal_names() {
        assert_eq!(pascal("RUNNING"), "Running");
        assert_eq!(pascal("PAUSED_FOR_NODE_REMOVAL"), "PausedForNodeRemoval");
        assert_eq!(pascal("BM25"), "Bm25");
        assert_eq!(pascal("LMDirichlet"), "LMDirichlet");
        assert_eq!(pascal("GET"), "Get");
        assert_eq!(pascal("match_phrase"), "MatchPhrase");
        assert_eq!(pascal("geo_distance"), "GeoDistance");
        assert_eq!(pascal("text"), "Text");
        assert_eq!(pascal("1h"), "V1H");
        assert_eq!(pascal("http/1.1"), "Http11");
        assert_eq!(pascal(""), "Empty");
        assert_eq!(pascal("self"), "Self_");
    }

    #[test]
    fn docs_keep_code_out_of_doctests() {
        let doc = doc(Some("Run it:\n```\nGET /\n```\n    indented\n"), "");
        assert_eq!(
            doc,
            "/// Run it:\n/// ```text\n/// GET /\n/// ```\n/// indented\n"
        );
    }
}
