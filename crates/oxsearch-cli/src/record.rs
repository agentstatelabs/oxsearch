//! A document on its line, and which ones a run keeps.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_json::value::RawValue;

use crate::Error;

/// A document as an export writes it, one per line, and an import reads it
/// back: its index, id and routing, and its source as the JSON it is.
#[derive(Debug, Serialize, Deserialize)]
pub struct Record {
    /// The index it was read from.
    #[serde(rename = "_index", default, skip_serializing_if = "Option::is_none")]
    pub index: Option<String>,
    /// Its id.
    #[serde(rename = "_id", default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The routing it was written with, if any.
    #[serde(rename = "_routing", default, skip_serializing_if = "Option::is_none")]
    pub routing: Option<String>,
    /// The document.
    #[serde(rename = "_source")]
    pub source: Box<RawValue>,
}

impl Record {
    /// A record from its line.
    pub fn parse(text: &str, line: u64) -> Result<Record, Error> {
        serde_json::from_str(text).map_err(|e| Error::Line {
            line,
            message: if e.to_string().contains("missing field `_source`") {
                "not a record, with `_source`; plain documents need --plain".to_owned()
            } else {
                e.to_string()
            },
        })
    }
}

/// A plain document from its line, with its id from `id_field` if given.
pub fn plain(
    text: &str,
    line: u64,
    id_field: Option<&str>,
) -> Result<(Box<RawValue>, Option<String>), Error> {
    let bad = |message: String| Error::Line { line, message };
    let fields: HashMap<&str, &RawValue> =
        serde_json::from_str(text).map_err(|e| bad(format!("not a JSON object: {e}")))?;
    let id = match id_field {
        None => None,
        Some(field) => {
            let value = fields
                .get(field)
                .ok_or_else(|| bad(format!("no `{field}` field for the id")))?;
            match serde_json::from_str::<Value>(value.get()) {
                Ok(Value::String(id)) => Some(id),
                Ok(Value::Number(id)) => Some(id.to_string()),
                _ => return Err(bad(format!("`{field}` is not a string or a number"))),
            }
        }
    };
    let source = RawValue::from_string(text.to_owned()).map_err(|e| bad(e.to_string()))?;
    Ok((source, id))
}

/// Which documents a run keeps: one in every `every`, from the first, up to
/// a limit.
#[derive(Clone, Debug)]
pub struct Sample {
    every: u64,
    limit: Option<u64>,
    seen: u64,
    kept: u64,
}

/// What to do with the next document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Keep it.
    Keep,
    /// Pass over it.
    Skip,
    /// The limit is reached: stop reading.
    Stop,
}

impl Sample {
    /// Keeps one document in every `every` (1 keeps them all), at most
    /// `limit` of them.
    pub fn new(every: u64, limit: Option<u64>) -> Sample {
        Sample {
            every: every.max(1),
            limit,
            seen: 0,
            kept: 0,
        }
    }

    /// What to do with the next document.
    pub fn decide(&mut self) -> Decision {
        if self.limit.is_some_and(|limit| self.kept >= limit) {
            return Decision::Stop;
        }
        let position = self.seen;
        self.seen += 1;
        if !position.is_multiple_of(self.every) {
            return Decision::Skip;
        }
        self.kept += 1;
        Decision::Keep
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_keep_the_source_as_it_is() {
        let line = r#"{"_index":"notes","_id":"7","_routing":"ada","_source":{"b":1.10,"a":[1, 2]},"_score":null}"#;
        let record = Record::parse(line, 1).unwrap();
        assert_eq!(record.index.as_deref(), Some("notes"));
        assert_eq!(record.routing.as_deref(), Some("ada"));
        assert_eq!(record.source.get(), r#"{"b":1.10,"a":[1, 2]}"#);
        let written = serde_json::to_string(&record).unwrap();
        assert_eq!(
            written,
            r#"{"_index":"notes","_id":"7","_routing":"ada","_source":{"b":1.10,"a":[1, 2]}}"#
        );
    }

    #[test]
    fn a_plain_document_isnt_a_record() {
        let error = Record::parse(r#"{"title": "x"}"#, 3).unwrap_err();
        assert_eq!(
            error.to_string(),
            "line 3: not a record, with `_source`; plain documents need --plain"
        );
    }

    #[test]
    fn plain_documents_and_their_ids() {
        let (source, id) = plain(r#"{"sku": 42, "n": "x"}"#, 1, Some("sku")).unwrap();
        assert_eq!(
            (source.get(), id.as_deref()),
            (r#"{"sku": 42, "n": "x"}"#, Some("42"))
        );
        let (_, id) = plain(r#"{"sku": "a-1"}"#, 1, Some("sku")).unwrap();
        assert_eq!(id.as_deref(), Some("a-1"));
        assert_eq!(plain(r#"{"n": 1}"#, 1, None).unwrap().1, None);
        for (text, message) in [
            (r#"{"n": 1}"#, "line 2: no `sku` field for the id"),
            (
                r#"{"sku": [1]}"#,
                "line 2: `sku` is not a string or a number",
            ),
        ] {
            assert_eq!(
                plain(text, 2, Some("sku")).unwrap_err().to_string(),
                message
            );
        }
        assert!(plain("[1, 2]", 2, None).is_err());
        assert!(plain(r#"{"n": 1"#, 2, None).is_err());
    }

    #[test]
    fn samples_keep_every_nth_up_to_the_limit() {
        let mut sample = Sample::new(3, Some(2));
        let decisions: Vec<Decision> = (0..6).map(|_| sample.decide()).collect();
        use Decision::{Keep, Skip, Stop};
        assert_eq!(decisions, [Keep, Skip, Skip, Keep, Stop, Stop]);
        let mut all = Sample::new(0, None);
        assert!((0..5).all(|_| all.decide() == Keep));
        assert_eq!(Sample::new(1, Some(0)).decide(), Stop);
    }
}
