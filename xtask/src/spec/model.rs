//! A serde model of the specification's `schema.json`, the input the
//! generator reads.
//!
//! Every struct and enum here denies unknown fields, so a specification whose
//! format has changed fails to load, with the path of the first field the
//! model doesn't know, instead of being read in part. Optional fields are
//! `Option`s, or default to empty when the format leaves them out when empty.

use std::collections::BTreeMap;
use std::fmt;

use serde::Deserialize;

/// The whole specification: every endpoint and every type.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schema {
    /// What the file is, and its license.
    #[serde(rename = "_info")]
    pub info: Info,
    /// The API's endpoints.
    pub endpoints: Vec<Endpoint>,
    /// Every type, including each endpoint's request and response.
    pub types: Vec<TypeDefinition>,
}

/// The file's title and license.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Info {
    /// The specification's title.
    pub title: String,
    /// The license the specification is published under.
    pub license: InfoLicense,
}

/// A license, by name and link.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InfoLicense {
    /// The license's name, such as `Apache 2.0`.
    pub name: String,
    /// A link to its text.
    pub url: String,
}

/// A type's name: the namespace it is declared in, such as
/// `_types.query_dsl`, and its name there.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(deny_unknown_fields)]
pub struct TypeName {
    /// The namespace, with `.` between its parts.
    pub namespace: String,
    /// The name within the namespace.
    pub name: String,
}

impl fmt::Display for TypeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}::{}", self.namespace, self.name)
    }
}

/// The type of a value: a property's, a body's, or a generic argument's.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ValueOf {
    /// An instance of a named type, with its generic arguments.
    InstanceOf {
        /// The type.
        #[serde(rename = "type")]
        type_name: TypeName,
        /// Its generic arguments, in order.
        #[serde(default)]
        generics: Vec<ValueOf>,
    },
    /// An array of values.
    ArrayOf {
        /// The type of each element.
        value: Box<ValueOf>,
    },
    /// One of several types; how to tell them apart is up to the reader.
    UnionOf {
        /// The types it may be.
        items: Vec<ValueOf>,
    },
    /// An object used as a map.
    #[serde(rename_all = "camelCase")]
    DictionaryOf {
        /// The type of the keys.
        key: Box<ValueOf>,
        /// The type of the values.
        value: Box<ValueOf>,
        /// Whether the object always has exactly one key.
        single_key: bool,
    },
    /// Any value the user chooses, such as a document's source.
    UserDefinedValue {},
    /// A constant.
    LiteralValue {
        /// The constant's value.
        value: Literal,
    },
}

/// A literal value.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Literal {
    /// A string.
    String(String),
    /// A number.
    Number(serde_json::Number),
    /// A boolean.
    Boolean(bool),
}

/// A type declared in the specification.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TypeDefinition {
    /// An object type with properties.
    Interface(Interface),
    /// An endpoint's request.
    Request(Request),
    /// An endpoint's response.
    Response(Response),
    /// A set of string values.
    Enum(Enum),
    /// Another name for a type, often a union.
    TypeAlias(TypeAlias),
}

impl TypeDefinition {
    /// The type's name.
    pub fn name(&self) -> &TypeName {
        match self {
            TypeDefinition::Interface(t) => &t.name,
            TypeDefinition::Request(t) => &t.name,
            TypeDefinition::Response(t) => &t.name,
            TypeDefinition::Enum(t) => &t.name,
            TypeDefinition::TypeAlias(t) => &t.name,
        }
    }

    /// The kind of type, as `schema.json` names it.
    pub fn kind(&self) -> &'static str {
        match self {
            TypeDefinition::Interface(_) => "interface",
            TypeDefinition::Request(_) => "request",
            TypeDefinition::Response(_) => "response",
            TypeDefinition::Enum(_) => "enum",
            TypeDefinition::TypeAlias(_) => "type_alias",
        }
    }
}

/// An object type.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Interface {
    /// The type's name.
    pub name: TypeName,
    /// Where it is declared in the specification's TypeScript sources.
    pub spec_location: String,
    /// What it is.
    pub description: Option<String>,
    /// Its generic parameters.
    #[serde(default)]
    pub generics: Vec<TypeName>,
    /// The type it extends.
    pub inherits: Option<Inherits>,
    /// Behaviors it implements, such as additional properties.
    #[serde(default)]
    pub behaviors: Vec<Inherits>,
    /// The names of behaviors it or its parents implement.
    #[serde(default)]
    pub attached_behaviors: Vec<String>,
    /// Its properties.
    pub properties: Vec<Property>,
    /// The property a value of another type stands for, when the type has a
    /// shorthand form.
    pub shortcut_property: Option<String>,
    /// The tag value naming this type in an internally tagged union.
    pub variant_name: Option<String>,
    /// How its variants are told apart, when it is a container.
    pub variants: Option<Variants>,
    /// The documentation's id for it.
    pub doc_id: Option<String>,
    /// A link to its documentation.
    pub doc_url: Option<String>,
    /// The id of further documentation.
    pub ext_doc_id: Option<String>,
    /// A link to further documentation.
    pub ext_doc_url: Option<String>,
    /// Where Elasticsearch's behavior departs from what the type says.
    pub es_quirk: Option<String>,
}

/// A request: path and query parameters, and a body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    /// The type's name.
    pub name: TypeName,
    /// Where it is declared in the specification's TypeScript sources.
    pub spec_location: String,
    /// What the endpoint does.
    pub description: Option<String>,
    /// Set when the request is deprecated.
    pub deprecation: Option<Deprecation>,
    /// Its generic parameters.
    #[serde(default)]
    pub generics: Vec<TypeName>,
    /// The type it extends.
    pub inherits: Option<Inherits>,
    /// The names of behaviors it or its parents implement.
    #[serde(default)]
    pub attached_behaviors: Vec<String>,
    /// The path parameters.
    pub path: Vec<Property>,
    /// The query parameters.
    pub query: Vec<Property>,
    /// The body.
    pub body: Body,
    /// Example requests, by name.
    #[serde(default)]
    pub examples: BTreeMap<String, Example>,
}

/// A response: its body and the error responses it documents.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Response {
    /// The type's name.
    pub name: TypeName,
    /// Where it is declared in the specification's TypeScript sources.
    pub spec_location: String,
    /// Its generic parameters.
    #[serde(default)]
    pub generics: Vec<TypeName>,
    /// The body.
    pub body: Body,
    /// Responses with other status codes, and their bodies.
    #[serde(default)]
    pub exceptions: Vec<ResponseException>,
    /// Example responses, by name.
    #[serde(default)]
    pub examples: BTreeMap<String, Example>,
}

/// A response with another status code.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResponseException {
    /// When it happens.
    pub description: Option<String>,
    /// Its body.
    pub body: Body,
    /// The status codes it comes with.
    pub status_codes: Vec<u16>,
}

/// A request's or response's body.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Body {
    /// No body.
    NoBody {},
    /// An object with these properties.
    Properties {
        /// The properties.
        properties: Vec<Property>,
    },
    /// A value of another type.
    #[serde(rename_all = "camelCase")]
    Value {
        /// The value's type.
        value: ValueOf,
        /// The name to give the body in generated code.
        codegen_name: Option<String>,
    },
}

/// An example request or response from the specification.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Example {
    /// A one-line summary.
    pub summary: Option<String>,
    /// What it shows.
    pub description: Option<String>,
    /// The request line, such as `POST /_bulk`.
    pub method_request: Option<String>,
    /// The body, as JSON text (or NDJSON for bulk requests).
    pub value: Option<String>,
    /// `request` or `response`.
    #[serde(rename = "type")]
    pub example_type: Option<String>,
    /// For a response, its status code.
    pub response_code: Option<u16>,
    /// Where the example applies.
    pub availability: Option<Availabilities>,
    /// The same request written with the official clients.
    #[serde(default)]
    pub alternatives: Vec<ExampleAlternative>,
}

/// An example written in a client language.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExampleAlternative {
    /// The language.
    pub language: String,
    /// The code.
    pub code: String,
}

/// A set of string values.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Enum {
    /// The type's name.
    pub name: TypeName,
    /// Where it is declared in the specification's TypeScript sources.
    pub spec_location: String,
    /// The values.
    pub members: Vec<EnumMember>,
    /// Whether Elasticsearch accepts values beyond the members.
    #[serde(default)]
    pub is_open: bool,
    /// Where Elasticsearch's behavior departs from what the type says.
    pub es_quirk: Option<String>,
}

/// One value of an enum.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnumMember {
    /// The value.
    pub name: String,
    /// Other values Elasticsearch accepts for it.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// What it means.
    pub description: Option<String>,
    /// The name to give it in generated code.
    pub codegen_name: Option<String>,
    /// Set when it is deprecated.
    pub deprecation: Option<Deprecation>,
    /// Where it is available.
    pub availability: Option<Availabilities>,
}

/// Another name for a type.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TypeAlias {
    /// The type's name.
    pub name: TypeName,
    /// Where it is declared in the specification's TypeScript sources.
    pub spec_location: String,
    /// What it is.
    pub description: Option<String>,
    /// Its generic parameters.
    #[serde(default)]
    pub generics: Vec<TypeName>,
    /// The type it names.
    #[serde(rename = "type")]
    pub value: ValueOf,
    /// How the members of a union are told apart.
    pub variants: Option<Variants>,
    /// Names for a union's members in generated code, in order.
    #[serde(default)]
    pub codegen_names: Vec<String>,
    /// The documentation's id for it.
    pub doc_id: Option<String>,
    /// A link to its documentation.
    pub doc_url: Option<String>,
    /// The id of further documentation.
    pub ext_doc_id: Option<String>,
    /// A link to further documentation.
    pub ext_doc_url: Option<String>,
}

/// How a container's or a union's variants are told apart.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Variants {
    /// An object with one property set, named after the variant, such as a
    /// query: `{"match": {...}}`. The container's other properties, if
    /// any, are marked `container_property`.
    #[serde(rename_all = "camelCase")]
    Container {
        /// Whether Elasticsearch may add variants the specification doesn't
        /// list.
        #[serde(default)]
        non_exhaustive: bool,
    },
    /// An object whose `tag` property names the variant.
    #[serde(rename_all = "camelCase")]
    InternalTag {
        /// The property that names the variant.
        tag: String,
        /// The variant when the tag is missing.
        default_tag: Option<String>,
        /// Whether Elasticsearch may add variants the specification doesn't
        /// list.
        #[serde(default)]
        non_exhaustive: bool,
    },
    /// The variant's name is the key the value sits under in its parent.
    #[serde(rename_all = "camelCase")]
    ExternalTag {
        /// Whether Elasticsearch may add variants the specification doesn't
        /// list.
        #[serde(default)]
        non_exhaustive: bool,
    },
    /// Nothing names the variant; it is told apart by its shape.
    #[serde(rename_all = "camelCase")]
    Untagged {
        /// The variant used when no other one matches.
        untyped_variant: TypeName,
    },
}

/// A type a type extends, or a behavior it implements, with generic
/// arguments.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inherits {
    /// The type or behavior.
    #[serde(rename = "type")]
    pub type_name: TypeName,
    /// Its generic arguments, in order.
    #[serde(default)]
    pub generics: Vec<ValueOf>,
    /// Settings for a behavior, such as the field name additional
    /// properties go under.
    #[serde(default)]
    pub meta: BTreeMap<String, String>,
}

/// A property of an object, or a path or query parameter.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Property {
    /// The name on the wire.
    pub name: String,
    /// The value's type.
    #[serde(rename = "type")]
    pub value: ValueOf,
    /// Whether it must be present.
    pub required: bool,
    /// What it is.
    pub description: Option<String>,
    /// Elasticsearch's value when it is absent.
    pub server_default: Option<serde_json::Value>,
    /// Other names Elasticsearch accepts for it.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// The name to give it in generated code.
    pub codegen_name: Option<String>,
    /// In a container, a property that sits beside the variant rather than
    /// naming one.
    #[serde(default)]
    pub container_property: bool,
    /// Set when it is deprecated.
    pub deprecation: Option<Deprecation>,
    /// Where it is available.
    pub availability: Option<Availabilities>,
    /// The documentation's id for it.
    pub doc_id: Option<String>,
    /// A link to its documentation.
    pub doc_url: Option<String>,
    /// The id of further documentation.
    pub ext_doc_id: Option<String>,
    /// A link to further documentation.
    pub ext_doc_url: Option<String>,
    /// Where Elasticsearch's behavior departs from what the type says.
    pub es_quirk: Option<String>,
}

/// A deprecation: since when, and what to use instead.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deprecation {
    /// The version it was deprecated in.
    pub version: String,
    /// Why, and what to use instead.
    pub description: String,
}

/// Where something is available: on the self-managed stack, on serverless,
/// or both.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Availabilities {
    /// On the self-managed stack.
    pub stack: Option<Availability>,
    /// On serverless.
    pub serverless: Option<Availability>,
}

/// Availability in one flavor of Elasticsearch.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Availability {
    /// The version it appeared in.
    pub since: Option<String>,
    /// How stable it is.
    pub stability: Option<Stability>,
    /// Who can see it.
    pub visibility: Option<Visibility>,
    /// The feature flag that enables it.
    pub feature_flag: Option<String>,
}

/// How stable an API is.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Stability {
    /// Stable.
    Stable,
    /// Beta: may change.
    Beta,
    /// Experimental: may change or go away.
    Experimental,
    /// A technical preview: may change or go away.
    TechPreview,
}

/// Who can see an API.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    /// Everyone.
    Public,
    /// Elasticsearch's own components only.
    Private,
    /// Only when a feature flag is set.
    FeatureFlag,
}

/// An endpoint.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Endpoint {
    /// Its name, such as `search` or `indices.create`.
    pub name: String,
    /// What it does.
    pub description: String,
    /// Its request type.
    pub request: Option<TypeName>,
    /// Its response type.
    pub response: Option<TypeName>,
    /// Whether the request must have a body.
    pub request_body_required: bool,
    /// Its paths and the methods each accepts.
    pub urls: Vec<UrlTemplate>,
    /// The media types the request body may have.
    #[serde(default)]
    pub request_media_type: Vec<String>,
    /// The media types the response body may have.
    #[serde(default)]
    pub response_media_type: Vec<String>,
    /// Where it is available.
    pub availability: Availabilities,
    /// The privileges it needs.
    pub privileges: Option<Privileges>,
    /// Set when it is deprecated.
    pub deprecation: Option<Deprecation>,
    /// Whether client generators should leave it out.
    #[serde(default)]
    pub codegen_exclude: bool,
    /// The documentation's id for it.
    pub doc_id: Option<String>,
    /// A link to its documentation.
    pub doc_url: Option<String>,
    /// A link to its serverless documentation.
    pub doc_url_serverless: Option<String>,
    /// The documentation section it belongs to.
    pub doc_tag: Option<String>,
    /// The id of further documentation.
    pub ext_doc_id: Option<String>,
    /// A link to further documentation.
    pub ext_doc_url: Option<String>,
    /// What the further documentation covers.
    pub ext_doc_description: Option<String>,
    /// A link to the previous major version's documentation.
    pub ext_previous_version_doc_url: Option<String>,
}

/// One of an endpoint's paths.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UrlTemplate {
    /// The path, with `{parameter}` placeholders.
    pub path: String,
    /// The HTTP methods it accepts.
    pub methods: Vec<String>,
    /// Set when this path is deprecated.
    pub deprecation: Option<Deprecation>,
}

/// The privileges an endpoint needs.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Privileges {
    /// Cluster privileges.
    #[serde(default)]
    pub cluster: Vec<String>,
    /// Index privileges.
    #[serde(default)]
    pub index: Vec<String>,
}
