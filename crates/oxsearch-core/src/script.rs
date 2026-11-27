//! Short forms for the common scripts.

use indexmap::IndexMap;
use serde_json::Value;

use crate::Script;

impl Script {
    /// A script from its source, in Painless unless `lang` says otherwise:
    /// `Script::inline("ctx._source.views += params.by").param("by", 1)`.
    pub fn inline(source: impl Into<String>) -> Script {
        Script::new().source(source.into())
    }

    /// Sets one of the script's parameters. Elasticsearch compiles a script
    /// once for any values of its parameters, and again for each change to
    /// its source.
    pub fn param(mut self, name: impl Into<String>, value: impl Into<Value>) -> Script {
        self.params
            .get_or_insert_with(IndexMap::new)
            .insert(name.into(), value.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn inline_with_parameters() {
        let script = Script::inline("ctx._source.views += params.by").param("by", 2);
        assert_eq!(
            serde_json::to_value(&script).unwrap(),
            json!({"source": "ctx._source.views += params.by", "params": {"by": 2}})
        );
    }
}
