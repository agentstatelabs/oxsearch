//! What a run did.

use std::fmt;

use oxsearch::helpers::Outcome;

/// The refusals a report keeps, beyond its counts.
const KEPT: usize = 10;

/// What an export, import or transfer did.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Report {
    /// Documents read.
    pub read: u64,
    /// Documents the sample passed over.
    pub skipped: u64,
    /// Documents written: to the file, or accepted by Elasticsearch.
    pub written: u64,
    /// Documents Elasticsearch refused.
    pub failed: u64,
    /// The first refusals.
    pub failures: Vec<Failure>,
}

/// A document Elasticsearch refused.
#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    /// Its index.
    pub index: Option<String>,
    /// Its id.
    pub id: Option<String>,
    /// The refusal's status code.
    pub status: u16,
    /// Why.
    pub reason: String,
}

impl Report {
    /// Whether every document read was written or deliberately passed over.
    pub fn is_ok(&self) -> bool {
        self.failed == 0
    }

    /// Counts a bulk action's outcome.
    pub(crate) fn outcome(&mut self, outcome: Outcome, operation: &'static str) {
        if outcome.is_ok() {
            self.written += 1;
            metrics::counter!("oxsearch.cli.documents", "operation" => operation, "result" => "written")
                .increment(1);
            return;
        }
        self.failed += 1;
        metrics::counter!("oxsearch.cli.documents", "operation" => operation, "result" => "failed")
            .increment(1);
        if self.failures.len() < KEPT {
            let reason = outcome.error.as_ref().map_or_else(
                || format!("status {}", outcome.status),
                |e| {
                    let kind = e.r#type.as_deref().unwrap_or("error");
                    match &e.reason {
                        Some(reason) => format!("{kind}: {reason}"),
                        None => kind.to_owned(),
                    }
                },
            );
            self.failures.push(Failure {
                index: outcome.index,
                id: outcome.id,
                status: outcome.status,
                reason,
            });
        }
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} read, {} skipped, {} written, {} refused",
            self.read, self.skipped, self.written, self.failed
        )?;
        for failure in &self.failures {
            write!(
                f,
                "\n  refused {}/{} ({}): {}",
                failure.index.as_deref().unwrap_or("?"),
                failure.id.as_deref().unwrap_or("?"),
                failure.status,
                failure.reason
            )?;
        }
        if self.failed > self.failures.len() as u64 {
            write!(
                f,
                "\n  and {} more",
                self.failed - self.failures.len() as u64
            )?;
        }
        Ok(())
    }
}
