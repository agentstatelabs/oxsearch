//! Value shapes the Elasticsearch specification uses that serde doesn't
//! provide: a value or an array of values, an object keyed by a field name,
//! values Elasticsearch sometimes writes as strings, and so on. The
//! generated types are built from these, and from [`IndexMap`] for objects
//! used as maps.

use std::fmt;
use std::marker::PhantomData;

pub use indexmap::IndexMap;
use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};

use crate::__private::{ByKind, Kind, deserialize_by_kind};

/// One value, or an array of them: `"a"` or `["a", "b"]`. Written back in
/// the form it was read.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum OneOrMany<T> {
    /// A single value.
    One(T),
    /// An array of values.
    Many(Vec<T>),
}

impl<T> OneOrMany<T> {
    /// The values, one or many, as a slice.
    pub fn as_slice(&self) -> &[T] {
        match self {
            OneOrMany::One(one) => std::slice::from_ref(one),
            OneOrMany::Many(many) => many,
        }
    }
}

impl<T> From<T> for OneOrMany<T> {
    fn from(one: T) -> Self {
        OneOrMany::One(one)
    }
}

impl<T> From<Vec<T>> for OneOrMany<T> {
    fn from(many: Vec<T>) -> Self {
        OneOrMany::Many(many)
    }
}

impl From<&str> for OneOrMany<String> {
    fn from(one: &str) -> Self {
        OneOrMany::One(one.to_owned())
    }
}

impl<T: Serialize> Serialize for OneOrMany<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            OneOrMany::One(one) => one.serialize(serializer),
            OneOrMany::Many(many) => many.serialize(serializer),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for OneOrMany<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Shape<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> ByKind<'de> for Shape<T> {
            type Value = OneOrMany<T>;
            const EXPECTING: &'static str = "a value or an array of values";
            fn visit<D: Deserializer<'de>>(
                self,
                kind: Kind,
                value: D,
            ) -> Result<Self::Value, D::Error> {
                match kind {
                    Kind::Array => Vec::deserialize(value).map(OneOrMany::Many),
                    _ => T::deserialize(value).map(OneOrMany::One),
                }
            }
        }
        deserialize_by_kind(deserializer, Shape(PhantomData))
    }
}

/// An object with exactly one property, whose name is data: the field a
/// query runs on, as in `{"title": {"query": "hnsw"}}`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SingleKey<K, V> {
    /// The property's name, such as a field name.
    pub key: K,
    /// Its value.
    pub value: V,
}

impl<K, V> SingleKey<K, V> {
    /// A one-property object.
    pub fn new(key: impl Into<K>, value: impl Into<V>) -> Self {
        SingleKey {
            key: key.into(),
            value: value.into(),
        }
    }
}

impl<K: Serialize, V: Serialize> Serialize for SingleKey<K, V> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry(&self.key, &self.value)?;
        map.end()
    }
}

impl<'de, K: Deserialize<'de>, V: Deserialize<'de>> Deserialize<'de> for SingleKey<K, V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct OneEntry<K, V>(PhantomData<(K, V)>);
        impl<'de, K: Deserialize<'de>, V: Deserialize<'de>> Visitor<'de> for OneEntry<K, V> {
            type Value = SingleKey<K, V>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object with one property")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let Some((key, value)) = map.next_entry()? else {
                    return Err(de::Error::invalid_length(0, &self));
                };
                if map.next_key::<de::IgnoredAny>()?.is_some() {
                    return Err(de::Error::invalid_length(2, &self));
                }
                Ok(SingleKey { key, value })
            }
        }
        deserializer.deserialize_map(OneEntry(PhantomData))
    }
}

/// A value Elasticsearch may write as a string, such as `"1"` for `1`.
/// Written back in the form it was read.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Stringified<T> {
    /// The value itself.
    Value(T),
    /// The value written as a string.
    String(String),
}

impl<T> From<T> for Stringified<T> {
    fn from(value: T) -> Self {
        Stringified::Value(value)
    }
}

impl<T: Serialize> Serialize for Stringified<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Stringified::Value(value) => value.serialize(serializer),
            Stringified::String(string) => serializer.serialize_str(string),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Stringified<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Shape<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> ByKind<'de> for Shape<T> {
            type Value = Stringified<T>;
            const EXPECTING: &'static str = "a value, or a value written as a string";
            fn visit<D: Deserializer<'de>>(
                self,
                kind: Kind,
                value: D,
            ) -> Result<Self::Value, D::Error> {
                match kind {
                    Kind::String => String::deserialize(value).map(Stringified::String),
                    _ => T::deserialize(value).map(Stringified::Value),
                }
            }
        }
        deserialize_by_kind(deserializer, Shape(PhantomData))
    }
}

/// A value where `null` means something, such as clearing a setting.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum WithNullValue<T> {
    /// An explicit `null`.
    Null,
    /// A value.
    Value(T),
}

impl<T> From<T> for WithNullValue<T> {
    fn from(value: T) -> Self {
        WithNullValue::Value(value)
    }
}

impl<T: Serialize> Serialize for WithNullValue<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            WithNullValue::Null => serializer.serialize_unit(),
            WithNullValue::Value(value) => value.serialize(serializer),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for WithNullValue<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Shape<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> ByKind<'de> for Shape<T> {
            type Value = WithNullValue<T>;
            const EXPECTING: &'static str = "a value or null";
            fn visit<D: Deserializer<'de>>(
                self,
                kind: Kind,
                value: D,
            ) -> Result<Self::Value, D::Error> {
                match kind {
                    Kind::Null => <()>::deserialize(value).map(|()| WithNullValue::Null),
                    _ => T::deserialize(value).map(WithNullValue::Value),
                }
            }
        }
        deserialize_by_kind(deserializer, Shape(PhantomData))
    }
}

/// One flag, or several joined by `|`, such as `"AND|OR|PREFIX"`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PipeSeparatedFlags<T> {
    /// One flag.
    Flag(T),
    /// Several flags, as written.
    Flags(String),
}

impl<T> From<T> for PipeSeparatedFlags<T> {
    fn from(flag: T) -> Self {
        PipeSeparatedFlags::Flag(flag)
    }
}

impl<T: Serialize> Serialize for PipeSeparatedFlags<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            PipeSeparatedFlags::Flag(flag) => flag.serialize(serializer),
            PipeSeparatedFlags::Flags(flags) => serializer.serialize_str(flags),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for PipeSeparatedFlags<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let flags = String::deserialize(deserializer)?;
        if flags.contains('|') {
            return Ok(PipeSeparatedFlags::Flags(flags));
        }
        T::deserialize(de::value::StringDeserializer::<D::Error>::new(flags))
            .map(PipeSeparatedFlags::Flag)
    }
}

/// A union whose variant is named by the key it sits under, as in
/// `"sterms#by_tag"` when a request asks for `typed_keys`.
pub trait Tag {
    /// The variant's name, if it has one.
    fn tag(&self) -> Option<&str>;
}

/// Reading a [`Tag`]ged union, given the name from its key.
pub trait Tagged<'de>: Sized {
    /// Reads the variant the tag names.
    fn deserialize_tagged<D: Deserializer<'de>>(
        tag: Option<&str>,
        deserializer: D,
    ) -> Result<Self, D::Error>;
}

impl<T: Tag> Tag for Vec<T> {
    fn tag(&self) -> Option<&str> {
        self.first().and_then(Tag::tag)
    }
}

impl<'de, T: Tagged<'de>> Tagged<'de> for Vec<T> {
    fn deserialize_tagged<D: Deserializer<'de>>(
        tag: Option<&str>,
        deserializer: D,
    ) -> Result<Self, D::Error> {
        struct Elements<'t, T>(Option<&'t str>, PhantomData<T>);
        impl<'de, T: Tagged<'de>> Visitor<'de> for Elements<'_, T> {
            type Value = Vec<T>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an array")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut out = Vec::new();
                while let Some(element) = seq.next_element_seed(TaggedSeed(self.0, PhantomData))? {
                    out.push(element);
                }
                Ok(out)
            }
        }
        deserializer.deserialize_seq(Elements(tag, PhantomData))
    }
}

struct TaggedSeed<'t, T>(Option<&'t str>, PhantomData<T>);

impl<'de, T: Tagged<'de>> DeserializeSeed<'de> for TaggedSeed<'_, T> {
    type Value = T;
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<T, D::Error> {
        T::deserialize_tagged(self.0, deserializer)
    }
}

/// An object whose keys carry the type of their values, as `type#name`,
/// which Elasticsearch writes when a request asks for `typed_keys`. Keys
/// without a type are kept as they are.
#[derive(Clone, Debug, PartialEq)]
pub struct TypedKeys<V>(pub IndexMap<String, V>);

impl<V> Default for TypedKeys<V> {
    fn default() -> Self {
        TypedKeys(IndexMap::new())
    }
}

impl<V: Tag + Serialize> Serialize for TypedKeys<V> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (name, value) in &self.0 {
            match value.tag() {
                Some(tag) => map.serialize_entry(&format!("{tag}#{name}"), value)?,
                None => map.serialize_entry(name, value)?,
            }
        }
        map.end()
    }
}

impl<'de, V: Tagged<'de>> Deserialize<'de> for TypedKeys<V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Entries<V>(PhantomData<V>);
        impl<'de, V: Tagged<'de>> Visitor<'de> for Entries<V> {
            type Value = TypedKeys<V>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut out = IndexMap::new();
                while let Some(key) = map.next_key::<String>()? {
                    let (tag, name) = match key.split_once('#') {
                        Some((tag, name)) => (Some(tag.to_owned()), name.to_owned()),
                        None => (None, key),
                    };
                    let value = map.next_value_seed(TaggedSeed(tag.as_deref(), PhantomData))?;
                    out.insert(name, value);
                }
                Ok(TypedKeys(out))
            }
        }
        deserializer.deserialize_map(Entries(PhantomData))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn round_trip<T: Serialize + for<'de> Deserialize<'de> + PartialEq + fmt::Debug>(
        json: serde_json::Value,
    ) -> T {
        let value: T = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(&value).unwrap(), json);
        value
    }

    #[test]
    fn one_or_many() {
        assert_eq!(
            round_trip::<OneOrMany<String>>(json!("a")),
            OneOrMany::One("a".into())
        );
        assert_eq!(
            round_trip::<OneOrMany<String>>(json!(["a", "b"])),
            OneOrMany::Many(vec!["a".into(), "b".into()])
        );
        let error = serde_json::from_str::<OneOrMany<u32>>(r#""x""#).unwrap_err();
        assert!(error.to_string().contains("invalid type"), "{error}");
    }

    #[test]
    fn single_key() {
        let pair = round_trip::<SingleKey<String, u32>>(json!({"title": 3}));
        assert_eq!(pair.key, "title");
        assert!(serde_json::from_value::<SingleKey<String, u32>>(json!({})).is_err());
        assert!(serde_json::from_value::<SingleKey<String, u32>>(json!({"a": 1, "b": 2})).is_err());
    }

    #[test]
    fn stringified_and_null() {
        assert_eq!(
            round_trip::<Stringified<u32>>(json!("1")),
            Stringified::String("1".into())
        );
        assert_eq!(
            round_trip::<Stringified<u32>>(json!(1)),
            Stringified::Value(1)
        );
        assert_eq!(
            round_trip::<WithNullValue<u32>>(json!(null)),
            WithNullValue::Null
        );
        assert_eq!(
            round_trip::<WithNullValue<u32>>(json!(2)),
            WithNullValue::Value(2)
        );
        assert_eq!(
            round_trip::<PipeSeparatedFlags<String>>(json!("AND|OR")),
            PipeSeparatedFlags::Flags("AND|OR".into())
        );
    }

    #[derive(Debug, PartialEq, Serialize)]
    #[serde(untagged)]
    enum Shape {
        Square(u32),
        Raw(serde_json::Value),
    }

    impl Tag for Shape {
        fn tag(&self) -> Option<&str> {
            match self {
                Shape::Square(_) => Some("square"),
                Shape::Raw(_) => None,
            }
        }
    }

    impl<'de> Tagged<'de> for Shape {
        fn deserialize_tagged<D: Deserializer<'de>>(
            tag: Option<&str>,
            d: D,
        ) -> Result<Self, D::Error> {
            match tag {
                Some("square") => u32::deserialize(d).map(Shape::Square),
                _ => serde_json::Value::deserialize(d).map(Shape::Raw),
            }
        }
    }

    #[test]
    fn typed_keys() {
        let json = json!({"square#a": 2, "b": {"x": 1}});
        let keys: TypedKeys<Shape> = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(keys.0["a"], Shape::Square(2));
        assert_eq!(serde_json::to_value(&keys).unwrap(), json);
        let lists: TypedKeys<Vec<Shape>> =
            serde_json::from_value(json!({"square#a": [1, 2]})).unwrap();
        assert_eq!(lists.0["a"], vec![Shape::Square(1), Shape::Square(2)]);
    }
}
