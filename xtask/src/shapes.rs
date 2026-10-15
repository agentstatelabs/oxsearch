//! The shapes the generator has to map to serde: an inventory of the
//! specification by shape, and the unions whose members their JSON alone
//! can't tell apart.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::graph::Graph;
use crate::placement::Placement;
use crate::spec::Pin;
use crate::spec::model::{
    Body, Interface, Literal, Property, TypeAlias, TypeDefinition, TypeName, ValueOf, Variants,
};

/// The report, relative to the workspace root.
pub const REPORT_FILE: &str = "spec/shapes.md";

/// The crates of the first generated slice, AI search (DESIGN.md §8).
pub const SLICE: &[&str] = &["core", "mapping", "search", "inference", "docs"];

/// The JSON kinds a value may take, as a set of bits.
pub type Kinds = u8;
/// A string.
pub const STRING: Kinds = 1;
/// A number.
pub const NUMBER: Kinds = 2;
/// `true` or `false`.
pub const BOOLEAN: Kinds = 4;
/// `null`.
pub const NULL: Kinds = 8;
/// An array.
pub const ARRAY: Kinds = 16;
/// An object.
pub const OBJECT: Kinds = 32;
/// Anything: a generic parameter or a user-defined value.
pub const ANY: Kinds = 63;

/// How a union's members are told apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum UnionKind {
    /// `T` or `T[]`.
    OneOrMany,
    /// Each member takes different JSON kinds, so the JSON says which.
    ByJsonKind,
    /// A member is generic or user-defined, so it could be anything.
    Open,
    /// Members share a primitive kind, such as an enum and a string.
    OverlappingPrimitives,
    /// Two or more members are objects. Their properties have to tell them
    /// apart.
    OverlappingObjects,
}

impl UnionKind {
    fn describe(self) -> &'static str {
        match self {
            UnionKind::OneOrMany => "one or many (`T` or `T[]`)",
            UnionKind::ByJsonKind => "told apart by JSON kind",
            UnionKind::Open => "with a generic or user-defined member",
            UnionKind::OverlappingPrimitives => "members share a primitive kind",
            UnionKind::OverlappingObjects => "two or more object members",
        }
    }
}

/// The inventory.
pub struct Inventory<'g> {
    graph: &'g Graph<'g>,
    /// For each shape, how many types or values have it: in all, and in the
    /// first slice's crates.
    pub shapes: BTreeMap<&'static str, (usize, usize)>,
    /// Every union: where it is, its members, and how they're told apart.
    pub unions: Vec<Union>,
}

/// A union, where the specification declares it.
pub struct Union {
    /// The type that declares it.
    pub owner: usize,
    /// Where in the type: a property, or `type` for a union alias.
    pub via: String,
    /// Its members, written as in the specification.
    pub members: Vec<String>,
    /// How its members are told apart.
    pub kind: UnionKind,
    /// Whether the owner is in the first slice.
    pub in_slice: bool,
}

/// Takes the inventory.
pub fn inventory<'g>(graph: &'g Graph<'g>, placement: &Placement<'_>) -> Inventory<'g> {
    let mut inventory = Inventory {
        graph,
        shapes: BTreeMap::new(),
        unions: Vec::new(),
    };
    for (i, definition) in graph.types.iter().enumerate() {
        let in_slice = SLICE.contains(&placement.krate[i]);
        let mut count = |shape: &'static str| {
            let entry = inventory.shapes.entry(shape).or_default();
            entry.0 += 1;
            if in_slice {
                entry.1 += 1;
            }
        };
        match definition {
            TypeDefinition::Interface(t) => {
                interface_shapes(t, &mut count);
            }
            TypeDefinition::Request(_) => count("request"),
            TypeDefinition::Response(_) => count("response"),
            TypeDefinition::Enum(t) => count(if t.is_open {
                "enum, open"
            } else {
                "enum, closed"
            }),
            TypeDefinition::TypeAlias(t) => alias_shapes(t, &mut count),
        }
        if !generics(definition).is_empty() {
            count("generic type");
        }
        let mut values = Vec::new();
        values_of(definition, &mut values);
        for (via, value) in values {
            walk(value, &mut |v| {
                if let ValueOf::DictionaryOf {
                    single_key: true, ..
                } = v
                {
                    count("object keyed by a field name (single-key dictionary)");
                }
            });
            collect_unions(graph, i, &via, value, in_slice, &mut inventory.unions);
        }
    }
    for union in &inventory.unions {
        let shape = match union.kind {
            UnionKind::OneOrMany => "union: one or many",
            UnionKind::ByJsonKind => "union: told apart by JSON kind",
            UnionKind::Open => "union: generic or user-defined member",
            UnionKind::OverlappingPrimitives => "union: members share a primitive kind",
            UnionKind::OverlappingObjects => "union: two or more object members",
        };
        let entry = inventory.shapes.entry(shape).or_default();
        entry.0 += 1;
        if union.in_slice {
            entry.1 += 1;
        }
    }
    inventory
}

fn interface_shapes(t: &Interface, count: &mut impl FnMut(&'static str)) {
    match &t.variants {
        Some(Variants::Container { .. }) => {
            count("container: one variant, keyed by its name");
            if t.properties.iter().any(|p| p.container_property) {
                count("container with properties beside the variant");
            }
            return;
        }
        Some(_) => count("struct with unexpected variants"),
        None => count("struct"),
    }
    if t.shortcut_property.is_some() {
        count("struct with a shorthand form");
    }
    for behavior in &t.behaviors {
        match behavior.type_name.name.as_str() {
            "AdditionalProperty" => count("struct with one property keyed by a field name"),
            "AdditionalProperties" => count("struct with additional properties"),
            _ => {}
        }
    }
}

fn alias_shapes(t: &TypeAlias, count: &mut impl FnMut(&'static str)) {
    match &t.variants {
        Some(Variants::InternalTag { default_tag, .. }) => {
            count("union, internally tagged");
            if default_tag.is_some() {
                count("union, internally tagged, with a default tag");
            }
        }
        Some(Variants::ExternalTag { .. }) => count("union, externally tagged (typed keys)"),
        Some(Variants::Untagged { .. }) => count("union, untagged, with an untyped fallback"),
        Some(Variants::Container { .. }) => count("alias with unexpected variants"),
        None => count(match t.value {
            ValueOf::UnionOf { .. } => "alias of a union",
            _ => "alias",
        }),
    }
}

fn generics(definition: &TypeDefinition) -> &[TypeName] {
    match definition {
        TypeDefinition::Interface(t) => &t.generics,
        TypeDefinition::Request(t) => &t.generics,
        TypeDefinition::Response(t) => &t.generics,
        TypeDefinition::TypeAlias(t) => &t.generics,
        TypeDefinition::Enum(_) => &[],
    }
}

/// The values a definition declares, with where. A union alias's own value
/// is `type`; variants of tagged unions are not unions of their own.
fn values_of<'a>(definition: &'a TypeDefinition, out: &mut Vec<(String, &'a ValueOf)>) {
    let props = |prefix: &str, properties: &'a [Property], out: &mut Vec<(String, &'a ValueOf)>| {
        for p in properties {
            out.push((format!("{prefix}{}", p.name), &p.value));
        }
    };
    let body = |b: &'a Body, out: &mut Vec<(String, &'a ValueOf)>| match b {
        Body::NoBody {} => {}
        Body::Properties { properties } => props("body.", properties, out),
        Body::Value { value, .. } => out.push(("body".to_owned(), value)),
    };
    match definition {
        TypeDefinition::Interface(t) => props("", &t.properties, out),
        TypeDefinition::Request(t) => {
            props("path.", &t.path, out);
            props("query.", &t.query, out);
            body(&t.body, out);
        }
        TypeDefinition::Response(t) => body(&t.body, out),
        TypeDefinition::Enum(_) => {}
        TypeDefinition::TypeAlias(t) => {
            if t.variants.is_none() {
                out.push(("type".to_owned(), &t.value));
            }
        }
    }
}

fn walk<'a>(value: &'a ValueOf, f: &mut impl FnMut(&'a ValueOf)) {
    f(value);
    match value {
        ValueOf::InstanceOf { generics, .. } => generics.iter().for_each(|g| walk(g, f)),
        ValueOf::ArrayOf { value } => walk(value, f),
        ValueOf::UnionOf { items } => items.iter().for_each(|i| walk(i, f)),
        ValueOf::DictionaryOf { key, value, .. } => {
            walk(key, f);
            walk(value, f);
        }
        ValueOf::UserDefinedValue {} | ValueOf::LiteralValue { .. } => {}
    }
}

fn collect_unions(
    graph: &Graph<'_>,
    owner: usize,
    via: &str,
    value: &ValueOf,
    in_slice: bool,
    out: &mut Vec<Union>,
) {
    walk(value, &mut |v| {
        if let ValueOf::UnionOf { items } = v {
            let kinds: Vec<Kinds> = items
                .iter()
                .map(|item| json_kinds(graph, item, &[], 0))
                .collect();
            out.push(Union {
                owner,
                via: via.to_owned(),
                members: items.iter().map(write_value).collect(),
                kind: classify(items, &kinds),
                in_slice,
            });
        }
    });
}

fn classify(items: &[ValueOf], kinds: &[Kinds]) -> UnionKind {
    if let [a, ValueOf::ArrayOf { value: b }] = items
        && write_value(a) == write_value(b)
    {
        return UnionKind::OneOrMany;
    }
    if kinds.contains(&ANY) {
        return UnionKind::Open;
    }
    let mut seen: Kinds = 0;
    let mut overlap: Kinds = 0;
    for &k in kinds {
        overlap |= seen & k;
        seen |= k;
    }
    if overlap == 0 {
        UnionKind::ByJsonKind
    } else if overlap & OBJECT != 0 {
        UnionKind::OverlappingObjects
    } else {
        UnionKind::OverlappingPrimitives
    }
}

/// The JSON kinds a value may take. `bound` gives the kinds of the generic
/// parameters bound so far; any other generic parameter may be anything.
/// `depth` stops runaway recursion through aliases.
pub fn json_kinds(
    graph: &Graph<'_>,
    value: &ValueOf,
    bound: &[(&TypeName, Kinds)],
    depth: usize,
) -> Kinds {
    if depth > 32 {
        return ANY;
    }
    match value {
        ValueOf::InstanceOf {
            type_name,
            generics,
        } => {
            if type_name.namespace == "_builtins" {
                return match type_name.name.as_str() {
                    "string" | "binary" => STRING,
                    "number" => NUMBER,
                    "boolean" => BOOLEAN,
                    "null" => NULL,
                    _ => 0,
                };
            }
            if let Some(&(_, k)) = bound.iter().find(|(name, _)| *name == type_name) {
                return k;
            }
            let Some(i) = graph.find(type_name) else {
                // A generic parameter of the type the value is in.
                return ANY;
            };
            match graph.types[i] {
                TypeDefinition::Interface(t) => {
                    let mut k = OBJECT;
                    if let Some(shortcut) = &t.shortcut_property
                        && let Some(p) = t.properties.iter().find(|p| &p.name == shortcut)
                    {
                        k |= json_kinds(graph, &p.value, &[], depth + 1);
                    }
                    k
                }
                TypeDefinition::Request(_) | TypeDefinition::Response(_) => OBJECT,
                TypeDefinition::Enum(_) => STRING,
                TypeDefinition::TypeAlias(t) => {
                    if t.variants.is_some() {
                        return OBJECT;
                    }
                    let args: Vec<Kinds> = generics
                        .iter()
                        .map(|g| json_kinds(graph, g, bound, depth + 1))
                        .collect();
                    let inner: Vec<(&TypeName, Kinds)> = t.generics.iter().zip(args).collect();
                    json_kinds(graph, &t.value, &inner, depth + 1)
                }
            }
        }
        ValueOf::ArrayOf { .. } => ARRAY,
        ValueOf::DictionaryOf { .. } => OBJECT,
        ValueOf::UnionOf { items } => items
            .iter()
            .fold(0, |k, item| k | json_kinds(graph, item, bound, depth + 1)),
        ValueOf::UserDefinedValue {} => ANY,
        ValueOf::LiteralValue { value } => match value {
            Literal::String(_) => STRING,
            Literal::Number(_) => NUMBER,
            Literal::Boolean(_) => BOOLEAN,
        },
    }
}

/// A value's type, written as in the specification: `Field | Field[]`,
/// `Dictionary<string, Query>`.
fn write_value(value: &ValueOf) -> String {
    match value {
        ValueOf::InstanceOf {
            type_name,
            generics,
        } => {
            if generics.is_empty() {
                type_name.name.clone()
            } else {
                let args: Vec<String> = generics.iter().map(write_value).collect();
                format!("{}<{}>", type_name.name, args.join(", "))
            }
        }
        ValueOf::ArrayOf { value } => format!("{}[]", write_value(value)),
        ValueOf::UnionOf { items } => {
            let items: Vec<String> = items.iter().map(write_value).collect();
            format!("({})", items.join(" | "))
        }
        ValueOf::DictionaryOf {
            key,
            value,
            single_key,
        } => format!(
            "{}<{}, {}>",
            if *single_key {
                "SingleKeyDictionary"
            } else {
                "Dictionary"
            },
            write_value(key),
            write_value(value)
        ),
        ValueOf::UserDefinedValue {} => "UserDefinedValue".to_owned(),
        ValueOf::LiteralValue { value } => match value {
            Literal::String(s) => format!("{s:?}"),
            Literal::Number(n) => n.to_string(),
            Literal::Boolean(b) => b.to_string(),
        },
    }
}

impl Inventory<'_> {
    /// The report, in Markdown. Committed as `spec/shapes.md`; CI fails when
    /// it and the generator differ.
    pub fn report(&self, pin: &Pin) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "# Shapes");
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "Generated by `cargo run -p xtask -- shapes` from {} {} at `{}`. Don't edit it: \
             CI regenerates it and fails if it differs.",
            pin.repository, pin.branch, pin.commit
        );
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "The shapes the generator maps to serde, counted over the whole specification and \
             over the first slice's crates ({}). A type can have more than one shape: a generic \
             container counts as both.",
            SLICE.join(", ")
        );
        let _ = writeln!(out);
        let _ = writeln!(out, "| Shape | All | First slice |");
        let _ = writeln!(out, "|---|--:|--:|");
        for (shape, (all, slice)) in &self.shapes {
            let _ = writeln!(out, "| {shape} | {all} | {slice} |");
        }
        let _ = writeln!(out);
        let _ = writeln!(out, "## Unions with two or more object members");
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "JSON kinds alone can't tell these members apart: the generated code has to look at \
             their properties. First-slice unions are marked ●."
        );
        let _ = writeln!(out);
        let mut lines: Vec<String> = self
            .unions
            .iter()
            .filter(|u| u.kind == UnionKind::OverlappingObjects)
            .map(|u| {
                format!(
                    "- {}`{}` `{}`: {}",
                    if u.in_slice { "● " } else { "" },
                    self.graph.name(u.owner),
                    u.via,
                    u.members.join(" | ")
                )
            })
            .collect();
        lines.sort();
        lines.dedup();
        for line in lines {
            let _ = writeln!(out, "{line}");
        }
        let _ = writeln!(out);
        let _ = writeln!(out, "## How each union kind is told apart");
        let _ = writeln!(out);
        for kind in [
            UnionKind::OneOrMany,
            UnionKind::ByJsonKind,
            UnionKind::Open,
            UnionKind::OverlappingPrimitives,
            UnionKind::OverlappingObjects,
        ] {
            let n = self.unions.iter().filter(|u| u.kind == kind).count();
            let _ = writeln!(out, "- {}: {n}", kind.describe());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(json: &str) -> ValueOf {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn classifies_unions() {
        let field = r#"{"kind": "instance_of", "type": {"namespace": "_types", "name": "Field"}}"#;
        let fields = format!(r#"{{"kind": "array_of", "value": {field}}}"#);
        let one_or_many = [value(field), value(&fields)];
        assert_eq!(
            classify(&one_or_many, &[STRING, ARRAY]),
            UnionKind::OneOrMany
        );
        let string =
            r#"{"kind": "instance_of", "type": {"namespace": "_builtins", "name": "string"}}"#;
        let number =
            r#"{"kind": "instance_of", "type": {"namespace": "_builtins", "name": "number"}}"#;
        let two = [value(string), value(number)];
        assert_eq!(classify(&two, &[STRING, NUMBER]), UnionKind::ByJsonKind);
        assert_eq!(
            classify(&two, &[STRING, STRING]),
            UnionKind::OverlappingPrimitives
        );
        assert_eq!(
            classify(&two, &[OBJECT, OBJECT | ARRAY]),
            UnionKind::OverlappingObjects
        );
        assert_eq!(classify(&two, &[STRING, ANY]), UnionKind::Open);
    }

    #[test]
    fn writes_values_as_the_specification_does() {
        let v = value(
            r#"{"kind": "dictionary_of", "singleKey": false,
                "key": {"kind": "instance_of", "type": {"namespace": "_builtins", "name": "string"}},
                "value": {"kind": "array_of", "value": {"kind": "literal_value", "value": "x"}}}"#,
        );
        assert_eq!(write_value(&v), r#"Dictionary<string, "x"[]>"#);
    }
}
