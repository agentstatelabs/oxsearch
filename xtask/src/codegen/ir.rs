//! The generator's intermediate form: the Rust items to write, built from
//! the specification, with recursion broken by boxing.

use std::collections::{BTreeSet, HashMap, HashSet};

use super::names::{literal_variant, pascal, snake};
use crate::Error;
use crate::graph::{self, Graph};
use crate::placement::{Open, Placement};
use crate::shapes::{self, ANY, OBJECT};
use crate::spec::model::{
    Body, Inherits, Interface, Literal, Property, TypeAlias, TypeDefinition, TypeName, ValueOf,
    Variants,
};

/// An item's index in [`Ir::items`].
pub type ItemId = usize;

/// A Rust type.
#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    /// A primitive or standard type, written as it is.
    Prim(&'static str),
    /// `()`.
    Unit,
    /// Any JSON value.
    Json,
    /// A generic parameter.
    Param(String),
    /// A generated type, by the specification type it comes from.
    Named {
        /// The specification type.
        target: usize,
        /// Its generic arguments.
        args: Vec<Ty>,
    },
    /// A generated item with no specification type of its own, such as a
    /// union declared inline in a property.
    Inline {
        /// The item.
        item: ItemId,
        /// Its generic arguments.
        args: Vec<Ty>,
    },
    /// An array.
    Vec(Box<Ty>),
    /// An object used as a map.
    Map(Box<Ty>, Box<Ty>),
    /// An object with one property, named by data.
    SingleKey(Box<Ty>, Box<Ty>),
    /// A value or an array of values.
    OneOrMany(Box<Ty>),
    /// A value Elasticsearch may write as a string.
    Stringified(Box<Ty>),
    /// A value where `null` means something.
    WithNull(Box<Ty>),
    /// Flags joined by `|`.
    Flags(Box<Ty>),
    /// A value or `null`.
    Opt(Box<Ty>),
    /// An object whose keys carry their values' types.
    TypedKeys(Box<Ty>),
}

/// Where in an item a type is used: a field, or a variant's payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Position {
    /// A struct's field.
    Field,
    /// An enum variant's payload.
    Variant,
}

/// A generated item.
pub struct Item {
    /// The crate, such as `core`.
    pub krate: String,
    /// The module path inside the crate.
    pub module: Vec<String>,
    /// The Rust name.
    pub ident: String,
    /// What it is.
    pub doc: Option<String>,
    /// Its generic parameters.
    pub generics: Vec<String>,
    /// Its shape.
    pub kind: ItemKind,
}

/// The shapes of generated items.
pub enum ItemKind {
    /// A struct (also a container's struct when it has properties beside the
    /// variant).
    Struct(Struct),
    /// A container: one property, named after the variant.
    Container(Container),
    /// An enum of strings.
    Enum(Vec<EnumMember>),
    /// A union of other types.
    Union(Vec<UnionMember>),
    /// An internally tagged union.
    Tagged {
        /// The property naming the member.
        tag: String,
        /// The member when the tag is missing.
        default: Option<String>,
        /// The members.
        members: Vec<Variant>,
    },
    /// An externally tagged union, read under typed keys.
    External(Vec<Variant>),
    /// An untagged union read as its untyped member.
    Untagged {
        /// The members.
        members: Vec<Variant>,
        /// The untyped member's index.
        fallback: usize,
    },
    /// `pub type`.
    Alias(Ty),
    /// An endpoint's request: path and query parameters and a body.
    Request(Request),
    /// A response whose body is another type.
    Newtype(Ty),
}

/// A struct's field.
pub struct Field {
    /// The name on the wire.
    pub wire: String,
    /// The Rust name.
    pub ident: String,
    /// The type, without the `Option` an optional field adds.
    pub ty: Ty,
    /// Whether the specification requires it: constructors take it.
    pub required: bool,
    /// Whether the Rust field is an `Option`: the property is optional, or
    /// the type is read from responses, which may leave out required ones.
    pub optional: bool,
    /// Whether a missing value reads as empty: a required array or map in a
    /// type read from responses.
    pub defaulted: bool,
    /// What it is.
    pub doc: Option<String>,
    /// Other names Elasticsearch accepts.
    pub aliases: Vec<String>,
}

/// A struct.
pub struct Struct {
    /// Its fields.
    pub fields: Vec<Field>,
    /// Additional properties, collected into a map: (field, key, value).
    pub extra: Option<(String, Ty, Ty)>,
    /// One more property keyed by data, such as a field name:
    /// (key field, value field, key type, value type).
    pub keyed: Option<(String, String, Ty, Ty)>,
    /// The field a shorthand form stands for, and whether to write the
    /// shorthand back.
    pub shorthand: Option<(usize, bool)>,
    /// For a container with properties beside its variant: the field and
    /// the variant enum.
    pub variant: Option<(String, ItemId)>,
    /// Whether it has serde implementations.
    pub serde: bool,
}

/// A container: one property, named after the variant.
pub struct Container {
    /// Its variants.
    pub variants: Vec<Variant>,
    /// For an unknown key: a variant that takes it as data (key type, value
    /// type, variant name) rather than `Raw`.
    pub keyed: Option<(String, Ty, Ty)>,
}

/// A variant of a container or tagged union.
pub struct Variant {
    /// The key or tag naming it on the wire.
    pub wire: String,
    /// Other names Elasticsearch accepts.
    pub aliases: Vec<String>,
    /// The Rust name.
    pub ident: String,
    /// Its payload.
    pub ty: Ty,
    /// What it is.
    pub doc: Option<String>,
}

/// One value of an enum of strings.
pub struct EnumMember {
    /// The Rust name.
    pub ident: String,
    /// The value.
    pub wire: String,
    /// Other values Elasticsearch accepts for it.
    pub aliases: Vec<String>,
    /// What it means.
    pub doc: Option<String>,
}

/// A member of a union.
pub struct UnionMember {
    /// The Rust name.
    pub ident: String,
    /// Its payload, or `None` for a literal.
    pub ty: Option<Ty>,
    /// A literal member's value.
    pub literal: Option<Literal>,
    /// The JSON kinds it takes.
    pub kinds: u8,
    /// For an object member, the properties it may have.
    pub keys: Option<BTreeSet<String>>,
    /// Whether it is an enum of strings, with `from_known`.
    pub string_enum: bool,
}

/// An endpoint's request.
pub struct Request {
    /// Path parameters.
    pub path: Vec<Field>,
    /// Query parameters.
    pub query: Vec<Field>,
    /// The body, if it has one.
    pub body: Option<Ty>,
}

/// Where a specification type lives in Rust.
#[derive(Clone)]
pub struct Location {
    /// The crate.
    pub krate: String,
    /// The module path.
    pub module: Vec<String>,
    /// The Rust name.
    pub ident: String,
}

/// Types written as something other than a generated item.
enum Special {
    /// A primitive.
    Prim(&'static str),
    /// `()`.
    Unit,
    /// One of the specification's helpers: Stringified, WithNullValue,
    /// PipeSeparatedFlags.
    Helper(&'static str),
    /// A generic alias, expanded where it is used.
    Expand,
    /// Not a type: a behavior.
    Behavior,
}

/// The items to generate, and what the generator needs to write them.
pub struct Ir<'g> {
    /// The type graph the items come from.
    pub graph: &'g Graph<'g>,
    /// Every item, in the order written.
    pub items: Vec<Item>,
    /// The item each generated specification type became.
    pub item_of: HashMap<usize, ItemId>,
    /// Where every specification type lives in Rust, generated or not.
    pub locations: Vec<Option<Location>>,
    /// The references (item, item, position) that are boxed.
    pub boxed: HashSet<(ItemId, ItemId, Position)>,
    /// The items for endpoints' responses.
    responses: Vec<ItemId>,
    cut: HashSet<(usize, usize)>,
    /// What each reference through a seam becomes, by (from, to) type.
    open: HashMap<(usize, usize), Open>,
    special: Vec<Option<Special>>,
}

/// The type parameter a search response's aggregation results become.
pub const AGGREGATIONS: &str = "TAggregations";

struct Ctx<'c> {
    krate: &'c str,
    module: &'c [String],
    owner: &'c str,
    field: &'c str,
    declaring: usize,
    env: &'c [(TypeName, Ty)],
}

impl<'g> Ir<'g> {
    /// Builds the items for the given crates.
    pub fn build(
        graph: &'g Graph<'g>,
        placement: &'g Placement<'g>,
        crates: &[&str],
    ) -> Result<Ir<'g>, Error> {
        let special: Vec<Option<Special>> =
            (0..graph.types.len()).map(|i| special(graph, i)).collect();
        let mut locations = Vec::with_capacity(graph.types.len());
        let mut seen = HashMap::new();
        for (i, special) in special.iter().enumerate() {
            if special.is_some() {
                locations.push(None);
                continue;
            }
            let name = graph.name(i);
            let krate = placement.krate[i].to_owned();
            let location = Location {
                module: module_path(&name.namespace, &krate),
                ident: pascal(&name.name),
                krate,
            };
            let key = (
                location.krate.clone(),
                location.module.clone(),
                location.ident.clone(),
            );
            if let Some(other) = seen.insert(key, i) {
                return Err(format!(
                    "{} and {} would have the same Rust path",
                    graph.name(other),
                    name
                )
                .into());
            }
            locations.push(Some(location));
        }
        let cut = placement
            .cuts
            .iter()
            .map(|&(_, from, _, to)| (from, to))
            .collect();
        let open = placement
            .cuts
            .iter()
            .filter_map(|&(_, from, _, to)| {
                crate::placement::open_form(placement, from, to).map(|o| ((from, to), o))
            })
            .collect();
        let mut ir = Ir {
            graph,
            items: Vec::new(),
            item_of: HashMap::new(),
            locations,
            boxed: HashSet::new(),
            responses: Vec::new(),
            cut,
            open,
            special,
        };
        for i in 0..graph.types.len() {
            if ir.special[i].is_none() && crates.contains(&placement.krate[i]) {
                ir.build_type(i)?;
            }
        }
        ir.check_names()?;
        ir.propagate(AGGREGATIONS);
        ir.tolerate_responses();
        ir.choose_boxes();
        Ok(ir)
    }

    fn location(&self, i: usize) -> &Location {
        self.locations[i]
            .as_ref()
            .expect("generated types have a location")
    }

    fn push(&mut self, item: Item) -> ItemId {
        self.items.push(item);
        self.items.len() - 1
    }

    fn build_type(&mut self, i: usize) -> Result<(), Error> {
        let graph = self.graph;
        let location = self.location(i).clone();
        let definition = graph.types[i];
        let item = match definition {
            TypeDefinition::Interface(t) => match &t.variants {
                Some(Variants::Container { .. }) => return self.build_container(i, t, &location),
                _ => self.build_struct(i, t, &location)?,
            },
            TypeDefinition::Enum(t) => {
                let mut used = BTreeSet::new();
                let members = t
                    .members
                    .iter()
                    .map(|m| {
                        let mut ident = pascal(m.codegen_name.as_deref().unwrap_or(&m.name));
                        if ident == "Raw" {
                            ident = "RawValue".to_owned();
                        }
                        EnumMember {
                            ident: unique(&mut used, ident),
                            wire: m.name.clone(),
                            aliases: m.aliases.clone(),
                            doc: m.description.clone(),
                        }
                    })
                    .collect();
                Item {
                    krate: location.krate,
                    module: location.module,
                    ident: location.ident,
                    doc: None,
                    generics: Vec::new(),
                    kind: ItemKind::Enum(members),
                }
            }
            TypeDefinition::TypeAlias(t) => self.build_alias(i, t, &location)?,
            TypeDefinition::Request(t) => {
                let ctx = Ctx {
                    krate: &location.krate,
                    module: &location.module,
                    owner: "Request",
                    field: "",
                    declaring: i,
                    env: &[],
                };
                let mut used = BTreeSet::new();
                let mut fields =
                    |props: &[Property], ir: &mut Ir<'g>| -> Result<Vec<Field>, Error> {
                        props
                            .iter()
                            .map(|p| {
                                ir.field(
                                    &Ctx {
                                        field: &p.name,
                                        ..ctx
                                    },
                                    p,
                                    &mut used,
                                )
                            })
                            .collect()
                    };
                let path = fields(&t.path, self)?;
                let query = fields(&t.query, self)?;
                let body = match &t.body {
                    Body::NoBody {} => None,
                    Body::Value { value, .. } => Some(self.ty(
                        &Ctx {
                            field: "body",
                            ..ctx
                        },
                        value,
                    )?),
                    Body::Properties { properties } => {
                        let mut used = BTreeSet::new();
                        let fields = properties
                            .iter()
                            .map(|p| {
                                self.field(
                                    &Ctx {
                                        owner: "RequestBody",
                                        field: &p.name,
                                        ..ctx
                                    },
                                    p,
                                    &mut used,
                                )
                            })
                            .collect::<Result<Vec<_>, Error>>()?;
                        let generics = used_params(fields.iter().map(|f| &f.ty));
                        let args = generics.iter().map(|g| Ty::Param(g.clone())).collect();
                        let body = self.push(Item {
                            krate: location.krate.clone(),
                            module: location.module.clone(),
                            ident: "RequestBody".to_owned(),
                            doc: Some("The request's body.".to_owned()),
                            generics,
                            kind: ItemKind::Struct(Struct {
                                fields,
                                extra: None,
                                keyed: None,
                                shorthand: None,
                                variant: None,
                                serde: true,
                            }),
                        });
                        Some(Ty::Inline { item: body, args })
                    }
                };
                Item {
                    krate: location.krate,
                    module: location.module,
                    ident: location.ident,
                    doc: t.description.clone(),
                    generics: t.generics.iter().map(|g| g.name.clone()).collect(),
                    kind: ItemKind::Request(Request { path, query, body }),
                }
            }
            TypeDefinition::Response(t) => {
                let ctx = Ctx {
                    krate: &location.krate,
                    module: &location.module,
                    owner: "Response",
                    field: "",
                    declaring: i,
                    env: &[],
                };
                let kind = match &t.body {
                    Body::Value { value, .. } => ItemKind::Newtype(self.ty(
                        &Ctx {
                            field: "body",
                            ..ctx
                        },
                        value,
                    )?),
                    Body::NoBody {} => ItemKind::Struct(plain(Vec::new())),
                    Body::Properties { properties } => {
                        let mut used = BTreeSet::new();
                        let fields = properties
                            .iter()
                            .map(|p| {
                                self.field(
                                    &Ctx {
                                        field: &p.name,
                                        ..ctx
                                    },
                                    p,
                                    &mut used,
                                )
                            })
                            .collect::<Result<Vec<_>, Error>>()?;
                        ItemKind::Struct(plain(fields))
                    }
                };
                Item {
                    krate: location.krate,
                    module: location.module,
                    ident: location.ident,
                    doc: None,
                    generics: t.generics.iter().map(|g| g.name.clone()).collect(),
                    kind,
                }
            }
        };
        let is_response = matches!(definition, TypeDefinition::Response(_));
        let id = self.push(item);
        self.item_of.insert(i, id);
        if is_response {
            self.responses.push(id);
        }
        Ok(())
    }

    /// A property as a field.
    fn field(
        &mut self,
        ctx: &Ctx<'_>,
        p: &Property,
        used: &mut BTreeSet<String>,
    ) -> Result<Field, Error> {
        let mut ident = snake(&p.name);
        if used.contains(&ident) && p.name.starts_with('_') {
            ident = format!("_{ident}");
        }
        Ok(Field {
            wire: p.name.clone(),
            ident: unique(used, ident),
            ty: self.ty(ctx, &p.value)?,
            required: p.required,
            optional: !p.required,
            defaulted: false,
            doc: p.description.clone(),
            aliases: p.aliases.clone(),
        })
    }

    /// An interface's properties with its parents', parents first and
    /// overridden by name, each with the type it is declared in and the
    /// generic arguments in force there.
    fn flatten(
        &mut self,
        i: usize,
        env: Vec<(TypeName, Ty)>,
        ctx: &Ctx<'_>,
    ) -> Result<Flat<'g>, Error> {
        let graph = self.graph;
        let TypeDefinition::Interface(t) = graph.types[i] else {
            return Err(format!(
                "{} inherits from a type that isn't an interface",
                graph.name(i)
            )
            .into());
        };
        let mut flat = match &t.inherits {
            Some(parent) => self.parent(i, parent, &env, ctx)?,
            None => Flat::default(),
        };
        for behavior in &t.behaviors {
            flat.behaviors.push((behavior, i, env.clone()));
        }
        for p in &t.properties {
            flat.props.retain(|(q, _, _)| q.name != p.name);
            flat.props.push((p, i, env.clone()));
        }
        Ok(flat)
    }

    fn parent(
        &mut self,
        i: usize,
        parent: &'g Inherits,
        env: &[(TypeName, Ty)],
        ctx: &Ctx<'_>,
    ) -> Result<Flat<'g>, Error> {
        let graph = self.graph;
        let p = graph.find(&parent.type_name).ok_or_else(|| {
            format!(
                "{} inherits {}, which isn't declared",
                graph.name(i),
                parent.type_name
            )
        })?;
        let TypeDefinition::Interface(pt) = graph.types[p] else {
            return Err(format!(
                "{} inherits {}, which isn't an interface",
                graph.name(i),
                parent.type_name
            )
            .into());
        };
        let mut parent_env = Vec::new();
        for (param, arg) in pt.generics.iter().zip(&parent.generics) {
            let arg = self.ty(
                &Ctx {
                    declaring: i,
                    env,
                    ..*ctx
                },
                arg,
            )?;
            parent_env.push((param.clone(), arg));
        }
        self.flatten(p, parent_env, ctx)
    }

    fn build_struct(
        &mut self,
        i: usize,
        t: &'g Interface,
        location: &Location,
    ) -> Result<Item, Error> {
        let ctx = Ctx {
            krate: &location.krate,
            module: &location.module,
            owner: &location.ident,
            field: "",
            declaring: i,
            env: &[],
        };
        let flat = self.flatten(i, Vec::new(), &ctx)?;
        let mut used = BTreeSet::new();
        let mut fields = Vec::new();
        for (p, declaring, env) in &flat.props {
            if matches!(p.value, ValueOf::LiteralValue { .. }) {
                // The tag of the union the type is a member of.
                continue;
            }
            let ctx = Ctx {
                field: &p.name,
                declaring: *declaring,
                env,
                ..ctx
            };
            fields.push(self.field(&ctx, p, &mut used)?);
        }
        let mut extra = None;
        let mut keyed = None;
        for (behavior, declaring, env) in &flat.behaviors {
            let ctx = Ctx {
                declaring: *declaring,
                env,
                ..ctx
            };
            match behavior.type_name.name.as_str() {
                "AdditionalProperties" => {
                    let [k, v] = &behavior.generics[..] else {
                        return Err(format!(
                            "{}: AdditionalProperties needs two arguments",
                            self.graph.name(i)
                        )
                        .into());
                    };
                    let name = behavior
                        .meta
                        .get("fieldname")
                        .map_or("additional", String::as_str);
                    let ident = unique(&mut used, snake(name));
                    let value = self.ty(&Ctx { field: name, ..ctx }, v)?;
                    // Values tagged by their key, such as the aggregations
                    // in a bucket (`sterms#by_tag`), read their tag from it.
                    let value = if self.externally_tagged(v) && !matches!(value, Ty::Json) {
                        Ty::TypedKeys(Box::new(value))
                    } else {
                        value
                    };
                    extra = Some((ident, self.ty(&Ctx { field: name, ..ctx }, k)?, value));
                }
                "AdditionalProperty" => {
                    let [k, v] = &behavior.generics[..] else {
                        return Err(format!(
                            "{}: AdditionalProperty needs two arguments",
                            self.graph.name(i)
                        )
                        .into());
                    };
                    let key = behavior.meta.get("key").map_or("key", String::as_str);
                    let value = behavior.meta.get("value").map_or("value", String::as_str);
                    let key_ident = unique(&mut used, snake(key));
                    let value_ident = unique(&mut used, snake(value));
                    keyed = Some((
                        key_ident,
                        value_ident,
                        self.ty(&Ctx { field: key, ..ctx }, k)?,
                        self.ty(
                            &Ctx {
                                field: value,
                                ..ctx
                            },
                            v,
                        )?,
                    ));
                }
                _ => {}
            }
        }
        let shorthand = match &t.shortcut_property {
            Some(name) => {
                let index = fields.iter().position(|f| &f.wire == name).ok_or_else(|| {
                    format!("{}: no shorthand property {name}", self.graph.name(i))
                })?;
                let p = flat
                    .props
                    .iter()
                    .find(|(p, _, _)| &p.name == name)
                    .expect("the field came from a property")
                    .0;
                let kinds = shapes::json_kinds(self.graph, &p.value, &[], 0);
                let write_short = kinds & OBJECT == 0 && !self.mentions_cut(i, &p.value);
                let others_optional = fields
                    .iter()
                    .enumerate()
                    .all(|(n, f)| n == index || !f.required);
                others_optional.then_some((index, write_short))
            }
            None => None,
        };
        Ok(Item {
            krate: location.krate.clone(),
            module: location.module.clone(),
            ident: location.ident.clone(),
            doc: t.description.clone(),
            generics: t.generics.iter().map(|g| g.name.clone()).collect(),
            kind: ItemKind::Struct(Struct {
                fields,
                extra,
                keyed,
                shorthand,
                variant: None,
                serde: true,
            }),
        })
    }

    /// Whether a value refers, at its top level, to a type through a seam.
    fn mentions_cut(&self, declaring: usize, value: &ValueOf) -> bool {
        let mut found = false;
        walk(value, &mut |v| {
            if let ValueOf::InstanceOf { type_name, .. } = v
                && let Some(t) = self.graph.find(type_name)
            {
                found |= self.cut.contains(&(declaring, t));
            }
        });
        found
    }

    fn build_container(
        &mut self,
        i: usize,
        t: &'g Interface,
        location: &Location,
    ) -> Result<(), Error> {
        if !t.generics.is_empty() {
            return Err(format!(
                "{} is a generic container, which the generator doesn't support yet",
                self.graph.name(i)
            )
            .into());
        }
        let ctx = Ctx {
            krate: &location.krate,
            module: &location.module,
            owner: &location.ident,
            field: "",
            declaring: i,
            env: &[],
        };
        let flat = self.flatten(i, Vec::new(), &ctx)?;
        let mut props = Vec::new();
        let mut variants = Vec::new();
        let mut used_fields = BTreeSet::new();
        let mut used_variants = BTreeSet::new();
        used_variants.insert("Raw".to_owned());
        for (p, declaring, env) in &flat.props {
            let ctx = Ctx {
                field: &p.name,
                declaring: *declaring,
                env,
                ..ctx
            };
            // The specification marks a container's own properties that sit
            // beside the variant; inherited ones, such as a query's `boost`
            // and `_name`, always do.
            if p.container_property || *declaring != i {
                props.push(self.field(&ctx, p, &mut used_fields)?);
            } else {
                variants.push(Variant {
                    wire: p.name.clone(),
                    aliases: p.aliases.clone(),
                    ident: unique(&mut used_variants, pascal(&p.name)),
                    ty: self.ty(&ctx, &p.value)?,
                    doc: p.description.clone(),
                });
            }
        }
        let mut keyed = None;
        for (behavior, declaring, env) in &flat.behaviors {
            if behavior.type_name.name == "AdditionalProperty" {
                let [k, v] = &behavior.generics[..] else {
                    return Err(format!(
                        "{}: AdditionalProperty needs two arguments",
                        self.graph.name(i)
                    )
                    .into());
                };
                let ctx = Ctx {
                    declaring: *declaring,
                    env,
                    ..ctx
                };
                let key = behavior.meta.get("key").map_or("key", String::as_str);
                keyed = Some((
                    unique(&mut used_variants, pascal(key)),
                    self.ty(&ctx, k)?,
                    self.ty(&ctx, v)?,
                ));
            }
        }
        let container = Container { variants, keyed };
        if props.is_empty() {
            let id = self.push(Item {
                krate: location.krate.clone(),
                module: location.module.clone(),
                ident: location.ident.clone(),
                doc: t.description.clone(),
                generics: Vec::new(),
                kind: ItemKind::Container(container),
            });
            self.item_of.insert(i, id);
            return Ok(());
        }
        let enum_id = self.push(Item {
            krate: location.krate.clone(),
            module: location.module.clone(),
            ident: format!("{}Variant", location.ident),
            doc: Some(format!("The variants of [`{}`].", location.ident)),
            generics: Vec::new(),
            kind: ItemKind::Container(container),
        });
        let field = unique(&mut used_fields, "variant".to_owned());
        let id = self.push(Item {
            krate: location.krate.clone(),
            module: location.module.clone(),
            ident: location.ident.clone(),
            doc: t.description.clone(),
            generics: Vec::new(),
            kind: ItemKind::Struct(Struct {
                fields: props,
                extra: None,
                keyed: None,
                shorthand: None,
                variant: Some((field, enum_id)),
                serde: true,
            }),
        });
        self.item_of.insert(i, id);
        Ok(())
    }

    fn build_alias(
        &mut self,
        i: usize,
        t: &'g TypeAlias,
        location: &Location,
    ) -> Result<Item, Error> {
        let ctx = Ctx {
            krate: &location.krate,
            module: &location.module,
            owner: &location.ident,
            field: "",
            declaring: i,
            env: &[],
        };
        let generics: Vec<String> = t.generics.iter().map(|g| g.name.clone()).collect();
        let item = |kind| Item {
            krate: location.krate.clone(),
            module: location.module.clone(),
            ident: location.ident.clone(),
            doc: t.description.clone(),
            generics: generics.clone(),
            kind,
        };
        let ValueOf::UnionOf { items } = &t.value else {
            return Ok(item(ItemKind::Alias(self.ty(&ctx, &t.value)?)));
        };
        let kind = match &t.variants {
            Some(Variants::InternalTag {
                tag, default_tag, ..
            }) => ItemKind::Tagged {
                tag: tag.clone(),
                default: default_tag.clone(),
                members: self.tagged_members(&ctx, items, &location.ident, Some(tag))?,
            },
            Some(Variants::ExternalTag { .. }) => {
                ItemKind::External(self.tagged_members(&ctx, items, &location.ident, None)?)
            }
            Some(Variants::Untagged { untyped_variant }) => {
                let members = self.tagged_members(&ctx, items, &location.ident, None)?;
                let fallback = items
                    .iter()
                    .position(|m| matches!(m, ValueOf::InstanceOf { type_name, .. } if type_name == untyped_variant))
                    .ok_or_else(|| format!("{}: its untyped variant isn't a member", self.graph.name(i)))?;
                ItemKind::Untagged { members, fallback }
            }
            Some(Variants::Container { .. }) => {
                return Err(
                    format!("{} is an alias with container variants", self.graph.name(i)).into(),
                );
            }
            None => {
                if let Some(one) = self.one_or_many(&ctx, items)? {
                    ItemKind::Alias(one)
                } else {
                    ItemKind::Union(self.union_members(
                        &ctx,
                        items,
                        &location.ident,
                        &t.codegen_names,
                    )?)
                }
            }
        };
        Ok(item(kind))
    }

    /// The members of a tagged union, named by their variant names.
    /// The members of a tagged union. A member's tag is the literal its tag
    /// property holds (`"type": "text"` for a text mapping), else its variant
    /// name, else its type's name.
    fn tagged_members(
        &mut self,
        ctx: &Ctx<'_>,
        items: &[ValueOf],
        union: &str,
        tag: Option<&str>,
    ) -> Result<Vec<Variant>, Error> {
        let mut used = BTreeSet::new();
        used.insert("Raw".to_owned());
        let mut members = Vec::new();
        for item in items {
            let ValueOf::InstanceOf { type_name, .. } = item else {
                return Err(format!("{union}: a tagged union's member isn't a type").into());
            };
            let member = self.graph.find(type_name);
            let interface = member.and_then(|m| match self.graph.types[m] {
                TypeDefinition::Interface(t) => Some(t),
                _ => None,
            });
            let literal = interface.zip(tag).and_then(|(t, tag)| {
                t.properties.iter().find_map(|p| match &p.value {
                    ValueOf::LiteralValue {
                        value: Literal::String(value),
                    } if p.name == tag => Some(value.clone()),
                    _ => None,
                })
            });
            let wire = literal
                .or_else(|| interface.and_then(|t| t.variant_name.clone()))
                .unwrap_or_else(|| snake(&type_name.name));
            let name = match type_name.name.strip_suffix(union) {
                Some(stripped) if !stripped.is_empty() => stripped.to_owned(),
                _ => wire.clone(),
            };
            members.push(Variant {
                ident: unique(&mut used, pascal(&name)),
                wire,
                aliases: Vec::new(),
                ty: self.ty(ctx, item)?,
                doc: None,
            });
        }
        Ok(members)
    }

    /// `T | T[]` as `OneOrMany<T>`, unless `T` can itself be an array.
    fn one_or_many(&mut self, ctx: &Ctx<'_>, items: &[ValueOf]) -> Result<Option<Ty>, Error> {
        if let [one, ValueOf::ArrayOf { value: many }] = items
            && format!("{one:?}") == format!("{many:?}")
            && shapes::json_kinds(self.graph, one, &[], 0) & shapes::ARRAY == 0
            && !self.mentions_cut(ctx.declaring, one)
        {
            return Ok(Some(Ty::OneOrMany(Box::new(self.ty(ctx, one)?))));
        }
        Ok(None)
    }

    fn union_members(
        &mut self,
        ctx: &Ctx<'_>,
        items: &[ValueOf],
        union: &str,
        codegen_names: &[String],
    ) -> Result<Vec<UnionMember>, Error> {
        let mut used = BTreeSet::new();
        let mut members = Vec::new();
        for (n, item) in items.iter().enumerate() {
            let name = codegen_names
                .get(n)
                .map(|c| pascal(c))
                .unwrap_or_else(|| self.member_name(item, union));
            let (ty, literal, kinds) = match item {
                ValueOf::LiteralValue { value } => {
                    let kinds = match value {
                        Literal::String(_) => shapes::STRING,
                        Literal::Number(_) => shapes::NUMBER,
                        Literal::Boolean(_) => shapes::BOOLEAN,
                    };
                    (None, Some(value.clone()), kinds)
                }
                _ => {
                    let kinds = if self.mentions_cut(ctx.declaring, item) {
                        ANY
                    } else {
                        shapes::json_kinds(self.graph, item, &[], 0)
                    };
                    (Some(self.ty(ctx, item)?), None, kinds)
                }
            };
            members.push(UnionMember {
                ident: unique(&mut used, name),
                ty,
                literal,
                kinds,
                keys: self.object_keys(item),
                string_enum: matches!(item, ValueOf::InstanceOf { type_name, .. }
                    if self.graph.find(type_name).is_some_and(|t| matches!(self.graph.types[t], TypeDefinition::Enum(_)))),
            });
        }
        Ok(members)
    }

    fn member_name(&self, item: &ValueOf, union: &str) -> String {
        match item {
            ValueOf::InstanceOf { type_name, .. } => {
                let name = match type_name.name.strip_suffix(union) {
                    Some(stripped)
                        if !stripped.is_empty() && type_name.namespace != "_builtins" =>
                    {
                        stripped
                    }
                    _ => &type_name.name,
                };
                match name {
                    "boolean" => "Bool".to_owned(),
                    other => pascal(other),
                }
            }
            ValueOf::ArrayOf { value } => format!("{}List", self.member_name(value, union)),
            ValueOf::DictionaryOf { .. } => "Map".to_owned(),
            ValueOf::UnionOf { .. } => "Union".to_owned(),
            ValueOf::UserDefinedValue {} => "Value".to_owned(),
            ValueOf::LiteralValue { value } => match value {
                Literal::String(s) => pascal(s),
                Literal::Number(n) => literal_variant(&n.to_string()),
                Literal::Boolean(b) => literal_variant(&b.to_string()),
            },
        }
    }

    /// The properties an object member may have, to tell it from other
    /// object members.
    fn object_keys(&self, item: &ValueOf) -> Option<BTreeSet<String>> {
        let ValueOf::InstanceOf { type_name, .. } = item else {
            return None;
        };
        let mut i = self.graph.find(type_name)?;
        let mut keys = BTreeSet::new();
        loop {
            match self.graph.types[i] {
                TypeDefinition::Interface(t) => {
                    keys.extend(t.properties.iter().flat_map(|p| {
                        std::iter::once(p.name.clone()).chain(p.aliases.iter().cloned())
                    }));
                    match &t.inherits {
                        Some(parent) => i = self.graph.find(&parent.type_name)?,
                        None => return Some(keys),
                    }
                }
                _ => return None,
            }
        }
    }

    /// The Rust type for a value.
    fn ty(&mut self, ctx: &Ctx<'_>, value: &ValueOf) -> Result<Ty, Error> {
        Ok(match value {
            ValueOf::InstanceOf {
                type_name,
                generics,
            } => {
                if type_name.namespace == "_builtins" {
                    return Ok(match type_name.name.as_str() {
                        "string" | "binary" => Ty::Prim("::std::string::String"),
                        "boolean" => Ty::Prim("bool"),
                        "number" => Ty::Prim("f64"),
                        "null" | "void" => Ty::Unit,
                        other => return Err(format!("unknown primitive {other}").into()),
                    });
                }
                if let Some((_, bound)) = ctx.env.iter().find(|(name, _)| name == type_name) {
                    return Ok(bound.clone());
                }
                let Some(target) = self.graph.find(type_name) else {
                    let declaring = self.graph.name(ctx.declaring);
                    if type_name.namespace == format!("{}.{}", declaring.namespace, declaring.name)
                    {
                        return Ok(Ty::Param(type_name.name.clone()));
                    }
                    // A parent's parameter left unbound.
                    return Ok(Ty::Json);
                };
                if self.cut.contains(&(ctx.declaring, target)) {
                    return Ok(Ty::Json);
                }
                let args = generics
                    .iter()
                    .map(|g| self.ty(ctx, g))
                    .collect::<Result<Vec<_>, _>>()?;
                match &self.special[target] {
                    Some(Special::Prim(p)) => Ty::Prim(p),
                    Some(Special::Unit) => Ty::Unit,
                    Some(Special::Helper(helper)) => {
                        let arg = Box::new(args.into_iter().next().unwrap_or(Ty::Json));
                        match *helper {
                            "Stringified" => Ty::Stringified(arg),
                            "WithNullValue" => Ty::WithNull(arg),
                            _ => Ty::Flags(arg),
                        }
                    }
                    Some(Special::Expand) => {
                        let TypeDefinition::TypeAlias(alias) = self.graph.types[target] else {
                            unreachable!("only aliases are expanded");
                        };
                        let env: Vec<(TypeName, Ty)> =
                            alias.generics.iter().cloned().zip(args).collect();
                        self.ty(
                            &Ctx {
                                declaring: target,
                                env: &env,
                                ..*ctx
                            },
                            &alias.value,
                        )?
                    }
                    Some(Special::Behavior) => Ty::Json,
                    None => Ty::Named { target, args },
                }
            }
            ValueOf::ArrayOf { value } => Ty::Vec(Box::new(self.ty(ctx, value)?)),
            ValueOf::DictionaryOf {
                key,
                value,
                single_key,
            } => {
                // A map of aggregations through the search seam is open as a
                // whole.
                if let ValueOf::InstanceOf { type_name, .. } = &**value
                    && let Some(target) = self.graph.find(type_name)
                {
                    match self.open.get(&(ctx.declaring, target)) {
                        Some(Open::AggsBody) => return Ok(Ty::Prim("::oxsearch_core::AggsBody")),
                        Some(Open::AggsParam) => return Ok(Ty::Param(AGGREGATIONS.to_owned())),
                        Some(Open::Json) | None => {}
                    }
                }
                let k = self.ty(ctx, key)?;
                let v = self.ty(ctx, value)?;
                if *single_key {
                    Ty::SingleKey(Box::new(k), Box::new(v))
                } else if self.externally_tagged(value) && !matches!(v, Ty::Json) {
                    Ty::TypedKeys(Box::new(v))
                } else {
                    Ty::Map(Box::new(k), Box::new(v))
                }
            }
            ValueOf::UnionOf { items } => {
                if let Some(one) = self.one_or_many(ctx, items)? {
                    return Ok(one);
                }
                // `T | null` is an `Option<T>`.
                if let [a, b] = &items[..] {
                    match (self.is_null(a), self.is_null(b)) {
                        (false, true) => return Ok(Ty::Opt(Box::new(self.ty(ctx, a)?))),
                        (true, false) => return Ok(Ty::Opt(Box::new(self.ty(ctx, b)?))),
                        _ => {}
                    }
                }
                let base = format!("{}{}", ctx.owner, pascal(ctx.field));
                let members = self.union_members(ctx, items, &base, &[])?;
                let generics = used_params(members.iter().filter_map(|m| m.ty.as_ref()));
                let args = generics.iter().map(|g| Ty::Param(g.clone())).collect();
                let item = self.push(Item {
                    krate: ctx.krate.to_owned(),
                    module: ctx.module.to_vec(),
                    ident: base,
                    doc: None,
                    generics,
                    kind: ItemKind::Union(members),
                });
                Ty::Inline { item, args }
            }
            ValueOf::UserDefinedValue {} => Ty::Json,
            ValueOf::LiteralValue { value } => match value {
                Literal::String(_) => Ty::Prim("::std::string::String"),
                Literal::Number(_) => Ty::Prim("f64"),
                Literal::Boolean(_) => Ty::Prim("bool"),
            },
        })
    }

    /// Whether a value is `null`: the primitive, or the specification's
    /// `NullValue`.
    fn is_null(&self, value: &ValueOf) -> bool {
        let ValueOf::InstanceOf { type_name, .. } = value else {
            return false;
        };
        (type_name.namespace == "_builtins" && type_name.name == "null")
            || self.graph.find(type_name).is_some_and(|t| {
                matches!(self.special[t], Some(Special::Unit)) && type_name.name == "NullValue"
            })
    }

    /// Whether a value (or each element of an array value) is an externally
    /// tagged union.
    fn externally_tagged(&self, value: &ValueOf) -> bool {
        match value {
            ValueOf::ArrayOf { value } => self.externally_tagged(value),
            ValueOf::InstanceOf { type_name, .. } => self.graph.find(type_name).is_some_and(|t| {
                matches!(
                    self.graph.types[t],
                    TypeDefinition::TypeAlias(TypeAlias {
                        variants: Some(Variants::ExternalTag { .. }),
                        ..
                    })
                )
            }),
            _ => false,
        }
    }

    /// Makes inline union names unique within their module.
    fn check_names(&mut self) -> Result<(), Error> {
        let mut seen: HashMap<(String, Vec<String>), BTreeSet<String>> = HashMap::new();
        for item in &mut self.items {
            let names = seen
                .entry((item.krate.clone(), item.module.clone()))
                .or_default();
            if !names.insert(item.ident.clone()) {
                let mut n = 2;
                while !names.insert(format!("{}{n}", item.ident)) {
                    n += 1;
                }
                item.ident = format!("{}{n}", item.ident);
            }
        }
        Ok(())
    }

    /// Adds a type parameter a seam introduced to every item that contains
    /// it, directly or through another item, and passes it on in their
    /// references: `search::ResponseBody<TDocument, TAggregations>` makes
    /// `search::Response` generic in it too.
    fn propagate(&mut self, param: &str) {
        loop {
            let having: HashSet<ItemId> = (0..self.items.len())
                .filter(|&id| self.items[id].generics.iter().any(|g| g == param))
                .collect();
            let mut changed = false;
            for id in 0..self.items.len() {
                let mut mentions = false;
                let item_of = &self.item_of;
                for ty in item_types_mut(&mut self.items[id].kind) {
                    mentions |= add_param(ty, param, &having, item_of);
                }
                if mentions && !self.items[id].generics.iter().any(|g| g == param) {
                    self.items[id].generics.push(param.to_owned());
                    changed = true;
                }
            }
            if !changed {
                return;
            }
        }
    }

    /// Types read from responses tolerate a missing required property,
    /// because Elasticsearch leaves out some the specification requires: it
    /// reads as `None`, or as empty for an array or a map. Constructors
    /// still take required properties.
    fn tolerate_responses(&mut self) {
        let mut seen = vec![false; self.items.len()];
        let mut stack = self.responses.clone();
        while let Some(id) = stack.pop() {
            if std::mem::replace(&mut seen[id], true) {
                continue;
            }
            stack.extend(self.references(id).into_iter().filter(|&t| !seen[t]));
        }
        for (id, item) in self.items.iter_mut().enumerate() {
            if !seen[id] {
                continue;
            }
            if let ItemKind::Struct(s) = &mut item.kind {
                for field in s.fields.iter_mut().filter(|f| f.required) {
                    if matches!(field.ty, Ty::Vec(_) | Ty::Map(..) | Ty::TypedKeys(_)) {
                        field.defaulted = true;
                    } else {
                        field.optional = true;
                    }
                }
            }
        }
    }

    /// Every generated item an item refers to.
    pub fn references(&self, id: ItemId) -> Vec<ItemId> {
        let mut out = Vec::new();
        for ty in self.item_types(id) {
            self.all_targets(ty, &mut out);
        }
        if let ItemKind::Struct(s) = &self.items[id].kind
            && let Some((_, variant)) = &s.variant
        {
            out.push(*variant);
        }
        out
    }

    /// Every type an item mentions.
    fn item_types(&self, id: ItemId) -> Vec<&Ty> {
        match &self.items[id].kind {
            ItemKind::Struct(s) => {
                let mut out: Vec<&Ty> = s.fields.iter().map(|f| &f.ty).collect();
                if let Some((_, k, v)) = &s.extra {
                    out.extend([k, v]);
                }
                if let Some((_, _, k, v)) = &s.keyed {
                    out.extend([k, v]);
                }
                out
            }
            ItemKind::Container(c) => {
                let mut out: Vec<&Ty> = c.variants.iter().map(|v| &v.ty).collect();
                if let Some((_, k, v)) = &c.keyed {
                    out.extend([k, v]);
                }
                out
            }
            ItemKind::Union(members) => members.iter().filter_map(|m| m.ty.as_ref()).collect(),
            ItemKind::Tagged { members, .. }
            | ItemKind::External(members)
            | ItemKind::Untagged { members, .. } => members.iter().map(|v| &v.ty).collect(),
            ItemKind::Alias(ty) | ItemKind::Newtype(ty) => vec![ty],
            ItemKind::Request(r) => r
                .path
                .iter()
                .chain(&r.query)
                .map(|f| &f.ty)
                .chain(&r.body)
                .collect(),
            ItemKind::Enum(_) => Vec::new(),
        }
    }

    /// Every generated item a type mentions, by value or not.
    fn all_targets(&self, ty: &Ty, out: &mut Vec<ItemId>) {
        match ty {
            Ty::Named { target, args } => {
                if let Some(&item) = self.item_of.get(target) {
                    out.push(item);
                }
                args.iter().for_each(|a| self.all_targets(a, out));
            }
            Ty::Inline { item, args } => {
                out.push(*item);
                args.iter().for_each(|a| self.all_targets(a, out));
            }
            Ty::Vec(t)
            | Ty::OneOrMany(t)
            | Ty::Stringified(t)
            | Ty::WithNull(t)
            | Ty::Flags(t)
            | Ty::Opt(t)
            | Ty::TypedKeys(t) => self.all_targets(t, out),
            Ty::Map(k, v) | Ty::SingleKey(k, v) => {
                self.all_targets(k, out);
                self.all_targets(v, out);
            }
            Ty::Prim(_) | Ty::Unit | Ty::Json | Ty::Param(_) => {}
        }
    }

    /// The items a type refers to by value, looking through aliases.
    pub fn direct_targets_of(&self, ty: &Ty, out: &mut Vec<ItemId>) {
        match ty {
            Ty::Named { target, args } => {
                if let Some(&item) = self.item_of.get(target) {
                    match &self.items[item].kind {
                        ItemKind::Alias(inner) => self.direct_targets_of(inner, out),
                        _ => out.push(item),
                    }
                }
                for arg in args {
                    self.direct_targets_of(arg, out);
                }
            }
            Ty::Inline { item, args } => {
                out.push(*item);
                for arg in args {
                    self.direct_targets_of(arg, out);
                }
            }
            Ty::SingleKey(k, v) => {
                self.direct_targets_of(k, out);
                self.direct_targets_of(v, out);
            }
            Ty::OneOrMany(t) | Ty::Stringified(t) | Ty::WithNull(t) | Ty::Flags(t) | Ty::Opt(t) => {
                self.direct_targets_of(t, out)
            }
            Ty::Vec(_)
            | Ty::Map(..)
            | Ty::TypedKeys(_)
            | Ty::Prim(_)
            | Ty::Unit
            | Ty::Json
            | Ty::Param(_) => {}
        }
    }

    /// Each item's by-value references: (target, position).
    fn by_value(&self) -> Vec<Vec<(ItemId, Position)>> {
        self.items
            .iter()
            .map(|item| {
                let mut out = Vec::new();
                let add = |ty: &Ty, position: Position, out: &mut Vec<(ItemId, Position)>| {
                    let mut targets = Vec::new();
                    self.direct_targets_of(ty, &mut targets);
                    out.extend(targets.into_iter().map(|t| (t, position)));
                };
                match &item.kind {
                    ItemKind::Struct(s) => {
                        for f in &s.fields {
                            add(&f.ty, Position::Field, &mut out);
                        }
                        if let Some((_, _, k, v)) = &s.keyed {
                            add(k, Position::Field, &mut out);
                            add(v, Position::Field, &mut out);
                        }
                        if let Some((_, variant)) = &s.variant {
                            out.push((*variant, Position::Field));
                        }
                    }
                    ItemKind::Request(r) => {
                        for f in r.path.iter().chain(&r.query) {
                            add(&f.ty, Position::Field, &mut out);
                        }
                        if let Some(body) = &r.body {
                            add(body, Position::Field, &mut out);
                        }
                    }
                    ItemKind::Container(c) => {
                        for v in &c.variants {
                            add(&v.ty, Position::Variant, &mut out);
                        }
                        if let Some((_, k, v)) = &c.keyed {
                            add(k, Position::Variant, &mut out);
                            add(v, Position::Variant, &mut out);
                        }
                    }
                    ItemKind::Union(members) => {
                        for m in members {
                            if let Some(ty) = &m.ty {
                                add(ty, Position::Variant, &mut out);
                            }
                        }
                    }
                    ItemKind::Tagged { members, .. }
                    | ItemKind::External(members)
                    | ItemKind::Untagged { members, .. } => {
                        for v in members {
                            add(&v.ty, Position::Variant, &mut out);
                        }
                    }
                    ItemKind::Newtype(ty) => add(ty, Position::Field, &mut out),
                    ItemKind::Enum(_) | ItemKind::Alias(_) => {}
                }
                out.sort();
                out.dedup();
                out
            })
            .collect()
    }

    /// Boxes references until no type contains itself by value: first enum
    /// variants' payloads that are structs, then any variant payload, then
    /// struct fields.
    fn choose_boxes(&mut self) {
        let edges = self.by_value();
        let is_struct = |item: &Item| {
            matches!(
                item.kind,
                ItemKind::Struct(_) | ItemKind::Request(_) | ItemKind::Newtype(_)
            )
        };
        loop {
            let adjacency: Vec<Vec<usize>> = edges
                .iter()
                .enumerate()
                .map(|(from, out)| {
                    out.iter()
                        .filter(|&&(to, position)| !self.boxed.contains(&(from, to, position)))
                        .map(|&(to, _)| to)
                        .collect()
                })
                .collect();
            let components = graph::cycles(&adjacency);
            if components.is_empty() {
                return;
            }
            for component in components {
                let members: BTreeSet<usize> = component.iter().copied().collect();
                let open: Vec<(usize, usize, Position)> = component
                    .iter()
                    .flat_map(|&from| edges[from].iter().map(move |&(to, p)| (from, to, p)))
                    .filter(|&(from, to, p)| {
                        members.contains(&to) && !self.boxed.contains(&(from, to, p))
                    })
                    .collect();
                let tiers: [Tier<'_>; 3] = [
                    &|&(_, to, p)| p == Position::Variant && is_struct(&self.items[to]),
                    &|&(_, _, p)| p == Position::Variant,
                    &|_| true,
                ];
                for tier in tiers {
                    let chosen: Vec<_> = open.iter().copied().filter(|e| tier(e)).collect();
                    if !chosen.is_empty() {
                        self.boxed.extend(chosen);
                        break;
                    }
                }
            }
        }
    }
}

/// The generic arguments in force where a property is declared.
type Env = Vec<(TypeName, Ty)>;

/// A rule choosing which references in a loop to box.
type Tier<'a> = &'a dyn Fn(&(usize, usize, Position)) -> bool;

/// Properties, behaviors, each with the type they are declared in and the
/// generic arguments in force there.
#[derive(Default)]
struct Flat<'g> {
    props: Vec<(&'g Property, usize, Env)>,
    behaviors: Vec<(&'g Inherits, usize, Env)>,
}

fn plain(fields: Vec<Field>) -> Struct {
    Struct {
        fields,
        extra: None,
        keyed: None,
        shorthand: None,
        variant: None,
        serde: true,
    }
}

fn special(graph: &Graph<'_>, i: usize) -> Option<Special> {
    let name = graph.name(i);
    match (name.namespace.as_str(), name.name.as_str()) {
        ("_types", "integer") => Some(Special::Prim("i32")),
        ("_types", "long") => Some(Special::Prim("i64")),
        ("_types", "short") => Some(Special::Prim("i16")),
        ("_types", "byte") => Some(Special::Prim("i8")),
        ("_types", "uint") => Some(Special::Prim("u32")),
        ("_types", "ulong") => Some(Special::Prim("u64")),
        ("_types", "float") => Some(Special::Prim("f32")),
        ("_types", "double") => Some(Special::Prim("f64")),
        ("_spec_utils", "Stringified") => Some(Special::Helper("Stringified")),
        ("_spec_utils", "WithNullValue") => Some(Special::Helper("WithNullValue")),
        ("_spec_utils", "PipeSeparatedFlags") => Some(Special::Helper("PipeSeparatedFlags")),
        ("_spec_utils", "NullValue" | "Void") => Some(Special::Unit),
        (
            "_spec_utils",
            "AdditionalProperties"
            | "AdditionalProperty"
            | "OverloadOf"
            | "CommonQueryParameters"
            | "CommonCatQueryParameters",
        ) => Some(Special::Behavior),
        _ => match graph.types[i] {
            TypeDefinition::TypeAlias(t)
                if !t.generics.is_empty()
                    && t.variants.is_none()
                    && !matches!(t.value, ValueOf::UnionOf { .. }) =>
            {
                Some(Special::Expand)
            }
            _ => None,
        },
    }
}

/// A namespace's module path in a crate: `_global` and `_types` parts are
/// dropped, and so is the crate's own area name.
pub fn module_path(namespace: &str, krate: &str) -> Vec<String> {
    let home = match krate {
        "core" | "search" | "docs" => None,
        "aggs" => Some("aggregations"),
        other => Some(other),
    };
    let mut parts: Vec<&str> = namespace
        .split('.')
        .filter(|p| *p != "_global" && *p != "_types")
        .collect();
    if home.is_some() && parts.first().copied() == home {
        parts.remove(0);
    }
    parts
        .iter()
        .map(|p| snake(p.trim_start_matches('_')))
        .collect()
}

fn unique(used: &mut BTreeSet<String>, name: String) -> String {
    if used.insert(name.clone()) {
        return name;
    }
    let mut n = 2;
    loop {
        let candidate = format!("{name}{n}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

/// The generic parameters types mention, in order of first mention.
fn used_params<'t>(types: impl Iterator<Item = &'t Ty>) -> Vec<String> {
    fn visit(ty: &Ty, out: &mut Vec<String>) {
        match ty {
            Ty::Param(p) => {
                if !out.contains(p) {
                    out.push(p.clone());
                }
            }
            Ty::Named { args, .. } | Ty::Inline { args, .. } => {
                args.iter().for_each(|a| visit(a, out))
            }
            Ty::Vec(t)
            | Ty::OneOrMany(t)
            | Ty::Stringified(t)
            | Ty::WithNull(t)
            | Ty::Flags(t)
            | Ty::Opt(t)
            | Ty::TypedKeys(t) => visit(t, out),
            Ty::Map(k, v) | Ty::SingleKey(k, v) => {
                visit(k, out);
                visit(v, out);
            }
            Ty::Prim(_) | Ty::Unit | Ty::Json => {}
        }
    }
    let mut out = Vec::new();
    for ty in types {
        visit(ty, &mut out);
    }
    out
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

/// The generic parameters a type uses.
pub fn params_in(ty: &Ty) -> Vec<String> {
    used_params(std::iter::once(ty))
}

/// The generic parameters any of the types use.
pub fn params_in_all<'t>(types: impl Iterator<Item = &'t Ty>) -> Vec<String> {
    used_params(types)
}

/// A crate's name in Rust paths.
pub fn crate_path(krate: &str) -> String {
    format!("oxsearch_{}", krate.replace('-', "_"))
}

/// Every type in an item, to change.
fn item_types_mut(kind: &mut ItemKind) -> Vec<&mut Ty> {
    match kind {
        ItemKind::Struct(s) => {
            let mut out: Vec<&mut Ty> = s.fields.iter_mut().map(|f| &mut f.ty).collect();
            if let Some((_, k, v)) = &mut s.extra {
                out.push(k);
                out.push(v);
            }
            if let Some((_, _, k, v)) = &mut s.keyed {
                out.push(k);
                out.push(v);
            }
            out
        }
        ItemKind::Container(c) => {
            let mut out: Vec<&mut Ty> = c.variants.iter_mut().map(|v| &mut v.ty).collect();
            if let Some((_, k, v)) = &mut c.keyed {
                out.push(k);
                out.push(v);
            }
            out
        }
        ItemKind::Union(members) => members.iter_mut().filter_map(|m| m.ty.as_mut()).collect(),
        ItemKind::Tagged { members, .. }
        | ItemKind::External(members)
        | ItemKind::Untagged { members, .. } => members.iter_mut().map(|v| &mut v.ty).collect(),
        ItemKind::Alias(ty) | ItemKind::Newtype(ty) => vec![ty],
        ItemKind::Request(r) => r
            .path
            .iter_mut()
            .chain(r.query.iter_mut())
            .map(|f| &mut f.ty)
            .chain(r.body.as_mut())
            .collect(),
        ItemKind::Enum(_) => Vec::new(),
    }
}

/// Passes `param` on in a type's references to items that take it, and
/// says whether the type mentions it afterwards.
fn add_param(
    ty: &mut Ty,
    param: &str,
    having: &HashSet<ItemId>,
    item_of: &HashMap<usize, ItemId>,
) -> bool {
    match ty {
        Ty::Param(p) => p == param,
        Ty::Named { target, args } => {
            let takes = item_of.get(target).is_some_and(|i| having.contains(i));
            let mut mentions = false;
            for arg in args.iter_mut() {
                mentions |= add_param(arg, param, having, item_of);
            }
            if takes && !args.iter().any(|a| matches!(a, Ty::Param(p) if p == param)) {
                args.push(Ty::Param(param.to_owned()));
                mentions = true;
            }
            mentions
        }
        Ty::Inline { item, args } => {
            let takes = having.contains(item);
            let mut mentions = false;
            for arg in args.iter_mut() {
                mentions |= add_param(arg, param, having, item_of);
            }
            if takes && !args.iter().any(|a| matches!(a, Ty::Param(p) if p == param)) {
                args.push(Ty::Param(param.to_owned()));
                mentions = true;
            }
            mentions
        }
        Ty::Vec(t)
        | Ty::OneOrMany(t)
        | Ty::Stringified(t)
        | Ty::WithNull(t)
        | Ty::Flags(t)
        | Ty::Opt(t)
        | Ty::TypedKeys(t) => add_param(t, param, having, item_of),
        Ty::Map(k, v) | Ty::SingleKey(k, v) => {
            let a = add_param(k, param, having, item_of);
            let b = add_param(v, param, having, item_of);
            a || b
        }
        Ty::Prim(_) | Ty::Unit | Ty::Json => false,
    }
}
