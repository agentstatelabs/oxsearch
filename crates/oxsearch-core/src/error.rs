//! What can go wrong in a call.

use std::fmt;

use serde_json::Value;

/// An error from a transport: the request couldn't be sent, or no response
/// came back.
pub type TransportError = Box<dyn std::error::Error + Send + Sync>;

/// What can go wrong in a call.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The request couldn't be sent, or no response came back.
    Transport(TransportError),
    /// Elasticsearch answered with an error.
    Elasticsearch(Box<ElasticsearchError>),
    /// The cluster's license doesn't include a feature the request uses,
    /// such as server-side RRF on the Basic license.
    License(Box<LicenseError>),
    /// A write conditional on a document's version found another version:
    /// the document changed since it was read (status 409).
    Conflict(Box<ElasticsearchError>),
    /// The request couldn't be written as JSON.
    Encode(serde_json::Error),
    /// A parameter can't be written in a URL, or no URL fits the parameters
    /// set.
    Parameter(String),
    /// The response didn't fit the type it was read into.
    Decode {
        /// The response's status code.
        status: u16,
        /// What didn't fit.
        source: serde_json::Error,
        /// The start of the response, for diagnosis.
        body: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Transport(e) => write!(f, "transport: {e}"),
            Error::Elasticsearch(e) => e.fmt(f),
            Error::License(e) => e.fmt(f),
            Error::Conflict(e) => e.fmt(f),
            Error::Encode(e) => write!(f, "writing the request: {e}"),
            Error::Parameter(e) => write!(f, "parameters: {e}"),
            Error::Decode {
                status,
                source,
                body,
            } => {
                write!(f, "reading a {status} response: {source}: {body}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Transport(e) => Some(e.as_ref()),
            Error::Encode(e) | Error::Decode { source: e, .. } => Some(e),
            _ => None,
        }
    }
}

/// An error response from Elasticsearch.
#[derive(Debug)]
#[non_exhaustive]
pub struct ElasticsearchError {
    /// The status code.
    pub status: u16,
    /// The error, as Elasticsearch describes it, when the body is one.
    pub response: Option<crate::ErrorResponseBase>,
    /// The body, as JSON when it is JSON.
    pub body: Value,
}

impl ElasticsearchError {
    /// The error's type, such as `index_not_found_exception`.
    pub fn error_type(&self) -> Option<&str> {
        self.body.pointer("/error/type").and_then(Value::as_str)
    }

    /// Why it happened.
    pub fn reason(&self) -> Option<&str> {
        self.body.pointer("/error/reason").and_then(Value::as_str)
    }
}

impl fmt::Display for ElasticsearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Elasticsearch answered {}", self.status)?;
        if let Some(kind) = self.error_type() {
            write!(f, ": {kind}")?;
        }
        if let Some(reason) = self.reason() {
            write!(f, ": {reason}")?;
        }
        Ok(())
    }
}

/// A request Elasticsearch refused because the cluster's license doesn't
/// include a feature it uses.
#[derive(Debug)]
#[non_exhaustive]
pub struct LicenseError {
    /// The feature, as Elasticsearch names it, such as `Reciprocal Rank
    /// Fusion (RRF)`.
    pub feature: String,
    /// The error response.
    pub error: ElasticsearchError,
}

impl fmt::Display for LicenseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the cluster's license doesn't include {}", self.feature)
    }
}

impl Error {
    /// The error for a response Elasticsearch failed, from its status and
    /// body: [`Error::License`] when the cluster's license doesn't include a
    /// feature the request uses, [`Error::Conflict`] for a version
    /// conflict, else [`Error::Elasticsearch`].
    pub fn from_response(status: u16, body: &[u8]) -> Error {
        let body: Value = serde_json::from_slice(body)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(body).into_owned()));
        let response = serde_json::from_value(body.clone()).ok();
        let error = ElasticsearchError {
            status,
            response,
            body,
        };
        // Elasticsearch names the feature a license doesn't include in the
        // root cause's metadata.
        let feature = error
            .body
            .pointer("/error/root_cause")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .chain(error.body.get("error"))
            .find_map(|cause| cause.get("license.expired.feature"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| error.reason().and_then(unlicensed_feature));
        match feature {
            Some(feature) => Error::License(Box::new(LicenseError { feature, error })),
            None if status == 409 => Error::Conflict(Box::new(error)),
            None => Error::Elasticsearch(Box::new(error)),
        }
    }
}

/// The feature in a license refusal's reason, for refusals without the
/// metadata that names it: "Current license is non-compliant for query
/// rules. Current license is …" names `query rules`.
fn unlicensed_feature(reason: &str) -> Option<String> {
    const MARKER: &str = "license is non-compliant for ";
    let start = reason.to_ascii_lowercase().find(MARKER)? + MARKER.len();
    let rest = &reason[start..];
    let feature = match rest.strip_prefix('[') {
        Some(bracketed) => &bracketed[..bracketed.find(']')?],
        None => rest.split(['.', ';']).next()?,
    };
    let feature = feature.trim();
    (!feature.is_empty()).then(|| feature.to_owned())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn error(body: Value) -> Error {
        Error::from_response(403, body.to_string().as_bytes())
    }

    #[test]
    fn license_errors_by_metadata() {
        let body = json!({"error": {
            "root_cause": [{"type": "security_exception", "reason": "current license is non-compliant for [Reciprocal Rank Fusion (RRF)]", "license.expired.feature": "Reciprocal Rank Fusion (RRF)"}],
            "type": "security_exception",
            "reason": "current license is non-compliant for [Reciprocal Rank Fusion (RRF)]"
        }, "status": 403});
        let Error::License(e) = error(body) else {
            panic!("not a license error")
        };
        assert_eq!(e.feature, "Reciprocal Rank Fusion (RRF)");
    }

    #[test]
    fn license_errors_by_reason() {
        let body = json!({"error": {
            "type": "security_exception",
            "reason": "Current license is non-compliant for query rules. Current license is active basic license."
        }, "status": 403});
        let Error::License(e) = error(body) else {
            panic!("not a license error")
        };
        assert_eq!(e.feature, "query rules");
    }

    #[test]
    fn conflicts() {
        let body = json!({"error": {"type": "version_conflict_engine_exception", "reason": "[1]: version conflict, required seqNo [0], primary term [1]. current document has seqNo [1] and primary term [1]"}, "status": 409});
        let Error::Conflict(e) = Error::from_response(409, body.to_string().as_bytes()) else {
            panic!("not a conflict")
        };
        assert_eq!(e.error_type(), Some("version_conflict_engine_exception"));
    }

    #[test]
    fn other_errors() {
        let body = json!({"error": {"type": "index_not_found_exception", "reason": "no such index [x]"}, "status": 404});
        let Error::Elasticsearch(e) = Error::from_response(404, body.to_string().as_bytes()) else {
            panic!("not an Elasticsearch error")
        };
        assert_eq!(e.error_type(), Some("index_not_found_exception"));
        assert!(e.response.is_some());
    }
}
