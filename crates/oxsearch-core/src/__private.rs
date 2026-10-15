//! Helpers for the generated code. Not part of the API: they change
//! whenever the generator does.

use std::fmt;
use std::marker::PhantomData;

use serde::de::{self, Deserializer, IntoDeserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{self, SerializeMap, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub use indexmap::IndexMap;
pub use serde;
pub use serde_json;

/// The kind of a JSON value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool,
    /// A number.
    Number,
    /// A string.
    String,
    /// An array.
    Array,
    /// An object.
    Object,
}

/// Reads a value by first looking at its JSON kind, without buffering it.
pub trait ByKind<'de>: Sized {
    /// What it reads.
    type Value;
    /// What it expects, for error messages.
    const EXPECTING: &'static str;
    /// Reads the value, of the given kind, from `value`.
    fn visit<D: Deserializer<'de>>(self, kind: Kind, value: D) -> Result<Self::Value, D::Error>;
}

/// Reads a value with a [`ByKind`].
pub fn deserialize_by_kind<'de, D: Deserializer<'de>, K: ByKind<'de>>(
    deserializer: D,
    by_kind: K,
) -> Result<K::Value, D::Error> {
    deserializer.deserialize_any(KindVisitor(by_kind))
}

struct KindVisitor<K>(K);

impl<'de, K: ByKind<'de>> Visitor<'de> for KindVisitor<K> {
    type Value = K::Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(K::EXPECTING)
    }
    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
        self.0.visit(Kind::Bool, v.into_deserializer())
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
        self.0.visit(Kind::Number, v.into_deserializer())
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
        self.0.visit(Kind::Number, v.into_deserializer())
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
        self.0.visit(Kind::Number, v.into_deserializer())
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
        self.0.visit(Kind::String, v.into_deserializer())
    }
    fn visit_borrowed_str<E: de::Error>(self, v: &'de str) -> Result<Self::Value, E> {
        self.0
            .visit(Kind::String, de::value::BorrowedStrDeserializer::new(v))
    }
    fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
        self.0.visit(Kind::String, v.into_deserializer())
    }
    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        self.0.visit(Kind::Null, ().into_deserializer())
    }
    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        self.0.visit(Kind::Null, ().into_deserializer())
    }
    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
        self.0
            .visit(Kind::Array, de::value::SeqAccessDeserializer::new(seq))
    }
    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        self.0
            .visit(Kind::Object, de::value::MapAccessDeserializer::new(map))
    }
}

/// The kind of an already-read JSON value.
pub fn kind_of(value: &Value) -> Kind {
    match value {
        Value::Null => Kind::Null,
        Value::Bool(_) => Kind::Bool,
        Value::Number(_) => Kind::Number,
        Value::String(_) => Kind::String,
        Value::Array(_) => Kind::Array,
        Value::Object(_) => Kind::Object,
    }
}

/// Reads a type from an already-read JSON value, keeping the outer error
/// type.
pub fn from_value<'de, T: Deserialize<'de>, E: de::Error>(value: Value) -> Result<T, E> {
    T::deserialize(value).map_err(E::custom)
}

/// Writes one variant of a container: `{"key": value}`.
pub fn serialize_variant<S: Serializer, V: Serialize + ?Sized>(
    serializer: S,
    key: &str,
    value: &V,
) -> Result<S::Ok, S::Error> {
    let mut map = serializer.serialize_map(Some(1))?;
    map.serialize_entry(key, value)?;
    map.end()
}

/// The `Raw` variant of a container: the whole one-key object.
pub fn raw_variant(key: &str, value: Value) -> Value {
    let mut map = Map::new();
    map.insert(key.to_owned(), value);
    Value::Object(map)
}

/// A container: an object with one property, named after its variant.
pub trait Container<'de>: Sized {
    /// The type's name, for error messages.
    const NAME: &'static str;
    /// Reads the variant `key` names from `map`'s next value; an unknown key
    /// becomes the `Raw` variant.
    fn variant<A: MapAccess<'de>>(key: &str, map: &mut A) -> Result<Self, A::Error>;
}

/// Reads a [`Container`]: exactly one property, read as its variant.
pub fn deserialize_container<'de, D: Deserializer<'de>, C: Container<'de>>(
    deserializer: D,
) -> Result<C, D::Error> {
    struct OneVariant<C>(PhantomData<C>);
    impl<'de, C: Container<'de>> Visitor<'de> for OneVariant<C> {
        type Value = C;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "a {} object with one property", C::NAME)
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<C, A::Error> {
            let Some(key) = map.next_key::<String>()? else {
                return Err(de::Error::custom(format_args!(
                    "{} has no variant",
                    C::NAME
                )));
            };
            let value = C::variant(&key, &mut map)?;
            if let Some(other) = map.next_key::<String>()? {
                return Err(de::Error::custom(format_args!(
                    "{} has two variants, {key} and {other}",
                    C::NAME
                )));
            }
            Ok(value)
        }
    }
    deserializer.deserialize_map(OneVariant(PhantomData))
}

/// The error for a container with two variants.
pub fn two_variants<E: de::Error>(name: &str, key: &str) -> E {
    E::custom(format_args!("{name} has a second variant, {key}"))
}

/// Reads an enum of strings. Booleans and numbers are read as their text,
/// for the enums whose values are `"true"`, `"false"` or numbers.
pub fn deserialize_string_enum<'de, D: Deserializer<'de>, T>(
    deserializer: D,
    name: &'static str,
    from_str: fn(&str) -> T,
) -> Result<T, D::Error> {
    struct Text<T> {
        name: &'static str,
        from_str: fn(&str) -> T,
    }
    impl<'de, T> Visitor<'de> for Text<T> {
        type Value = T;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "a {} value", self.name)
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<T, E> {
            Ok((self.from_str)(v))
        }
        fn visit_bool<E: de::Error>(self, v: bool) -> Result<T, E> {
            Ok((self.from_str)(if v { "true" } else { "false" }))
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<T, E> {
            Ok((self.from_str)(&v.to_string()))
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<T, E> {
            Ok((self.from_str)(&v.to_string()))
        }
        fn visit_f64<E: de::Error>(self, v: f64) -> Result<T, E> {
            Ok((self.from_str)(&v.to_string()))
        }
    }
    deserializer.deserialize_any(Text { name, from_str })
}

/// Writes a member of an internally tagged union: its own properties, and
/// the tag property naming it.
pub fn serialize_tagged<S: Serializer, T: Serialize>(
    serializer: S,
    tag: &str,
    name: &str,
    value: &T,
) -> Result<S::Ok, S::Error> {
    let mut value = serde_json::to_value(value).map_err(ser::Error::custom)?;
    match &mut value {
        Value::Object(map) => {
            map.insert(tag.to_owned(), Value::String(name.to_owned()));
        }
        _ => {
            return Err(ser::Error::custom(
                "a tagged union's member must be an object",
            ));
        }
    }
    value.serialize(serializer)
}

/// A tagged union's member as read: its tag, if it has one, and its
/// properties.
pub type TaggedObject = (Option<String>, Map<String, Value>);

/// Reads an internally tagged union's object and its tag, or the default
/// tag when it has none.
pub fn deserialize_tagged<'de, D: Deserializer<'de>>(
    deserializer: D,
    tag: &str,
    default: Option<&str>,
) -> Result<TaggedObject, D::Error> {
    let map = Map::deserialize(deserializer)?;
    let name = match map.get(tag) {
        Some(Value::String(name)) => Some(name.clone()),
        Some(_) => return Err(de::Error::custom(format_args!("`{tag}` isn't a string"))),
        None => default.map(str::to_owned),
    };
    Ok((name, map))
}

/// A type with a shorthand form: a value standing for one of its properties,
/// such as `"kimchy"` for `{"value": "kimchy"}`.
pub trait Shorthand<'de>: Sized {
    /// The shorthand value's type.
    type Short: Deserialize<'de>;
    /// The type with only the shorthand property set.
    fn from_short(short: Self::Short) -> Self;
    /// Reads the full form.
    fn deserialize_full<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error>;
}

/// Reads a [`Shorthand`] type: an object is the full form, anything else the
/// shorthand.
pub fn deserialize_shorthand<'de, D: Deserializer<'de>, T: Shorthand<'de>>(
    deserializer: D,
) -> Result<T, D::Error> {
    struct Form<T>(PhantomData<T>);
    impl<'de, T: Shorthand<'de>> ByKind<'de> for Form<T> {
        type Value = T;
        const EXPECTING: &'static str = "an object or its shorthand";
        fn visit<D: Deserializer<'de>>(self, kind: Kind, value: D) -> Result<T, D::Error> {
            match kind {
                Kind::Object => T::deserialize_full(value),
                _ => T::Short::deserialize(value).map(T::from_short),
            }
        }
    }
    deserialize_by_kind(deserializer, Form(PhantomData))
}

/// For a field whose `null` means something: a present `null` is
/// `Some(WithNullValue::Null)`, not `None`.
pub fn some<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

/// Whether a JSON object has a property, for telling union members apart.
pub fn has(value: &Value, key: &str) -> bool {
    value.as_object().is_some_and(|map| map.contains_key(key))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[derive(Debug, PartialEq)]
    enum Query {
        Term(u32),
        Raw(Value),
    }

    impl<'de> Container<'de> for Query {
        const NAME: &'static str = "Query";
        fn variant<A: MapAccess<'de>>(key: &str, map: &mut A) -> Result<Self, A::Error> {
            Ok(match key {
                "term" => Query::Term(map.next_value()?),
                _ => Query::Raw(raw_variant(key, map.next_value()?)),
            })
        }
    }

    fn query(json: Value) -> Result<Query, serde_json::Error> {
        deserialize_container(json)
    }

    #[test]
    fn containers() {
        assert_eq!(query(json!({"term": 3})).unwrap(), Query::Term(3));
        assert_eq!(
            query(json!({"new_query": {"a": 1}})).unwrap(),
            Query::Raw(json!({"new_query": {"a": 1}}))
        );
        assert!(query(json!({})).is_err());
        let error = query(json!({"term": 3, "match": 4}))
            .unwrap_err()
            .to_string();
        assert!(error.contains("two variants"), "{error}");
        let error = query(json!({"term": "x"})).unwrap_err().to_string();
        assert!(error.contains("invalid type"), "{error}");
    }

    #[test]
    fn string_enums() {
        let from = |s: &str| s.to_owned();
        let read = |json: Value| deserialize_string_enum(json, "Refresh", from).unwrap();
        assert_eq!(read(json!("wait_for")), "wait_for");
        assert_eq!(read(json!(true)), "true");
        assert_eq!(read(json!(1)), "1");
    }

    #[test]
    fn tagged() {
        let (tag, _) = deserialize_tagged(json!({"type": "text"}), "type", None).unwrap();
        assert_eq!(tag.as_deref(), Some("text"));
        let (tag, _) =
            deserialize_tagged(json!({"properties": {}}), "type", Some("object")).unwrap();
        assert_eq!(tag.as_deref(), Some("object"));
        let written = serialize_tagged(
            serde_json::value::Serializer,
            "type",
            "text",
            &json!({"index": true}),
        )
        .unwrap();
        assert_eq!(written, json!({"type": "text", "index": true}));
    }
}
