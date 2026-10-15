//! Derive macros for oxsearch's typed document layer: `#[derive(Document)]`
//! builds an index mapping and typed field handles from a struct. See
//! `oxsearch::document` for the attributes it reads.

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as Tokens};
use quote::quote;
use syn::spanned::Spanned;
use syn::{
    Data, DeriveInput, Fields, GenericArgument, Ident, LitInt, LitStr, PathArguments, Type,
    parse_macro_input,
};

/// Implements `oxsearch::document::Document` and `Mapped` for a struct with
/// named fields, and gives it a constant field handle for each field.
#[proc_macro_derive(
    Document,
    attributes(
        index,
        id,
        routing,
        keyword,
        text,
        date,
        semantic_text,
        dense_vector,
        sparse_vector
    )
)]
pub fn derive_document(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match document(&input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// Date and time types, mapped as `date` without an attribute.
const DATES: &[&str] = &[
    "DateTime",
    "NaiveDate",
    "NaiveDateTime",
    "OffsetDateTime",
    "PrimitiveDateTime",
    "UtcDateTime",
    "Date",
    "Timestamp",
    "Zoned",
    "SystemTime",
];

/// A field's mapped type, when an attribute or its Rust type chooses one.
enum Mapping {
    Keyword,
    Text,
    Date,
    SemanticText {
        inference_id: Option<LitStr>,
        search_inference_id: Option<LitStr>,
    },
    DenseVector {
        dims: Option<LitInt>,
        similarity: Option<Ident>,
        element_type: Option<Ident>,
    },
    SparseVector,
}

struct Field<'a> {
    ident: &'a Ident,
    ty: &'a Type,
    /// The name in JSON, after serde's renames.
    name: String,
    mapping: Option<Mapping>,
    id: bool,
    routing: bool,
}

fn document(input: &DeriveInput) -> syn::Result<Tokens> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new(
            input.generics.span(),
            "a document type can't be generic yet",
        ));
    }
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new(
            input.ident.span(),
            "a document type is a struct with named fields",
        ));
    };
    let Fields::Named(named) = &data.fields else {
        return Err(syn::Error::new(
            input.ident.span(),
            "a document type is a struct with named fields",
        ));
    };
    let mut index = None;
    let mut rename_all = None;
    for attr in &input.attrs {
        if attr.path().is_ident("index") {
            index = Some(attr.parse_args::<LitStr>()?);
        } else if attr.path().is_ident("serde") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("rename_all") {
                    let value: LitStr = meta.value()?.parse().map_err(|_| {
                        meta.error(
                            "oxsearch reads only `rename_all = \"…\"`, one rule for both ways",
                        )
                    })?;
                    rename_all = Some(value);
                } else if meta.input.peek(syn::Token![=]) {
                    meta.value()?.parse::<Tokens>()?;
                } else if meta.input.peek(syn::token::Paren) {
                    meta.input.parse::<proc_macro2::Group>()?;
                }
                Ok(())
            })?;
        }
    }
    let mut fields = Vec::new();
    for field in &named.named {
        if let Some(field) = parse_field(field, rename_all.as_ref())? {
            fields.push(field);
        }
    }
    let ident = &input.ident;
    let index = match &index {
        Some(name) => quote!(::core::option::Option::Some(#name)),
        None => quote!(::core::option::Option::None),
    };
    let p = quote!(::oxsearch::document::__private);
    let mut properties = Vec::new();
    let mut handles = Vec::new();
    for field in &fields {
        let name = &field.name;
        let ty = field.ty;
        let property = match &field.mapping {
            None => quote!(<#ty as #p::Mapped>::property()),
            Some(mapping) => property(mapping, &p)?,
        };
        properties.push(quote! {
            properties.insert(::std::string::String::from(#name), #property);
        });
        let kind = match &field.mapping {
            None => quote!(<#ty as #p::Mapped>::Kind),
            Some(Mapping::Keyword) => quote!(#p::kind::Keyword),
            Some(Mapping::Text) => quote!(#p::kind::Text),
            Some(Mapping::Date) => quote!(#p::kind::Date),
            Some(Mapping::SemanticText { .. }) => quote!(#p::kind::SemanticText),
            Some(Mapping::DenseVector { .. }) => quote!(#p::kind::DenseVector),
            Some(Mapping::SparseVector) => quote!(#p::kind::SparseVector),
        };
        let constant = Ident::new(
            &screaming(field.ident.to_string().trim_start_matches("r#")),
            field.ident.span(),
        );
        let doc = format!("The `{name}` field.");
        handles.push(quote! {
            #[doc = #doc]
            pub const #constant: #p::Field<#kind> = #p::Field::new(#name);
        });
    }
    let id = accessor(&fields, |f| f.id, "id", &p)?;
    let routing = accessor(&fields, |f| f.routing, "routing", &p)?;
    Ok(quote! {
        impl #p::Document for #ident {
            const INDEX: ::core::option::Option<&'static str> = #index;

            fn properties() -> #p::IndexMap<::std::string::String, #p::Property> {
                let mut properties = #p::IndexMap::new();
                #(#properties)*
                properties
            }

            #id
            #routing
        }

        impl #p::Mapped for #ident {
            type Kind = #p::kind::Object;

            fn property() -> #p::Property {
                #p::Property::Object(
                    #p::ObjectProperty::new().properties(<Self as #p::Document>::properties()),
                )
            }
        }

        impl #ident {
            #(#handles)*
        }
    })
}

fn parse_field<'a>(
    field: &'a syn::Field,
    rename_all: Option<&LitStr>,
) -> syn::Result<Option<Field<'a>>> {
    let ident = field.ident.as_ref().expect("named fields have names");
    let mut name = None;
    let mut skip = false;
    let mut mapping = None;
    let mut id = false;
    let mut routing = false;
    let set = |mapping: &mut Option<Mapping>, new: Mapping, span: Span| {
        if mapping.is_some() {
            return Err(syn::Error::new(span, "a field has one mapped type"));
        }
        *mapping = Some(new);
        Ok(())
    };
    for attr in &field.attrs {
        let path = attr.path();
        if path.is_ident("serde") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("rename") {
                    let value: LitStr = meta.value()?.parse().map_err(|_| {
                        meta.error("oxsearch reads only `rename = \"…\"`, one name for both ways")
                    })?;
                    name = Some(value.value());
                } else if meta.path.is_ident("skip")
                    || meta.path.is_ident("skip_serializing")
                    || meta.path.is_ident("skip_deserializing")
                {
                    skip |= meta.path.is_ident("skip");
                } else if meta.path.is_ident("flatten") {
                    return Err(meta.error("oxsearch can't map a flattened field yet"));
                } else if meta.input.peek(syn::Token![=]) {
                    meta.value()?.parse::<Tokens>()?;
                } else if meta.input.peek(syn::token::Paren) {
                    meta.input.parse::<proc_macro2::Group>()?;
                }
                Ok(())
            })?;
        } else if path.is_ident("id") {
            id = true;
        } else if path.is_ident("routing") {
            routing = true;
        } else if path.is_ident("keyword") {
            set(&mut mapping, Mapping::Keyword, attr.span())?;
        } else if path.is_ident("text") {
            set(&mut mapping, Mapping::Text, attr.span())?;
        } else if path.is_ident("date") {
            set(&mut mapping, Mapping::Date, attr.span())?;
        } else if path.is_ident("sparse_vector") {
            set(&mut mapping, Mapping::SparseVector, attr.span())?;
        } else if path.is_ident("semantic_text") {
            let mut inference_id = None;
            let mut search_inference_id = None;
            if !matches!(attr.meta, syn::Meta::Path(_)) {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("inference_id") {
                        inference_id = Some(meta.value()?.parse()?);
                    } else if meta.path.is_ident("search_inference_id") {
                        search_inference_id = Some(meta.value()?.parse()?);
                    } else {
                        return Err(meta.error(
                            "semantic_text takes `inference_id` and `search_inference_id`",
                        ));
                    }
                    Ok(())
                })?;
            }
            set(
                &mut mapping,
                Mapping::SemanticText {
                    inference_id,
                    search_inference_id,
                },
                attr.span(),
            )?;
        } else if path.is_ident("dense_vector") {
            let mut dims = None;
            let mut similarity = None;
            let mut element_type = None;
            if !matches!(attr.meta, syn::Meta::Path(_)) {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("dims") {
                        dims = Some(meta.value()?.parse()?);
                    } else if meta.path.is_ident("similarity") {
                        let value: LitStr = meta.value()?.parse()?;
                        let variant = match value.value().as_str() {
                            "cosine" => "Cosine",
                            "dot_product" => "DotProduct",
                            "l2_norm" => "L2Norm",
                            "max_inner_product" => "MaxInnerProduct",
                            _ => {
                                return Err(syn::Error::new(
                                    value.span(),
                                    "similarity is cosine, dot_product, l2_norm or max_inner_product",
                                ));
                            }
                        };
                        similarity = Some(Ident::new(variant, value.span()));
                    } else if meta.path.is_ident("element_type") {
                        let value: LitStr = meta.value()?.parse()?;
                        let variant = match value.value().as_str() {
                            "float" => "Float",
                            "byte" => "Byte",
                            "bit" => "Bit",
                            "bfloat16" => "Bfloat16",
                            _ => {
                                return Err(syn::Error::new(
                                    value.span(),
                                    "element_type is float, byte, bit or bfloat16",
                                ));
                            }
                        };
                        element_type = Some(Ident::new(variant, value.span()));
                    } else {
                        return Err(meta.error(
                            "dense_vector takes `dims`, `similarity` and `element_type`",
                        ));
                    }
                    Ok(())
                })?;
            }
            set(
                &mut mapping,
                Mapping::DenseVector {
                    dims,
                    similarity,
                    element_type,
                },
                attr.span(),
            )?;
        }
    }
    if skip {
        return Ok(None);
    }
    if mapping.is_none() && DATES.contains(&innermost(&field.ty).as_str()) {
        mapping = Some(Mapping::Date);
    }
    let raw = ident.to_string();
    let raw = raw.trim_start_matches("r#");
    let name = match (name, rename_all) {
        (Some(name), _) => name,
        (None, Some(rule)) => rename(raw, &rule.value())
            .ok_or_else(|| syn::Error::new(rule.span(), "an unknown rename_all rule"))?,
        (None, None) => raw.to_owned(),
    };
    Ok(Some(Field {
        ident,
        ty: &field.ty,
        name,
        mapping,
        id,
        routing,
    }))
}

fn property(mapping: &Mapping, p: &Tokens) -> syn::Result<Tokens> {
    Ok(match mapping {
        Mapping::Keyword => quote!(#p::Property::Keyword(#p::KeywordProperty::new())),
        Mapping::Text => quote!(#p::Property::Text(#p::TextProperty::new())),
        Mapping::Date => quote!(#p::Property::Date(#p::DateProperty::new())),
        Mapping::SparseVector => {
            quote!(#p::Property::SparseVector(#p::SparseVectorProperty::new()))
        }
        Mapping::SemanticText {
            inference_id,
            search_inference_id,
        } => {
            let inference_id = inference_id.iter();
            let search_inference_id = search_inference_id.iter();
            quote!(#p::Property::SemanticText(
                #p::SemanticTextProperty::new()
                    #(.inference_id(#inference_id))*
                    #(.search_inference_id(#search_inference_id))*
            ))
        }
        Mapping::DenseVector {
            dims,
            similarity,
            element_type,
        } => {
            let dims = dims.iter();
            let similarity = similarity.iter();
            let element_type = element_type.iter();
            quote!(#p::Property::DenseVector(
                #p::DenseVectorProperty::new()
                    #(.dims(#dims))*
                    #(.similarity(#p::DenseVectorSimilarity::#similarity))*
                    #(.element_type(#p::DenseVectorElementType::#element_type))*
            ))
        }
    })
}

/// The `id` or `routing` method, from the one field marked for it.
fn accessor(
    fields: &[Field<'_>],
    marked: impl Fn(&Field<'_>) -> bool,
    method: &str,
    p: &Tokens,
) -> syn::Result<Tokens> {
    let mut chosen = fields.iter().filter(|f| marked(f));
    let Some(field) = chosen.next() else {
        return Ok(Tokens::new());
    };
    if let Some(other) = chosen.next() {
        return Err(syn::Error::new(
            other.ident.span(),
            format!("one field gives a document's {method}"),
        ));
    }
    let ident = field.ident;
    let method = Ident::new(method, Span::call_site());
    let value = if outer(field.ty).as_deref() == Some("Option") {
        quote!(self.#ident.as_ref().map(#p::text))
    } else {
        quote!(::core::option::Option::Some(#p::text(&self.#ident)))
    };
    Ok(quote! {
        fn #method(&self) -> ::core::option::Option<::std::string::String> {
            #value
        }
    })
}

/// The last segment of a type's path.
fn outer(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(path) => path.path.segments.last().map(|s| s.ident.to_string()),
        _ => None,
    }
}

/// The type inside any `Option`, `Vec`, `Box`, set or array: what a field
/// holds one or many of.
fn innermost(ty: &Type) -> String {
    match ty {
        Type::Array(array) => innermost(&array.elem),
        Type::Slice(slice) => innermost(&slice.elem),
        Type::Path(path) => {
            let Some(last) = path.path.segments.last() else {
                return String::new();
            };
            let wrapper = matches!(
                last.ident.to_string().as_str(),
                "Option" | "Vec" | "Box" | "VecDeque" | "BTreeSet" | "HashSet"
            );
            if let (true, PathArguments::AngleBracketed(args)) = (wrapper, &last.arguments)
                && let Some(GenericArgument::Type(inner)) = args.args.first()
            {
                return innermost(inner);
            }
            last.ident.to_string()
        }
        _ => String::new(),
    }
}

/// A Rust field name as serde's `rename_all` rule writes it.
fn rename(field: &str, rule: &str) -> Option<String> {
    let words: Vec<&str> = field.split('_').filter(|w| !w.is_empty()).collect();
    let capitalized = |w: &&str| {
        let mut chars = w.chars();
        chars.next().map_or_else(String::new, |c| {
            c.to_uppercase().collect::<String>() + chars.as_str()
        })
    };
    Some(match rule {
        "lowercase" => field.to_lowercase(),
        "UPPERCASE" => field.to_uppercase(),
        "PascalCase" => words.iter().map(capitalized).collect(),
        "camelCase" => {
            let mut out = words.first().map(|w| (*w).to_owned()).unwrap_or_default();
            out.extend(words.iter().skip(1).map(capitalized));
            out
        }
        "snake_case" => field.to_owned(),
        "SCREAMING_SNAKE_CASE" => field.to_uppercase(),
        "kebab-case" => field.replace('_', "-"),
        "SCREAMING-KEBAB-CASE" => field.replace('_', "-").to_uppercase(),
        _ => return None,
    })
}

/// A field name as a constant's.
fn screaming(field: &str) -> String {
    field.to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames() {
        assert_eq!(rename("published_at", "camelCase").unwrap(), "publishedAt");
        assert_eq!(rename("published_at", "PascalCase").unwrap(), "PublishedAt");
        assert_eq!(
            rename("published_at", "kebab-case").unwrap(),
            "published-at"
        );
        assert_eq!(
            rename("published_at", "SCREAMING_SNAKE_CASE").unwrap(),
            "PUBLISHED_AT"
        );
        assert!(rename("x", "Title Case").is_none());
    }

    #[test]
    fn inner_types() {
        let ty: Type = syn::parse_quote!(Option<Vec<chrono::DateTime<chrono::Utc>>>);
        assert_eq!(innermost(&ty), "DateTime");
        assert_eq!(outer(&ty).as_deref(), Some("Option"));
    }
}
