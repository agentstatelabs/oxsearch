//! ES|QL results as Arrow record batches, the columnar format Elasticsearch
//! writes them in with `format=arrow`.

use arrow_array::RecordBatch;
use arrow_ipc::reader::StreamReader;
use oxsearch_core::{Client, Error, Transport};

use crate::{Esql, EsqlFormat};

impl Esql {
    /// Runs the query and reads its result as Arrow record batches, column
    /// by column, without JSON in between: for analysis in Arrow's
    /// ecosystem (Polars, DataFusion, …) or results too large to read row
    /// by row. Each ES|QL column is a field of the batches' schema.
    pub async fn fetch_arrow(
        &self,
        client: &Client<impl Transport>,
    ) -> Result<Vec<RecordBatch>, Error> {
        let request = self.request().format(EsqlFormat::Arrow);
        read_arrow(&client.send_bytes(&request).await?)
    }
}

/// The record batches of an Arrow IPC stream, as an ES|QL query with
/// `format=arrow` answers.
pub fn read_arrow(stream: &[u8]) -> Result<Vec<RecordBatch>, Error> {
    StreamReader::try_new(stream, None)
        .map_err(not_arrow)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(not_arrow)
}

fn not_arrow(e: impl std::fmt::Display) -> Error {
    Error::Decode {
        status: 200,
        source: serde::de::Error::custom(format!("not an Arrow IPC stream: {e}")),
        body: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
    use arrow_ipc::writer::StreamWriter;

    use super::*;

    #[test]
    fn record_batches() {
        let batch = RecordBatch::try_from_iter([
            (
                "title",
                Arc::new(StringArray::from(vec!["HNSW graphs", "Bulk ingest"])) as Arc<dyn Array>,
            ),
            (
                "year",
                Arc::new(Int64Array::from(vec![2024, 2021])) as Arc<dyn Array>,
            ),
        ])
        .unwrap();
        let mut stream = Vec::new();
        let mut writer = StreamWriter::try_new(&mut stream, &batch.schema()).unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();
        drop(writer);

        let batches = read_arrow(&stream).unwrap();
        assert_eq!(batches, [batch]);
        let Err(Error::Decode { source, .. }) = read_arrow(b"{\"columns\": []}") else {
            panic!("JSON isn't an Arrow stream");
        };
        assert!(
            source.to_string().starts_with("not an Arrow IPC stream"),
            "{source}"
        );
    }
}
