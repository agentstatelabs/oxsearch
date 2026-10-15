//! Writes the intermediate form as Rust source, one module at a time.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use super::ir::{
    Field, Ir, Item, ItemId, ItemKind, Position, Request, Struct, Ty, UnionMember, Variant,
    params_in,
};
use super::names::{bare, doc, string};
use crate::shapes::{ARRAY, BOOLEAN, NULL, NUMBER, OBJECT, STRING};
use crate::spec::model::Literal;

const CORE: &str = "::oxsearch_core";
const VALUE: &str = "::serde_json::Value";
const RESULT: &str = "::core::result::Result";
const OPTION: &str = "::core::option::Option";

/// The source of every module, by crate and module path.
pub fn emit(ir: &Ir<'_>) -> BTreeMap<(String, Vec<String>), String> {
    let mut modules: BTreeMap<(String, Vec<String>), String> = BTreeMap::new();
    for id in 0..ir.items.len() {
        let item = &ir.items[id];
        let out = modules
            .entry((item.krate.clone(), item.module.clone()))
            .or_default();
        Emitter { ir, id, item }.item(out);
    }
    modules
}

struct Emitter<'a> {
    ir: &'a Ir<'a>,
    id: ItemId,
    item: &'a Item,
}

impl Emitter<'_> {
    fn item(&self, out: &mut String) {
        out.push_str(&doc(self.item.doc.as_deref(), ""));
        match &self.item.kind {
            ItemKind::Struct(s) => self.structure(s, out),
            ItemKind::Container(c) => self.container(&c.variants, c.keyed.as_ref(), out),
            ItemKind::Enum(members) => self.string_enum(members, out),
            ItemKind::Union(members) => self.union(members, out),
            ItemKind::Tagged {
                tag,
                default,
                members,
            } => self.tagged(tag, default.as_deref(), members, out),
            ItemKind::External(members) => self.external(members, out),
            ItemKind::Untagged { members, fallback } => self.untagged(members, *fallback, out),
            ItemKind::Alias(ty) => {
                let _ = writeln!(
                    out,
                    "pub type {}{} = {};\n",
                    self.item.ident,
                    self.declare_generics(&params_in(ty)),
                    self.ty(ty, Position::Field)
                );
            }
            ItemKind::Request(r) => self.request(r, out),
            ItemKind::Newtype(ty) => self.newtype(ty, out),
        }
    }

    // --- types ---

    /// A type as Rust, boxed where the item needs it to break a loop.
    fn ty(&self, ty: &Ty, position: Position) -> String {
        self.ty_in(ty, position, false)
    }

    fn ty_in(&self, ty: &Ty, position: Position, indirect: bool) -> String {
        match ty {
            Ty::Prim(p) => (*p).to_owned(),
            Ty::Unit => "()".to_owned(),
            Ty::Json => VALUE.to_owned(),
            Ty::Param(p) => p.clone(),
            Ty::Named { args, .. } | Ty::Inline { args, .. } => {
                let path = self.path(ty);
                let args = if args.is_empty() {
                    String::new()
                } else {
                    let args: Vec<String> = args
                        .iter()
                        .map(|a| self.ty_in(a, position, indirect))
                        .collect();
                    format!("<{}>", args.join(", "))
                };
                let rendered = format!("{path}{args}");
                if !indirect && self.boxed(ty, position) {
                    format!("::std::boxed::Box<{rendered}>")
                } else {
                    rendered
                }
            }
            Ty::Vec(t) => format!("::std::vec::Vec<{}>", self.ty_in(t, position, true)),
            Ty::Map(k, v) => format!(
                "::indexmap::IndexMap<{}, {}>",
                self.ty_in(k, position, true),
                self.ty_in(v, position, true)
            ),
            Ty::SingleKey(k, v) => format!(
                "{CORE}::shapes::SingleKey<{}, {}>",
                self.ty_in(k, position, indirect),
                self.ty_in(v, position, indirect)
            ),
            Ty::OneOrMany(t) => format!(
                "{CORE}::shapes::OneOrMany<{}>",
                self.ty_in(t, position, indirect)
            ),
            Ty::Stringified(t) => format!(
                "{CORE}::shapes::Stringified<{}>",
                self.ty_in(t, position, indirect)
            ),
            Ty::WithNull(t) => format!(
                "{CORE}::shapes::WithNullValue<{}>",
                self.ty_in(t, position, indirect)
            ),
            Ty::Flags(t) => format!(
                "{CORE}::shapes::PipeSeparatedFlags<{}>",
                self.ty_in(t, position, indirect)
            ),
            Ty::TypedKeys(t) => format!(
                "{CORE}::shapes::TypedKeys<{}>",
                self.ty_in(t, position, true)
            ),
        }
    }

    /// Whether a by-value reference from this item is boxed.
    fn boxed(&self, ty: &Ty, position: Position) -> bool {
        let mut targets = Vec::new();
        self.ir.direct_targets_of(ty, &mut targets);
        targets
            .iter()
            .any(|&t| self.ir.boxed.contains(&(self.id, t, position)))
    }

    /// The path of a named or inline type, without generic arguments.
    fn path(&self, ty: &Ty) -> String {
        let (krate, module, ident) = match ty {
            Ty::Named { target, .. } => {
                let location = self.ir.locations[*target]
                    .as_ref()
                    .expect("named types have a location");
                (&location.krate, &location.module, &location.ident)
            }
            Ty::Inline { item, .. } => {
                let item = &self.ir.items[*item];
                (&item.krate, &item.module, &item.ident)
            }
            _ => unreachable!("only named and inline types have paths"),
        };
        // An item's ident may have been made unique after its location was
        // recorded.
        let ident = match ty {
            Ty::Named { target, .. } => self
                .ir
                .item_of
                .get(target)
                .map_or(ident.as_str(), |&i| self.ir.items[i].ident.as_str()),
            _ => ident.as_str(),
        };
        let root = if *krate == self.item.krate {
            "crate".to_owned()
        } else {
            format!("::{}", super::ir::crate_path(krate))
        };
        let mut path = root;
        for part in module {
            path.push_str("::");
            path.push_str(part);
        }
        path.push_str("::");
        path.push_str(ident);
        path
    }

    fn field_ty(&self, field: &Field) -> String {
        let ty = self.ty(&field.ty, Position::Field);
        if field.optional {
            format!("{OPTION}<{ty}>")
        } else {
            ty
        }
    }

    // --- generics ---

    fn declare_generics(&self, params: &[String]) -> String {
        if params.is_empty() {
            String::new()
        } else {
            let params: Vec<String> = params.iter().map(|p| format!("{p} = {VALUE}")).collect();
            format!("<{}>", params.join(", "))
        }
    }

    fn generics(&self) -> String {
        if self.item.generics.is_empty() {
            String::new()
        } else {
            format!("<{}>", self.item.generics.join(", "))
        }
    }

    /// `<'de, A: Deserialize<'de>, …>` for a `Deserialize` impl.
    fn de_generics(&self) -> String {
        let mut out = vec!["'de".to_owned()];
        out.extend(
            self.item
                .generics
                .iter()
                .map(|g| format!("{g}: ::serde::Deserialize<'de>")),
        );
        format!("<{}>", out.join(", "))
    }

    fn ser_generics(&self) -> String {
        if self.item.generics.is_empty() {
            String::new()
        } else {
            let bounds: Vec<String> = self
                .item
                .generics
                .iter()
                .map(|g| format!("{g}: ::serde::Serialize"))
                .collect();
            format!("<{}>", bounds.join(", "))
        }
    }

    fn self_ty(&self) -> String {
        format!("{}{}", self.item.ident, self.generics())
    }

    /// The generic parameters no field uses, which need a marker.
    fn unused_params<'t>(&self, types: impl Iterator<Item = &'t Ty>) -> Vec<String> {
        let used: BTreeSet<String> = super::ir::params_in_all(types).into_iter().collect();
        self.item
            .generics
            .iter()
            .filter(|g| !used.contains(*g))
            .cloned()
            .collect()
    }

    // --- structs ---

    fn serde_attrs(&self, field: &Field) -> String {
        let mut attrs = Vec::new();
        if bare(&field.ident) != field.wire {
            attrs.push(format!("rename = {}", string(&field.wire)));
        }
        for alias in &field.aliases {
            attrs.push(format!("alias = {}", string(alias)));
        }
        if field.optional {
            attrs.push(format!("skip_serializing_if = \"{OPTION}::is_none\""));
            if matches!(field.ty, Ty::WithNull(_)) {
                attrs.push(format!(
                    "default, deserialize_with = \"{CORE}::__private::some\""
                ));
            }
        }
        if field.defaulted {
            // A path, so serde doesn't ask generic parameters for `Default`.
            attrs.push("default = \"::core::default::Default::default\"".to_owned());
        }
        if attrs.is_empty() {
            String::new()
        } else {
            format!("    #[serde({})]\n", attrs.join(", "))
        }
    }

    fn fields(&self, fields: &[Field], serde: bool, public: bool, out: &mut String) {
        for field in fields {
            out.push_str(&doc(field.doc.as_deref(), "    "));
            if serde {
                out.push_str(&self.serde_attrs(field));
            }
            let _ = writeln!(
                out,
                "    {}{}: {},",
                if public { "pub " } else { "" },
                field.ident,
                self.field_ty(field)
            );
        }
    }

    fn marker_decl(&self, unused: &[String], serde: bool) -> String {
        if unused.is_empty() {
            return String::new();
        }
        format!(
            "    {}_marker: ::core::marker::PhantomData<fn() -> ({},)>,\n",
            if serde { "#[serde(skip)]\n    " } else { "" },
            unused.join(", ")
        )
    }

    fn structure(&self, s: &Struct, out: &mut String) {
        let ident = &self.item.ident;
        let mut types: Vec<&Ty> = s.fields.iter().map(|f| &f.ty).collect();
        if let Some((_, k, v)) = &s.extra {
            types.push(k);
            types.push(v);
        }
        if let Some((_, _, k, v)) = &s.keyed {
            types.push(k);
            types.push(v);
        }
        let unused = self.unused_params(types.into_iter());
        let derive_serde = s.serde && s.shorthand.is_none() && s.variant.is_none();
        let _ = writeln!(
            out,
            "#[derive(Clone, Debug, PartialEq{})]",
            if derive_serde {
                ", ::serde::Serialize, ::serde::Deserialize"
            } else {
                ""
            }
        );
        if s.keyed.is_some() {
            let repr = format!("{ident}Repr{}", self.generics());
            let _ = writeln!(
                out,
                "#[serde(try_from = {}, into = {})]",
                string(&repr),
                string(&repr)
            );
            if !self.item.generics.is_empty() {
                let bounds: Vec<String> = self
                    .item
                    .generics
                    .iter()
                    .map(|g| format!("{g}: ::serde::Serialize + ::core::clone::Clone"))
                    .collect();
                let _ = writeln!(
                    out,
                    "#[serde(bound(serialize = {}))]",
                    string(&bounds.join(", "))
                );
            }
        }
        out.push_str("#[non_exhaustive]\n");
        let _ = writeln!(
            out,
            "pub struct {ident}{} {{",
            self.declare_generics(&self.item.generics)
        );
        self.fields(&s.fields, derive_serde, true, out);
        if let Some((name, k, v)) = &s.extra {
            out.push_str("    /// Properties the specification doesn't name.\n");
            if derive_serde {
                out.push_str("    #[serde(flatten)]\n");
            }
            let _ = writeln!(
                out,
                "    pub {name}: ::indexmap::IndexMap<{}, {}>,",
                self.ty_in(k, Position::Field, true),
                self.ty_in(v, Position::Field, true)
            );
        }
        if let Some((key, value, k, v)) = &s.keyed {
            let _ = writeln!(
                out,
                "    /// The name of the property that holds `{}`.",
                bare(value)
            );
            let _ = writeln!(out, "    pub {key}: {},", self.ty(k, Position::Field));
            let _ = writeln!(out, "    /// The value under `{}`.", bare(key));
            let _ = writeln!(out, "    pub {value}: {},", self.ty(v, Position::Field));
        }
        if let Some((field, variant)) = &s.variant {
            let variant = self.ty(
                &Ty::Inline {
                    item: *variant,
                    args: Vec::new(),
                },
                Position::Field,
            );
            let _ = writeln!(
                out,
                "    /// The variant.\n    pub {field}: {OPTION}<{variant}>,"
            );
        }
        out.push_str(&self.marker_decl(&unused, derive_serde || s.keyed.is_some()));
        out.push_str("}\n\n");

        self.constructor(s, &unused, out);
        if let Some((index, write_short)) = s.shorthand {
            self.shorthand(s, index, write_short, &unused, out);
        }
        if s.keyed.is_some() {
            self.keyed_repr(s, &unused, out);
        }
        if s.variant.is_some() {
            self.container_struct_serde(s, out);
        }
    }

    fn constructor(&self, s: &Struct, unused: &[String], out: &mut String) {
        let ident = &self.item.ident;
        let mut params = Vec::new();
        let mut inits = Vec::new();
        for field in &s.fields {
            let ty = self.ty(&field.ty, Position::Field);
            let (inner, boxed) = unbox(&ty);
            if field.required {
                params.push(format!(
                    "{}: impl ::core::convert::Into<{inner}>",
                    field.ident
                ));
                let value = if boxed {
                    format!("::std::boxed::Box::new({}.into())", field.ident)
                } else {
                    format!("{}.into()", field.ident)
                };
                inits.push(if field.optional {
                    format!("{}: {OPTION}::Some({value})", field.ident)
                } else {
                    format!("{}: {value}", field.ident)
                });
            } else {
                inits.push(format!("{}: {OPTION}::None", field.ident));
            }
        }
        if let Some((name, _, _)) = &s.extra {
            inits.push(format!("{name}: ::indexmap::IndexMap::new()"));
        }
        if let Some((key, value, k, v)) = &s.keyed {
            params.push(format!(
                "{key}: impl ::core::convert::Into<{}>",
                self.ty(k, Position::Field)
            ));
            params.push(format!(
                "{value}: impl ::core::convert::Into<{}>",
                self.ty(v, Position::Field)
            ));
            inits.push(format!("{key}: {key}.into()"));
            inits.push(format!("{value}: {value}.into()"));
        }
        if let Some((field, _)) = &s.variant {
            inits.push(format!("{field}: {OPTION}::None"));
        }
        if !unused.is_empty() {
            inits.push("_marker: ::core::marker::PhantomData".to_owned());
        }
        let _ = writeln!(out, "impl{} {} {{", self.generics(), self.self_ty());
        let _ = writeln!(out, "    /// A `{ident}` with its required properties set.");
        let _ = writeln!(out, "    pub fn new({}) -> Self {{", params.join(", "));
        let _ = writeln!(out, "        Self {{ {} }}", inits.join(", "));
        out.push_str("    }\n");
        let mut setters: Vec<(&str, &Ty)> = s
            .fields
            .iter()
            .filter(|f| !f.required)
            .map(|f| (f.ident.as_str(), &f.ty))
            .collect();
        let variant_ty;
        if let Some((field, variant)) = &s.variant {
            variant_ty = Ty::Inline {
                item: *variant,
                args: Vec::new(),
            };
            setters.push((field.as_str(), &variant_ty));
        }
        for (field, ty) in setters {
            let rendered = self.ty(ty, Position::Field);
            let (inner, boxed) = unbox(&rendered);
            let name = if field == "new" { "new_" } else { field };
            let _ = writeln!(out, "    /// Sets `{}`.", bare(field));
            let value = if boxed {
                "::std::boxed::Box::new(value.into())"
            } else {
                "value.into()"
            };
            let _ = writeln!(
                out,
                "    pub fn {name}(mut self, value: impl ::core::convert::Into<{inner}>) -> Self {{ self.{field} = {OPTION}::Some({value}); self }}"
            );
        }
        out.push_str("}\n\n");
    }

    fn shorthand(
        &self,
        s: &Struct,
        index: usize,
        write_short: bool,
        unused: &[String],
        out: &mut String,
    ) {
        let ident = &self.item.ident;
        let def = format!("{ident}Def");
        let short = &s.fields[index];
        // The full form, read and written by serde through a remote
        // definition with the same fields.
        let _ = writeln!(
            out,
            "#[derive(::serde::Serialize, ::serde::Deserialize)]\n#[serde(remote = {})]",
            string(ident)
        );
        let _ = writeln!(out, "struct {def} {{");
        self.fields(&s.fields, true, false, out);
        out.push_str(&self.marker_decl(unused, true));
        out.push_str("}\n\n");
        let short_ty = self.ty(&short.ty, Position::Field);
        let _ = writeln!(out, "impl ::serde::Serialize for {ident} {{");
        let _ = writeln!(
            out,
            "    fn serialize<S__: ::serde::Serializer>(&self, serializer: S__) -> {RESULT}<S__::Ok, S__::Error> {{"
        );
        if write_short {
            let others: Vec<String> = s
                .fields
                .iter()
                .enumerate()
                .filter(|(n, _)| *n != index)
                .map(|(_, f)| format!("self.{}.is_none()", f.ident))
                .collect();
            let condition = if others.is_empty() {
                "true".to_owned()
            } else {
                others.join(" && ")
            };
            if !short.optional {
                let _ = writeln!(
                    out,
                    "        if {condition} {{ return ::serde::Serialize::serialize(&self.{}, serializer); }}",
                    short.ident
                );
            } else {
                let _ = writeln!(
                    out,
                    "        if let ({OPTION}::Some(short), true) = (&self.{}, {condition}) {{ return ::serde::Serialize::serialize(short, serializer); }}",
                    short.ident
                );
            }
        }
        let _ = writeln!(
            out,
            "        {def}::serialize(self, serializer)\n    }}\n}}\n"
        );
        let mut inits: Vec<String> = s
            .fields
            .iter()
            .enumerate()
            .map(|(n, f)| {
                if n == index {
                    if f.optional {
                        format!("{}: {OPTION}::Some(short)", f.ident)
                    } else {
                        format!("{}: short", f.ident)
                    }
                } else {
                    format!("{}: {OPTION}::None", f.ident)
                }
            })
            .collect();
        if !unused.is_empty() {
            inits.push("_marker: ::core::marker::PhantomData".to_owned());
        }
        let _ = writeln!(
            out,
            "impl<'de> {CORE}::__private::Shorthand<'de> for {ident} {{"
        );
        let _ = writeln!(out, "    type Short = {short_ty};");
        let _ = writeln!(
            out,
            "    fn from_short(short: {short_ty}) -> Self {{ Self {{ {} }} }}",
            inits.join(", ")
        );
        let _ = writeln!(
            out,
            "    fn deserialize_full<D__: ::serde::Deserializer<'de>>(deserializer: D__) -> {RESULT}<Self, D__::Error> {{ {def}::deserialize(deserializer) }}\n}}\n"
        );
        let _ = writeln!(out, "impl<'de> ::serde::Deserialize<'de> for {ident} {{");
        let _ = writeln!(
            out,
            "    fn deserialize<D__: ::serde::Deserializer<'de>>(deserializer: D__) -> {RESULT}<Self, D__::Error> {{ {CORE}::__private::deserialize_shorthand(deserializer) }}\n}}\n"
        );
    }

    fn keyed_repr(&self, s: &Struct, unused: &[String], out: &mut String) {
        let ident = &self.item.ident;
        let repr = format!("{ident}Repr");
        let (key, value, k, v) = s.keyed.as_ref().expect("keyed structs have a key");
        let generics = self.generics();
        let _ = writeln!(out, "#[derive(::serde::Serialize, ::serde::Deserialize)]");
        let _ = writeln!(
            out,
            "struct {repr}{} {{",
            self.declare_generics(&self.item.generics)
        );
        self.fields(&s.fields, true, false, out);
        if let Some((name, ek, ev)) = &s.extra {
            let _ = writeln!(
                out,
                "    #[serde(flatten)]\n    {name}: ::indexmap::IndexMap<{}, {}>,",
                self.ty_in(ek, Position::Field, true),
                self.ty_in(ev, Position::Field, true)
            );
        }
        let _ = writeln!(
            out,
            "    #[serde(flatten)]\n    keyed__: ::indexmap::IndexMap<{}, {}>,",
            self.ty_in(k, Position::Field, true),
            self.ty(v, Position::Field)
        );
        out.push_str(&self.marker_decl(unused, true));
        out.push_str("}\n\n");
        let mut moves: Vec<String> = s
            .fields
            .iter()
            .map(|f| format!("{}: repr.{}", f.ident, f.ident))
            .collect();
        if let Some((name, _, _)) = &s.extra {
            moves.push(format!("{name}: repr.{name}"));
        }
        if !unused.is_empty() {
            moves.push("_marker: ::core::marker::PhantomData".to_owned());
        }
        let _ = writeln!(
            out,
            "impl{generics} ::core::convert::TryFrom<{repr}{generics}> for {ident}{generics} {{"
        );
        let _ = writeln!(out, "    type Error = ::std::string::String;");
        let _ = writeln!(
            out,
            "    fn try_from(repr: {repr}{generics}) -> {RESULT}<Self, Self::Error> {{"
        );
        let _ = writeln!(out, "        let mut keyed = repr.keyed__.into_iter();");
        let _ = writeln!(out, "        match (keyed.next(), keyed.next()) {{");
        let _ = writeln!(
            out,
            "            ({OPTION}::Some(({key}, {value})), {OPTION}::None) => {RESULT}::Ok(Self {{ {}{key}, {value} }}),",
            moves.iter().map(|m| format!("{m}, ")).collect::<String>()
        );
        let _ = writeln!(
            out,
            "            _ => {RESULT}::Err(::std::string::String::from({})),",
            string(&format!(
                "{ident} needs exactly one property keyed by its {}",
                bare(key)
            ))
        );
        out.push_str("        }\n    }\n}\n\n");
        let mut back: Vec<String> = s
            .fields
            .iter()
            .map(|f| format!("{}: value.{}", f.ident, f.ident))
            .collect();
        if let Some((name, _, _)) = &s.extra {
            back.push(format!("{name}: value.{name}"));
        }
        if !unused.is_empty() {
            back.push("_marker: ::core::marker::PhantomData".to_owned());
        }
        let _ = writeln!(
            out,
            "impl{generics} ::core::convert::From<{ident}{generics}> for {repr}{generics} {{"
        );
        let _ = writeln!(out, "    fn from(value: {ident}{generics}) -> Self {{");
        let _ = writeln!(
            out,
            "        let mut keyed__ = ::indexmap::IndexMap::new();"
        );
        let _ = writeln!(out, "        keyed__.insert(value.{key}, value.{value});");
        let _ = writeln!(
            out,
            "        Self {{ {}keyed__ }}",
            back.iter().map(|m| format!("{m}, ")).collect::<String>()
        );
        out.push_str("    }\n}\n\n");
    }

    /// Serde for a container with properties beside its variant.
    fn container_struct_serde(&self, s: &Struct, out: &mut String) {
        let ident = &self.item.ident;
        let (variant_field, variant) = s
            .variant
            .as_ref()
            .expect("a container struct has a variant");
        let variant_ident = &self.ir.items[*variant].ident;
        let _ = writeln!(out, "impl ::serde::Serialize for {ident} {{");
        let _ = writeln!(
            out,
            "    fn serialize<S__: ::serde::Serializer>(&self, serializer: S__) -> {RESULT}<S__::Ok, S__::Error> {{"
        );
        out.push_str("        use ::serde::ser::SerializeMap as _;\n");
        out.push_str(
            "        let mut map = serializer.serialize_map(::core::option::Option::None)?;\n",
        );
        for field in &s.fields {
            if !field.optional {
                let _ = writeln!(
                    out,
                    "        map.serialize_entry({}, &self.{})?;",
                    string(&field.wire),
                    field.ident
                );
            } else {
                let _ = writeln!(
                    out,
                    "        if let {OPTION}::Some(value) = &self.{} {{ map.serialize_entry({}, value)?; }}",
                    field.ident,
                    string(&field.wire)
                );
            }
        }
        let _ = writeln!(
            out,
            "        if let {OPTION}::Some(variant) = &self.{variant_field} {{ variant.serialize_entry(&mut map)?; }}"
        );
        out.push_str("        map.end()\n    }\n}\n\n");

        let _ = writeln!(out, "impl<'de> ::serde::Deserialize<'de> for {ident} {{");
        let _ = writeln!(
            out,
            "    fn deserialize<D__: ::serde::Deserializer<'de>>(deserializer: D__) -> {RESULT}<Self, D__::Error> {{"
        );
        out.push_str("        struct Fields;\n");
        out.push_str("        impl<'de> ::serde::de::Visitor<'de> for Fields {\n");
        let _ = writeln!(out, "            type Value = {ident};");
        let _ = writeln!(
            out,
            "            fn expecting(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {{ f.write_str({}) }}",
            string(&format!("a {ident} object"))
        );
        let _ = writeln!(
            out,
            "            fn visit_map<A__: ::serde::de::MapAccess<'de>>(self, mut map: A__) -> {RESULT}<{ident}, A__::Error> {{"
        );
        for field in &s.fields {
            let _ = writeln!(
                out,
                "                let mut {} = {OPTION}::None;",
                field.ident
            );
        }
        out.push_str("                let mut variant__ = ::core::option::Option::None;\n");
        out.push_str("                while let ::core::option::Option::Some(key) = map.next_key::<::std::string::String>()? {\n");
        out.push_str("                    match key.as_str() {\n");
        for field in &s.fields {
            let names: Vec<String> = std::iter::once(&field.wire)
                .chain(&field.aliases)
                .map(|n| string(n))
                .collect();
            let _ = writeln!(
                out,
                "                        {} => {} = {OPTION}::Some(map.next_value()?),",
                names.join(" | "),
                field.ident
            );
        }
        out.push_str("                        _ => {\n");
        let _ = writeln!(
            out,
            "                            if variant__.is_some() {{ return {RESULT}::Err({CORE}::__private::two_variants({}, &key)); }}",
            string(ident)
        );
        let boxed = self.boxed(
            &Ty::Inline {
                item: *variant,
                args: Vec::new(),
            },
            Position::Field,
        );
        let read = format!(
            "<{variant_ident} as {CORE}::__private::Container<'de>>::variant(&key, &mut map)?"
        );
        let read = if boxed {
            format!("::std::boxed::Box::new({read})")
        } else {
            read
        };
        let _ = writeln!(
            out,
            "                            variant__ = {OPTION}::Some({read});"
        );
        out.push_str("                        }\n                    }\n                }\n");
        let mut inits = Vec::new();
        for field in &s.fields {
            if field.defaulted {
                inits.push(format!(
                    "{}: {}.unwrap_or_default()",
                    field.ident, field.ident
                ));
            } else if !field.optional {
                inits.push(format!(
                    "{}: {}.ok_or_else(|| <A__::Error as ::serde::de::Error>::missing_field({}))?",
                    field.ident,
                    field.ident,
                    string(&field.wire)
                ));
            } else {
                inits.push(field.ident.clone());
            }
        }
        inits.push(format!("{variant_field}: variant__"));
        let _ = writeln!(
            out,
            "                {RESULT}::Ok({ident} {{ {} }})",
            inits.join(", ")
        );
        out.push_str(
            "            }\n        }\n        deserializer.deserialize_map(Fields)\n    }\n}\n\n",
        );
    }

    // --- containers ---

    fn container(&self, variants: &[Variant], keyed: Option<&(String, Ty, Ty)>, out: &mut String) {
        let ident = &self.item.ident;
        out.push_str("#[derive(Clone, Debug, PartialEq)]\n#[non_exhaustive]\n");
        let _ = writeln!(out, "pub enum {ident} {{");
        for v in variants {
            out.push_str(&doc(v.doc.as_deref(), "    "));
            let _ = writeln!(
                out,
                "    {}({}),",
                v.ident,
                self.ty(&v.ty, Position::Variant)
            );
        }
        if let Some((name, k, v)) = keyed {
            let _ = writeln!(
                out,
                "    /// A property named by data, such as a field name.\n    {name}({CORE}::shapes::SingleKey<{}, {}>),",
                self.ty(k, Position::Variant),
                self.ty(v, Position::Variant)
            );
        }
        out.push_str("    /// Any variant, as JSON: an object with one property.\n");
        let _ = writeln!(out, "    Raw({VALUE}),\n}}\n");

        // Writing one variant, also into a map other properties are in.
        let _ = writeln!(out, "impl {ident} {{");
        out.push_str("    /// Writes the variant as one entry of a map.\n");
        let _ = writeln!(
            out,
            "    pub fn serialize_entry<M__: ::serde::ser::SerializeMap>(&self, map: &mut M__) -> {RESULT}<(), M__::Error> {{"
        );
        out.push_str("        match self {\n");
        for v in variants {
            let _ = writeln!(
                out,
                "            Self::{}(value) => map.serialize_entry({}, value),",
                v.ident,
                string(&v.wire)
            );
        }
        if let Some((name, _, _)) = keyed {
            let _ = writeln!(
                out,
                "            Self::{name}(pair) => map.serialize_entry(&pair.key, &pair.value),"
            );
        }
        out.push_str("            Self::Raw(value) => match value {\n");
        let _ = writeln!(
            out,
            "                {VALUE}::Object(entries) => {{ for (key, value) in entries {{ map.serialize_entry(key, value)?; }} {RESULT}::Ok(()) }}"
        );
        let _ = writeln!(
            out,
            "                _ => {RESULT}::Err(<M__::Error as ::serde::ser::Error>::custom(\"a raw variant must be an object\")),"
        );
        out.push_str("            },\n        }\n    }\n}\n\n");

        let _ = writeln!(out, "impl ::serde::Serialize for {ident} {{");
        let _ = writeln!(
            out,
            "    fn serialize<S__: ::serde::Serializer>(&self, serializer: S__) -> {RESULT}<S__::Ok, S__::Error> {{"
        );
        out.push_str("        match self {\n");
        for v in variants {
            let _ = writeln!(
                out,
                "            Self::{}(value) => {CORE}::__private::serialize_variant(serializer, {}, value),",
                v.ident,
                string(&v.wire)
            );
        }
        if let Some((name, _, _)) = keyed {
            let _ = writeln!(
                out,
                "            Self::{name}(pair) => ::serde::Serialize::serialize(pair, serializer),"
            );
        }
        out.push_str(
            "            Self::Raw(value) => ::serde::Serialize::serialize(value, serializer),\n",
        );
        out.push_str("        }\n    }\n}\n\n");

        let _ = writeln!(out, "impl<'de> ::serde::Deserialize<'de> for {ident} {{");
        let _ = writeln!(
            out,
            "    fn deserialize<D__: ::serde::Deserializer<'de>>(deserializer: D__) -> {RESULT}<Self, D__::Error> {{ {CORE}::__private::deserialize_container(deserializer) }}\n}}\n"
        );
        let _ = writeln!(
            out,
            "impl<'de> {CORE}::__private::Container<'de> for {ident} {{"
        );
        let _ = writeln!(out, "    const NAME: &'static str = {};", string(ident));
        let _ = writeln!(
            out,
            "    fn variant<A__: ::serde::de::MapAccess<'de>>(key: &str, map: &mut A__) -> {RESULT}<Self, A__::Error> {{"
        );
        let _ = writeln!(out, "        {RESULT}::Ok(match key {{");
        for v in variants {
            let names: Vec<String> = std::iter::once(&v.wire)
                .chain(&v.aliases)
                .map(|n| string(n))
                .collect();
            let _ = writeln!(
                out,
                "            {} => Self::{}(map.next_value()?),",
                names.join(" | "),
                v.ident
            );
        }
        if let Some((name, _, _)) = keyed {
            let _ = writeln!(
                out,
                "            _ => Self::{name}({CORE}::shapes::SingleKey {{ key: ::core::convert::From::from(key), value: map.next_value()? }}),"
            );
        } else {
            let _ = writeln!(
                out,
                "            _ => Self::Raw({CORE}::__private::raw_variant(key, map.next_value()?)),"
            );
        }
        out.push_str("        })\n    }\n}\n\n");
    }

    // --- enums of strings ---

    fn string_enum(&self, members: &[super::ir::EnumMember], out: &mut String) {
        let ident = &self.item.ident;
        out.push_str("#[derive(Clone, Debug, PartialEq, Eq, Hash)]\n#[non_exhaustive]\n");
        let _ = writeln!(out, "pub enum {ident} {{");
        for m in members {
            let mut text = m.doc.clone().unwrap_or_default();
            if text.is_empty() {
                text = format!("`{}`", m.wire);
            }
            out.push_str(&doc(Some(&text), "    "));
            let _ = writeln!(out, "    {},", m.ident);
        }
        out.push_str("    /// A value this version of the specification doesn't list.\n");
        out.push_str("    Raw(::std::string::String),\n}\n\n");
        let _ = writeln!(out, "impl {ident} {{");
        out.push_str("    /// The value as Elasticsearch writes it.\n    pub fn as_str(&self) -> &str {\n        match self {\n");
        for m in members {
            let _ = writeln!(out, "            Self::{} => {},", m.ident, string(&m.wire));
        }
        out.push_str("            Self::Raw(value) => value,\n        }\n    }\n\n");
        out.push_str("    /// The listed value with this name, if there is one.\n");
        let _ = writeln!(
            out,
            "    pub fn from_known(value: &str) -> {OPTION}<Self> {{\n        match value {{"
        );
        for m in members {
            let names: Vec<String> = std::iter::once(&m.wire)
                .chain(&m.aliases)
                .map(|n| string(n))
                .collect();
            let _ = writeln!(
                out,
                "            {} => {OPTION}::Some(Self::{}),",
                names.join(" | "),
                m.ident
            );
        }
        let _ = writeln!(
            out,
            "            _ => {OPTION}::None,\n        }}\n    }}\n}}\n"
        );
        let _ = writeln!(out, "impl ::serde::Serialize for {ident} {{");
        let _ = writeln!(
            out,
            "    fn serialize<S__: ::serde::Serializer>(&self, serializer: S__) -> {RESULT}<S__::Ok, S__::Error> {{ serializer.serialize_str(self.as_str()) }}\n}}\n"
        );
        let _ = writeln!(out, "impl<'de> ::serde::Deserialize<'de> for {ident} {{");
        let _ = writeln!(
            out,
            "    fn deserialize<D__: ::serde::Deserializer<'de>>(deserializer: D__) -> {RESULT}<Self, D__::Error> {{\n        {CORE}::__private::deserialize_string_enum(deserializer, {}, |value| Self::from_known(value).unwrap_or_else(|| Self::Raw(value.to_owned())))\n    }}\n}}\n",
            string(ident)
        );
    }

    // --- unions ---

    fn union(&self, members: &[UnionMember], out: &mut String) {
        let ident = &self.item.ident;
        out.push_str("#[derive(Clone, Debug, PartialEq)]\n#[non_exhaustive]\n");
        let _ = writeln!(
            out,
            "pub enum {ident}{} {{",
            self.declare_generics(&self.item.generics)
        );
        for m in members {
            match &m.ty {
                Some(ty) => {
                    let _ = writeln!(out, "    /// A {}.", describe_kinds(m.kinds));
                    let _ = writeln!(out, "    {}({}),", m.ident, self.ty(ty, Position::Variant));
                }
                None => {
                    let _ = writeln!(
                        out,
                        "    /// `{}`.",
                        literal_text(m.literal.as_ref().expect("a literal member"))
                    );
                    let _ = writeln!(out, "    {},", m.ident);
                }
            }
        }
        out.push_str("}\n\n");

        let generics = self.generics();
        let _ = writeln!(
            out,
            "impl{} ::serde::Serialize for {ident}{generics} {{",
            self.ser_generics()
        );
        let _ = writeln!(
            out,
            "    fn serialize<S__: ::serde::Serializer>(&self, serializer: S__) -> {RESULT}<S__::Ok, S__::Error> {{"
        );
        out.push_str("        match self {\n");
        for m in members {
            match &m.literal {
                None => {
                    let _ = writeln!(
                        out,
                        "            Self::{}(value) => ::serde::Serialize::serialize(value, serializer),",
                        m.ident
                    );
                }
                Some(literal) => {
                    let write = match literal {
                        Literal::String(s) => format!("serializer.serialize_str({})", string(s)),
                        Literal::Boolean(b) => format!("serializer.serialize_bool({b})"),
                        Literal::Number(n) => match n.as_i64() {
                            Some(i) => format!("serializer.serialize_i64({i})"),
                            None => format!(
                                "serializer.serialize_f64({}f64)",
                                n.as_f64().unwrap_or_default()
                            ),
                        },
                    };
                    let _ = writeln!(out, "            Self::{} => {write},", m.ident);
                }
            }
        }
        out.push_str("        }\n    }\n}\n\n");

        // Reading: dispatch on the JSON kind, and look closer only when
        // several members take it.
        let marker = if self.item.generics.is_empty() {
            String::new()
        } else {
            format!(
                "(::core::marker::PhantomData<fn() -> ({},)>)",
                self.item.generics.join(", ")
            )
        };
        let construct = if self.item.generics.is_empty() {
            "Shape"
        } else {
            "Shape(::core::marker::PhantomData)"
        };
        let _ = writeln!(
            out,
            "impl{} ::serde::Deserialize<'de> for {ident}{generics} {{",
            self.de_generics()
        );
        let _ = writeln!(
            out,
            "    fn deserialize<D__: ::serde::Deserializer<'de>>(deserializer: D__) -> {RESULT}<Self, D__::Error> {{"
        );
        let _ = writeln!(out, "        struct Shape{generics}{marker};");
        let _ = writeln!(
            out,
            "        impl{} {CORE}::__private::ByKind<'de> for Shape{generics} {{",
            self.de_generics()
        );
        let _ = writeln!(out, "            type Value = {ident}{generics};");
        let expecting: Vec<String> = members
            .iter()
            .map(|m| describe_kinds(m.kinds))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let _ = writeln!(
            out,
            "            const EXPECTING: &'static str = {};",
            string(&format!("{ident}: {}", expecting.join(" or ")))
        );
        let _ = writeln!(
            out,
            "            fn visit<V__: ::serde::Deserializer<'de>>(self, kind: {CORE}::__private::Kind, value: V__) -> {RESULT}<Self::Value, V__::Error> {{"
        );
        out.push_str("                match kind {\n");
        let mut covered = 0;
        for (kind, bit) in [
            ("Null", NULL),
            ("Bool", BOOLEAN),
            ("Number", NUMBER),
            ("String", STRING),
            ("Array", ARRAY),
            ("Object", OBJECT),
        ] {
            let candidates: Vec<&UnionMember> =
                members.iter().filter(|m| m.kinds & bit != 0).collect();
            if candidates.is_empty() {
                continue;
            }
            covered += 1;
            let _ = writeln!(
                out,
                "                    {CORE}::__private::Kind::{kind} => {{"
            );
            self.union_kind(&candidates, bit, out);
            out.push_str("                    }\n");
        }
        if covered < 6 {
            let _ = writeln!(
                out,
                "                    _ => {RESULT}::Err(<V__::Error as ::serde::de::Error>::custom(format_args!(\"{ident} can't be {{kind:?}}\"))),"
            );
        }
        out.push_str("                }\n            }\n        }\n");
        let _ = writeln!(
            out,
            "        {CORE}::__private::deserialize_by_kind(deserializer, {construct})\n    }}\n}}\n"
        );
    }

    fn union_kind(&self, candidates: &[&UnionMember], bit: u8, out: &mut String) {
        let indent = "                        ";
        let out_ty = &self.item.ident;
        if let [only] = candidates
            && only.literal.is_none()
        {
            let ty = self.ty(only.ty.as_ref().expect("not a literal"), Position::Variant);
            let _ = writeln!(
                out,
                "{indent}<{ty} as ::serde::Deserialize>::deserialize(value).map({out_ty}::{})",
                only.ident
            );
            return;
        }
        let _ = writeln!(
            out,
            "{indent}let json: {VALUE} = ::serde::Deserialize::deserialize(value)?;"
        );
        // Literals first, then enums by their known values, then objects by
        // a property only they have, then the rest in order.
        for m in candidates.iter().filter(|m| m.literal.is_some()) {
            let check = match m.literal.as_ref().expect("filtered to literals") {
                Literal::String(s) => format!("json.as_str() == {OPTION}::Some({})", string(s)),
                Literal::Boolean(b) => format!("json.as_bool() == {OPTION}::Some({b})"),
                Literal::Number(n) => format!(
                    "json.as_f64() == {OPTION}::Some({}f64)",
                    n.as_f64().unwrap_or_default()
                ),
            };
            let _ = writeln!(
                out,
                "{indent}if {check} {{ return {RESULT}::Ok({out_ty}::{}); }}",
                m.ident
            );
        }
        let rest: Vec<&&UnionMember> = candidates.iter().filter(|m| m.literal.is_none()).collect();
        let mut tried = BTreeSet::new();
        if bit == STRING {
            for m in rest.iter().filter(|m| m.string_enum) {
                let ty = m.ty.as_ref().expect("not a literal");
                let path = self.path(ty);
                let _ = writeln!(
                    out,
                    "{indent}if let {OPTION}::Some(found) = json.as_str().and_then({path}::from_known) {{ return {RESULT}::Ok({out_ty}::{}(::core::convert::From::from(found))); }}",
                    m.ident
                );
                tried.insert(m.ident.clone());
            }
        }
        if bit == OBJECT {
            let keyed: Vec<&&&UnionMember> = rest.iter().filter(|m| m.keys.is_some()).collect();
            for m in &keyed {
                let others: BTreeSet<&String> = keyed
                    .iter()
                    .filter(|o| o.ident != m.ident)
                    .flat_map(|o| o.keys.as_ref().expect("filtered to keyed").iter())
                    .collect();
                let unique: Vec<&String> = m
                    .keys
                    .as_ref()
                    .expect("filtered to keyed")
                    .iter()
                    .filter(|k| !others.contains(k))
                    .collect();
                if unique.is_empty() {
                    continue;
                }
                let checks: Vec<String> = unique
                    .iter()
                    .map(|k| format!("{CORE}::__private::has(&json, {})", string(k)))
                    .collect();
                let _ = writeln!(
                    out,
                    "{indent}if {} {{ return {CORE}::__private::from_value(json).map({out_ty}::{}); }}",
                    checks.join(" || "),
                    m.ident
                );
            }
        }
        // A member without its distinguishing key may still be the one, so
        // every member other than an enum is tried in order.
        let remaining: Vec<&&UnionMember> = rest
            .iter()
            .copied()
            .filter(|m| !tried.contains(&m.ident))
            .collect();
        match remaining.split_last() {
            None => {
                let _ = writeln!(
                    out,
                    "{indent}{RESULT}::Err(<V__::Error as ::serde::de::Error>::custom(\"no member of {} matches\"))",
                    self.item.ident
                );
            }
            Some((last, first)) => {
                for m in first {
                    let ty = self.ty(m.ty.as_ref().expect("not a literal"), Position::Variant);
                    let _ = writeln!(
                        out,
                        "{indent}if let {RESULT}::Ok(found) = {CORE}::__private::from_value::<{ty}, V__::Error>(json.clone()) {{ return {RESULT}::Ok({out_ty}::{}(found)); }}",
                        m.ident
                    );
                }
                let _ = writeln!(
                    out,
                    "{indent}{CORE}::__private::from_value(json).map({out_ty}::{})",
                    last.ident
                );
            }
        }
    }

    // --- tagged unions ---

    fn variants_decl(&self, members: &[Variant], raw: &str, out: &mut String) {
        out.push_str("#[derive(Clone, Debug, PartialEq)]\n#[non_exhaustive]\n");
        let _ = writeln!(
            out,
            "pub enum {}{} {{",
            self.item.ident,
            self.declare_generics(&self.item.generics)
        );
        for v in members {
            let _ = writeln!(out, "    /// `{}`.", v.wire);
            let _ = writeln!(
                out,
                "    {}({}),",
                v.ident,
                self.ty(&v.ty, Position::Variant)
            );
        }
        out.push_str(raw);
        out.push_str("}\n\n");
    }

    fn tagged(&self, tag: &str, default: Option<&str>, members: &[Variant], out: &mut String) {
        let ident = &self.item.ident;
        self.variants_decl(
            members,
            &format!("    /// Any member, as JSON.\n    Raw({VALUE}),\n"),
            out,
        );
        let generics = self.generics();
        let _ = writeln!(
            out,
            "impl{} ::serde::Serialize for {ident}{generics} {{",
            self.ser_generics()
        );
        let _ = writeln!(
            out,
            "    fn serialize<S__: ::serde::Serializer>(&self, serializer: S__) -> {RESULT}<S__::Ok, S__::Error> {{"
        );
        out.push_str("        match self {\n");
        for v in members {
            let _ = writeln!(
                out,
                "            Self::{}(value) => {CORE}::__private::serialize_tagged(serializer, {}, {}, value),",
                v.ident,
                string(tag),
                string(&v.wire)
            );
        }
        out.push_str("            Self::Raw(value) => ::serde::Serialize::serialize(value, serializer),\n        }\n    }\n}\n\n");
        let _ = writeln!(
            out,
            "impl{} ::serde::Deserialize<'de> for {ident}{generics} {{",
            self.de_generics()
        );
        let _ = writeln!(
            out,
            "    fn deserialize<D__: ::serde::Deserializer<'de>>(deserializer: D__) -> {RESULT}<Self, D__::Error> {{"
        );
        let default = default.map_or(format!("{OPTION}::None"), |d| {
            format!("{OPTION}::Some({})", string(d))
        });
        let _ = writeln!(
            out,
            "        let (tag, map) = {CORE}::__private::deserialize_tagged(deserializer, {}, {default})?;",
            string(tag)
        );
        let _ = writeln!(out, "        let json = {VALUE}::Object(map);");
        out.push_str("        match tag.as_deref() {\n");
        for v in members {
            let _ = writeln!(
                out,
                "            {OPTION}::Some({}) => {CORE}::__private::from_value(json).map(Self::{}),",
                string(&v.wire),
                v.ident
            );
        }
        let _ = writeln!(
            out,
            "            _ => {RESULT}::Ok(Self::Raw(json)),\n        }}\n    }}\n}}\n"
        );
    }

    fn external(&self, members: &[Variant], out: &mut String) {
        let ident = &self.item.ident;
        self.variants_decl(
            members,
            &format!("    /// Any member, as JSON, with its tag if it had one.\n    Raw {{ tag: {OPTION}<::std::string::String>, value: {VALUE} }},\n"),
            out,
        );
        let generics = self.generics();
        let _ = writeln!(
            out,
            "impl{} ::serde::Serialize for {ident}{generics} {{",
            self.ser_generics()
        );
        let _ = writeln!(
            out,
            "    fn serialize<S__: ::serde::Serializer>(&self, serializer: S__) -> {RESULT}<S__::Ok, S__::Error> {{"
        );
        out.push_str("        match self {\n");
        for v in members {
            let _ = writeln!(
                out,
                "            Self::{}(value) => ::serde::Serialize::serialize(value, serializer),",
                v.ident
            );
        }
        out.push_str("            Self::Raw { value, .. } => ::serde::Serialize::serialize(value, serializer),\n        }\n    }\n}\n\n");
        let _ = writeln!(
            out,
            "impl{generics} {CORE}::shapes::Tag for {ident}{generics} {{"
        );
        let _ = writeln!(
            out,
            "    fn tag(&self) -> {OPTION}<&str> {{\n        match self {{"
        );
        for v in members {
            let _ = writeln!(
                out,
                "            Self::{}(_) => {OPTION}::Some({}),",
                v.ident,
                string(&v.wire)
            );
        }
        out.push_str(
            "            Self::Raw { tag, .. } => tag.as_deref(),\n        }\n    }\n}\n\n",
        );
        let _ = writeln!(
            out,
            "impl{} {CORE}::shapes::Tagged<'de> for {ident}{generics} {{",
            self.de_generics()
        );
        let _ = writeln!(
            out,
            "    fn deserialize_tagged<D__: ::serde::Deserializer<'de>>(tag: {OPTION}<&str>, deserializer: D__) -> {RESULT}<Self, D__::Error> {{"
        );
        out.push_str("        match tag {\n");
        for v in members {
            let _ = writeln!(
                out,
                "            {OPTION}::Some({}) => ::serde::Deserialize::deserialize(deserializer).map(Self::{}),",
                string(&v.wire),
                v.ident
            );
        }
        let _ = writeln!(
            out,
            "            _ => <{VALUE} as ::serde::Deserialize>::deserialize(deserializer).map(|value| Self::Raw {{ tag: tag.map(::std::borrow::ToOwned::to_owned), value }}),"
        );
        out.push_str("        }\n    }\n}\n\n");
        let _ = writeln!(
            out,
            "impl{} ::serde::Deserialize<'de> for {ident}{generics} {{",
            self.de_generics()
        );
        let _ = writeln!(
            out,
            "    fn deserialize<D__: ::serde::Deserializer<'de>>(deserializer: D__) -> {RESULT}<Self, D__::Error> {{ <Self as {CORE}::shapes::Tagged<'de>>::deserialize_tagged({OPTION}::None, deserializer) }}\n}}\n"
        );
    }

    fn untagged(&self, members: &[Variant], fallback: usize, out: &mut String) {
        let ident = &self.item.ident;
        self.variants_decl(members, "", out);
        let generics = self.generics();
        let _ = writeln!(
            out,
            "impl{} ::serde::Serialize for {ident}{generics} {{",
            self.ser_generics()
        );
        let _ = writeln!(
            out,
            "    fn serialize<S__: ::serde::Serializer>(&self, serializer: S__) -> {RESULT}<S__::Ok, S__::Error> {{"
        );
        out.push_str("        match self {\n");
        for v in members {
            let _ = writeln!(
                out,
                "            Self::{}(value) => ::serde::Serialize::serialize(value, serializer),",
                v.ident
            );
        }
        out.push_str("        }\n    }\n}\n\n");
        let _ = writeln!(
            out,
            "impl{} ::serde::Deserialize<'de> for {ident}{generics} {{",
            self.de_generics()
        );
        let _ = writeln!(
            out,
            "    /// Always the untyped member, which keeps every property.\n    fn deserialize<D__: ::serde::Deserializer<'de>>(deserializer: D__) -> {RESULT}<Self, D__::Error> {{ ::serde::Deserialize::deserialize(deserializer).map(Self::{}) }}\n}}\n",
            members[fallback].ident
        );
    }

    // --- requests and responses ---

    fn request(&self, r: &Request, out: &mut String) {
        let ident = &self.item.ident;
        let fields: Vec<&Field> = r.path.iter().chain(&r.query).collect();
        let mut types: Vec<&Ty> = fields.iter().map(|f| &f.ty).collect();
        if let Some(body) = &r.body {
            types.push(body);
        }
        let unused = self.unused_params(types.into_iter());
        out.push_str("#[derive(Clone, Debug, PartialEq)]\n#[non_exhaustive]\n");
        let _ = writeln!(
            out,
            "pub struct {ident}{} {{",
            self.declare_generics(&self.item.generics)
        );
        if !r.path.is_empty() {
            out.push_str("    // Path parameters.\n");
            self.fields(&r.path, false, true, out);
        }
        if !r.query.is_empty() {
            out.push_str("    // Query parameters.\n");
            self.fields(&r.query, false, true, out);
        }
        if let Some(body) = &r.body {
            let _ = writeln!(
                out,
                "    /// The body.\n    pub body: {OPTION}<{}>,",
                self.ty(body, Position::Field)
            );
        }
        out.push_str(&self.marker_decl(&unused, false));
        out.push_str("}\n\n");
        let mut all: Vec<Field> = Vec::new();
        for f in fields {
            all.push(Field {
                wire: f.wire.clone(),
                ident: f.ident.clone(),
                ty: f.ty.clone(),
                required: f.required,
                optional: f.optional,
                defaulted: f.defaulted,
                doc: None,
                aliases: Vec::new(),
            });
        }
        if let Some(body) = &r.body {
            all.push(Field {
                wire: "body".to_owned(),
                ident: "body".to_owned(),
                ty: body.clone(),
                required: false,
                optional: true,
                defaulted: false,
                doc: None,
                aliases: Vec::new(),
            });
        }
        let s = Struct {
            fields: all,
            extra: None,
            keyed: None,
            shorthand: None,
            variant: None,
            serde: false,
        };
        self.constructor(&s, &unused, out);
    }

    fn newtype(&self, ty: &Ty, out: &mut String) {
        let ident = &self.item.ident;
        let unused = self.unused_params(std::iter::once(ty));
        out.push_str("#[derive(Clone, Debug, PartialEq, ::serde::Serialize, ::serde::Deserialize)]\n#[serde(transparent)]\n");
        let marker = if unused.is_empty() {
            String::new()
        } else {
            format!(
                ", #[serde(skip)] ::core::marker::PhantomData<fn() -> ({},)>",
                unused.join(", ")
            )
        };
        let _ = writeln!(
            out,
            "pub struct {ident}{}(pub {}{marker});\n",
            self.declare_generics(&self.item.generics),
            self.ty(ty, Position::Field)
        );
    }
}

/// `Box<T>` as (`T`, true), anything else as (it, false).
fn unbox(ty: &str) -> (&str, bool) {
    match ty
        .strip_prefix("::std::boxed::Box<")
        .and_then(|t| t.strip_suffix('>'))
    {
        Some(inner) => (inner, true),
        None => (ty, false),
    }
}

fn describe_kinds(kinds: u8) -> String {
    let mut names = Vec::new();
    for (bit, name) in [
        (STRING, "string"),
        (NUMBER, "number"),
        (BOOLEAN, "boolean"),
        (NULL, "null"),
        (ARRAY, "array"),
        (OBJECT, "object"),
    ] {
        if kinds & bit != 0 {
            names.push(name);
        }
    }
    if names.len() == 6 {
        "value of any kind".to_owned()
    } else {
        names.join(" or ")
    }
}

fn literal_text(literal: &Literal) -> String {
    match literal {
        Literal::String(s) => format!("{s:?}"),
        Literal::Number(n) => n.to_string(),
        Literal::Boolean(b) => b.to_string(),
    }
}

/// A type as code outside every generated crate names it, as the generated
/// tests do.
pub fn outside(ir: &Ir<'_>, ty: &Ty) -> String {
    let item = Item {
        krate: String::new(),
        module: Vec::new(),
        ident: String::new(),
        doc: None,
        generics: Vec::new(),
        kind: ItemKind::Alias(Ty::Unit),
    };
    Emitter {
        ir,
        id: usize::MAX,
        item: &item,
    }
    .ty(ty, Position::Field)
}
