//! ES|QL results read into your own types, a row at a time, by column name.

use oxsearch_core::{Client, Error, RowError, Transport};
use serde::de::{
    self, DeserializeOwned, DeserializeSeed, IntoDeserializer, MapAccess, SeqAccess, Visitor,
};
use serde::forward_to_deserialize_any;
use serde_json::{Map, Value};

use crate::{AsyncEsqlResult, Esql, EsqlColumnInfo, EsqlResult};

impl EsqlResult {
    /// The rows, each read into `R` by column name, as
    /// [`read_rows`] does.
    pub fn rows<R: DeserializeOwned>(&self) -> Result<Vec<R>, Error> {
        read_rows(&self.columns, &self.values)
    }

    /// A column's position, by name.
    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| column_name(c) == name)
    }
}

impl AsyncEsqlResult {
    /// The rows so far, each read into `R` by column name, as
    /// [`read_rows`] does.
    pub fn rows<R: DeserializeOwned>(&self) -> Result<Vec<R>, Error> {
        read_rows(&self.columns, &self.values)
    }
}

impl Esql {
    /// Runs the query and reads its rows into `R`, by column name, as
    /// [`read_rows`] does: `query.fetch::<Article>(&client)`.
    pub async fn fetch<R: DeserializeOwned>(
        &self,
        client: &Client<impl Transport>,
    ) -> Result<Vec<R>, Error> {
        client.send(&self.request()).await?.rows()
    }
}

/// Rows, as a result's `values` holds them, read into `R` by column name.
///
/// A column fills the field of its name, and a dotted name
/// (`author.name`) also fills a nested struct's field, unless another
/// column takes the first part's name (`title` beside `title.keyword`).
/// A multi-valued column reads into a `Vec`, and so does a column with a
/// single value, which ES|QL writes without the list, or none. A column
/// without a value reads into an `Option`'s `None`. Columns `R` doesn't
/// name are left out.
///
/// A row that doesn't fit, with a column missing or of another type,
/// fails with [`Error::Row`], which names the row and the column.
pub fn read_rows<R: DeserializeOwned>(
    columns: &[EsqlColumnInfo],
    values: &[Vec<Value>],
) -> Result<Vec<R>, Error> {
    values
        .iter()
        .enumerate()
        .map(|(n, row)| {
            serde_path_to_error::deserialize(Cell(Value::Object(object(columns, row)))).map_err(
                |e| {
                    let path = e.path().to_string();
                    let column = (path != ".").then_some(path);
                    Error::Row(Box::new(RowError::new(
                        n,
                        column,
                        e.into_inner().to_string(),
                        columns.iter().map(|c| column_name(c).to_owned()).collect(),
                    )))
                },
            )
        })
        .collect()
}

/// A row as an object, by column name, with dotted names nested too.
fn object(columns: &[EsqlColumnInfo], row: &[Value]) -> Map<String, Value> {
    let mut object: Map<String, Value> = columns
        .iter()
        .zip(row)
        .map(|(column, value)| (column_name(column).to_owned(), value.clone()))
        .collect();
    for (column, value) in columns.iter().zip(row) {
        let name = column_name(column);
        if name.contains('.') && !value.is_null() {
            nest(&mut object, name, value);
        }
    }
    object
}

fn column_name(column: &EsqlColumnInfo) -> &str {
    column.name.as_deref().unwrap_or_default()
}

/// Puts a value at a dotted name's place in nested objects, unless a
/// column already holds a value on the way.
fn nest(object: &mut Map<String, Value>, name: &str, value: &Value) {
    let mut parts = name.split('.').peekable();
    let mut at = object;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            at.entry(part).or_insert_with(|| value.clone());
            return;
        }
        let next = at.entry(part).or_insert_with(|| Value::Object(Map::new()));
        match next {
            Value::Object(inner) => at = inner,
            _ => return,
        }
    }
}

/// A value of a row, which reads into a list whether it is one or not.
struct Cell(Value);

impl<'de> de::Deserializer<'de> for Cell {
    type Error = serde_json::Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.0 {
            Value::Array(items) => visitor.visit_seq(Cells(items.into_iter())),
            Value::Object(fields) => visitor.visit_map(Fields {
                fields: fields.into_iter(),
                value: None,
            }),
            other => other.deserialize_any(visitor),
        }
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let items = match self.0 {
            Value::Array(items) => items,
            Value::Null => Vec::new(),
            one => vec![one],
        };
        visitor.visit_seq(Cells(items.into_iter()))
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.0 {
            Value::Null => visitor.visit_none(),
            some => visitor.visit_some(Cell(some)),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.0.deserialize_enum(name, variants, visitor)
    }

    forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf
        unit unit_struct tuple tuple_struct map struct identifier ignored_any
    }
}

struct Cells(std::vec::IntoIter<Value>);

impl<'de> SeqAccess<'de> for Cells {
    type Error = serde_json::Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Self::Error> {
        self.0.next().map(|v| seed.deserialize(Cell(v))).transpose()
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.0.len())
    }
}

struct Fields {
    fields: serde_json::map::IntoIter,
    value: Option<Value>,
}

impl<'de> MapAccess<'de> for Fields {
    type Error = serde_json::Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        let Some((key, value)) = self.fields.next() else {
            return Ok(None);
        };
        self.value = Some(value);
        seed.deserialize(key.into_deserializer()).map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, Self::Error> {
        let value = self.value.take().unwrap_or(Value::Null);
        seed.deserialize(Cell(value))
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::json;

    use super::*;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Author {
        name: String,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct Book {
        title: String,
        year: i64,
        tags: Vec<String>,
        rating: Option<f64>,
        author: Author,
    }

    fn result(columns: &[(&str, &str)], values: Value) -> EsqlResult {
        serde_json::from_value(json!({
            "columns": columns.iter().map(|(n, t)| json!({"name": n, "type": t})).collect::<Vec<_>>(),
            "values": values,
        }))
        .unwrap()
    }

    #[test]
    fn rows_by_column_name() {
        let books = result(
            &[
                ("author.name", "keyword"),
                ("rating", "double"),
                ("tags", "keyword"),
                ("title", "text"),
                ("title.keyword", "keyword"),
                ("year", "long"),
            ],
            json!([
                ["Ada", 4.5, ["vectors", "search"], "HNSW", "HNSW", 2024],
                ["Grace", null, "ingest", "Bulk", "Bulk", 2025]
            ]),
        );
        let rows: Vec<Book> = books.rows().unwrap();
        assert_eq!(
            rows[0],
            Book {
                title: "HNSW".into(),
                year: 2024,
                tags: vec!["vectors".into(), "search".into()],
                rating: Some(4.5),
                author: Author { name: "Ada".into() },
            }
        );
        assert_eq!(rows[1].rating, None);
        // A single value reads as a list of one.
        assert_eq!(rows[1].tags, ["ingest"]);
        assert_eq!(books.column("year"), Some(5));

        // A struct may also name the dotted column itself.
        #[derive(Deserialize)]
        struct Flat {
            #[serde(rename = "author.name")]
            author: String,
        }
        let flat: Vec<Flat> = books.rows().unwrap();
        assert_eq!(flat[1].author, "Grace");
    }

    #[test]
    fn rows_that_dont_fit() {
        let books = result(
            &[("title", "text"), ("year", "keyword")],
            json!([["HNSW", "2024"]]),
        );
        let Err(Error::Row(e)) = books.rows::<Book>() else {
            panic!("a mistyped column");
        };
        assert_eq!((e.row, e.column.as_deref()), (0, Some("year")));
        assert!(e.to_string().contains("the columns are title, year"), "{e}");

        let books = result(&[("title", "text")], json!([["HNSW"]]));
        let Err(Error::Row(e)) = books.rows::<Book>() else {
            panic!("a missing column");
        };
        assert_eq!(e.column, None);
        assert!(e.message.contains("missing field `year`"), "{e}");
    }
}
