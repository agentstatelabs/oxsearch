//! Why a run stopped.

use std::fmt;
use std::path::PathBuf;

/// Why an export, import or transfer stopped before its end.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The arguments don't make a run.
    Usage(String),
    /// A file couldn't be opened, read or written.
    Io {
        /// The file, or `-` for standard input or output.
        path: PathBuf,
        /// What went wrong.
        source: std::io::Error,
    },
    /// A line isn't a document or a record.
    Line {
        /// The line's number, from 1.
        line: u64,
        /// What's wrong with it.
        message: String,
    },
    /// Elasticsearch refused a request, or couldn't be reached.
    Elasticsearch(oxsearch::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Usage(message) => f.write_str(message),
            Error::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Error::Line { line, message } => write!(f, "line {line}: {message}"),
            Error::Elasticsearch(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            Error::Elasticsearch(error) => Some(error),
            Error::Usage(_) | Error::Line { .. } => None,
        }
    }
}

impl From<oxsearch::Error> for Error {
    fn from(error: oxsearch::Error) -> Error {
        Error::Elasticsearch(error)
    }
}
