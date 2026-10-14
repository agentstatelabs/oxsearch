//! The type graph: every type the specification declares, and every
//! reference from one type to another, labeled with where it is made.

use std::collections::HashMap;

use crate::spec::model::{
    Body, Inherits, Property, Schema, TypeDefinition, TypeName, ValueOf, Variants,
};

/// The namespace of the primitive types (`string`, `boolean`, `integer`…),
/// which the specification references but doesn't declare.
const BUILTINS: &str = "_builtins";
/// The namespace of the specification's own helpers. Behaviors from it, such
/// as `AdditionalProperties`, describe a shape rather than name a type, so a
/// behavior's generic arguments are references but the behavior isn't.
const SPEC_UTILS: &str = "_spec_utils";
/// The behavior saying a type is a variant of another's shape, such as the
/// form of a role an API returns rather than accepts. The type lists all its
/// own properties, so it doesn't use the other type.
const OVERLOAD_OF: &str = "OverloadOf";

/// A reference from one type to another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edge {
    /// The referenced type, as an index into [`Graph::types`].
    pub to: usize,
    /// Where the reference is made, such as `inner_hits`, `inherits`,
    /// `body.aggregations` or `query.routing`.
    pub via: String,
}

/// The types and the references between them.
pub struct Graph<'a> {
    /// Every declared type, in the specification's order.
    pub types: Vec<&'a TypeDefinition>,
    /// Each type's references, by the referring type's index.
    pub edges: Vec<Vec<Edge>>,
    index: HashMap<&'a TypeName, usize>,
}

impl<'a> Graph<'a> {
    /// Builds the graph. Fails on a reference to a type that isn't declared,
    /// isn't a primitive, and isn't one of the referring type's generic
    /// parameters.
    pub fn new(schema: &'a Schema) -> Result<Graph<'a>, crate::Error> {
        let types: Vec<&TypeDefinition> = schema.types.iter().collect();
        let mut index = HashMap::with_capacity(types.len());
        for (i, definition) in types.iter().enumerate() {
            if index.insert(definition.name(), i).is_some() {
                return Err(format!("{} is declared twice", definition.name()).into());
            }
        }
        let mut edges = Vec::with_capacity(types.len());
        let mut unknown = Vec::new();
        for definition in &types {
            let owner = definition.name();
            let generic_namespace = format!("{}.{}", owner.namespace, owner.name);
            let mut found = Vec::new();
            references(definition, &mut found);
            let mut out: Vec<Edge> = Vec::new();
            for (name, via) in found {
                if let Some(&to) = index.get(name) {
                    let edge = Edge { to, via };
                    if !out.contains(&edge) {
                        out.push(edge);
                    }
                } else if name.namespace != BUILTINS && name.namespace != generic_namespace {
                    unknown.push(format!("{owner} references {name}, which isn't declared"));
                }
            }
            edges.push(out);
        }
        if !unknown.is_empty() {
            unknown.sort();
            unknown.dedup();
            return Err(unknown.join("\n").into());
        }
        Ok(Graph {
            types,
            edges,
            index,
        })
    }

    /// The index of a declared type.
    pub fn find(&self, name: &TypeName) -> Option<usize> {
        self.index.get(name).copied()
    }

    /// A type's name, by index.
    pub fn name(&self, i: usize) -> &'a TypeName {
        self.types[i].name()
    }
}

/// Every type a definition references, with where.
fn references<'a>(definition: &'a TypeDefinition, out: &mut Vec<(&'a TypeName, String)>) {
    match definition {
        TypeDefinition::Interface(t) => {
            inherits(t.inherits.as_ref(), out);
            behaviors(&t.behaviors, out);
            properties("", &t.properties, out);
            variants(t.variants.as_ref(), out);
        }
        TypeDefinition::Request(t) => {
            inherits(t.inherits.as_ref(), out);
            properties("path.", &t.path, out);
            properties("query.", &t.query, out);
            body(&t.body, "body", out);
        }
        TypeDefinition::Response(t) => {
            body(&t.body, "body", out);
            for exception in &t.exceptions {
                body(&exception.body, "exceptions", out);
            }
        }
        TypeDefinition::Enum(_) => {}
        TypeDefinition::TypeAlias(t) => {
            value(&t.value, "type", out);
            variants(t.variants.as_ref(), out);
        }
    }
}

fn inherits<'a>(parent: Option<&'a Inherits>, out: &mut Vec<(&'a TypeName, String)>) {
    if let Some(parent) = parent {
        out.push((&parent.type_name, "inherits".to_owned()));
        for generic in &parent.generics {
            value(generic, "inherits", out);
        }
    }
}

fn behaviors<'a>(behaviors: &'a [Inherits], out: &mut Vec<(&'a TypeName, String)>) {
    for behavior in behaviors {
        if behavior.type_name.namespace == SPEC_UTILS && behavior.type_name.name == OVERLOAD_OF {
            continue;
        }
        if behavior.type_name.namespace != SPEC_UTILS {
            out.push((&behavior.type_name, "behaviors".to_owned()));
        }
        let via = format!("behaviors.{}", behavior.type_name.name);
        for generic in &behavior.generics {
            value(generic, &via, out);
        }
    }
}

fn properties<'a>(prefix: &str, properties: &'a [Property], out: &mut Vec<(&'a TypeName, String)>) {
    for property in properties {
        value(&property.value, &format!("{prefix}{}", property.name), out);
    }
}

fn body<'a>(body: &'a Body, via: &str, out: &mut Vec<(&'a TypeName, String)>) {
    match body {
        Body::NoBody {} => {}
        Body::Properties { properties: p } => properties(&format!("{via}."), p, out),
        Body::Value { value: v, .. } => value(v, via, out),
    }
}

fn variants<'a>(variants: Option<&'a Variants>, out: &mut Vec<(&'a TypeName, String)>) {
    if let Some(Variants::Untagged { untyped_variant }) = variants {
        out.push((untyped_variant, "variants".to_owned()));
    }
}

fn value<'a>(value_of: &'a ValueOf, via: &str, out: &mut Vec<(&'a TypeName, String)>) {
    match value_of {
        ValueOf::InstanceOf {
            type_name,
            generics,
        } => {
            out.push((type_name, via.to_owned()));
            for generic in generics {
                value(generic, via, out);
            }
        }
        ValueOf::ArrayOf { value: v } => value(v, via, out),
        ValueOf::UnionOf { items } => {
            for item in items {
                value(item, via, out);
            }
        }
        ValueOf::DictionaryOf { key, value: v, .. } => {
            value(key, via, out);
            value(v, via, out);
        }
        ValueOf::UserDefinedValue {} | ValueOf::LiteralValue { .. } => {}
    }
}

/// The strongly connected components of a directed graph given as adjacency
/// lists, each sorted, in a deterministic order (by smallest member).
/// Components of one node without a self-loop are left out.
pub fn cycles(adjacency: &[Vec<usize>]) -> Vec<Vec<usize>> {
    // Tarjan's algorithm, iteratively, so deep graphs don't overflow the
    // stack.
    let n = adjacency.len();
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0; n];
    let mut on_stack = vec![false; n];
    let mut stack = Vec::new();
    let mut next = 0;
    let mut components = Vec::new();
    for root in 0..n {
        if index[root] != usize::MAX {
            continue;
        }
        let mut work = vec![(root, 0usize)];
        while let Some(&mut (v, ref mut child)) = work.last_mut() {
            if *child == 0 {
                index[v] = next;
                low[v] = next;
                next += 1;
                stack.push(v);
                on_stack[v] = true;
            }
            if let Some(&w) = adjacency[v].get(*child) {
                *child += 1;
                if index[w] == usize::MAX {
                    work.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            work.pop();
            if let Some(&(parent, _)) = work.last() {
                low[parent] = low[parent].min(low[v]);
            }
            if low[v] == index[v] {
                let mut component = Vec::new();
                loop {
                    let w = stack.pop().expect("v is on the stack");
                    on_stack[w] = false;
                    component.push(w);
                    if w == v {
                        break;
                    }
                }
                if component.len() > 1 || adjacency[v].contains(&v) {
                    component.sort_unstable();
                    components.push(component);
                }
            }
        }
    }
    components.sort();
    components
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_cycles() {
        // 0 → 1 → 2 → 0, 3 → 3, 4 → 0 alone.
        let adjacency = vec![vec![1], vec![2], vec![0], vec![3], vec![0]];
        assert_eq!(cycles(&adjacency), vec![vec![0, 1, 2], vec![3]]);
        assert!(cycles(&[vec![1], vec![]]).is_empty());
    }
}
