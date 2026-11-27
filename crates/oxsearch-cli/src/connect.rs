//! Clients for the clusters, with credentials from the environment.

use oxsearch::Client;
use oxsearch::auth::Auth;
use oxsearch::transport_reqwest::ReqwestTransport;

use crate::Error;

/// A client for the cluster at `url`, with credentials from the environment
/// variables that start with `prefix`: `{prefix}API_KEY` (encoded, as
/// Kibana shows it), or `{prefix}USERNAME` and `{prefix}PASSWORD`. They're
/// read from the environment, not arguments, so they don't show in process
/// listings or shell history.
pub fn connect(url: &str, prefix: &str) -> Result<Client<ReqwestTransport>, Error> {
    let var = |name: &str| {
        std::env::var(format!("{prefix}{name}"))
            .ok()
            .filter(|v| !v.is_empty())
    };
    let mut builder = ReqwestTransport::builder().node(url);
    match (var("API_KEY"), var("USERNAME"), var("PASSWORD")) {
        (Some(key), None, None) => builder = builder.auth(Auth::api_key(key)),
        (None, Some(user), Some(password)) => builder = builder.auth(Auth::basic(user, password)),
        (None, None, None) => {}
        (Some(_), _, _) => {
            return Err(Error::Usage(format!(
                "set {prefix}API_KEY or {prefix}USERNAME and {prefix}PASSWORD, not both"
            )));
        }
        _ => {
            return Err(Error::Usage(format!(
                "set both {prefix}USERNAME and {prefix}PASSWORD"
            )));
        }
    }
    let transport = builder
        .build()
        .map_err(|e| Error::Usage(format!("{}: {e}", redacted(url))))?;
    Ok(Client::new(transport))
}

/// A URL with any user and password left out, for logs.
pub fn redacted(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    let authority_end = rest.find('/').unwrap_or(rest.len());
    match rest[..authority_end].rfind('@') {
        Some(at) => format!("{scheme}://{}", &rest[at + 1..]),
        None => url.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_stay_out_of_logs() {
        assert_eq!(
            redacted("https://elastic:secret@es.example:9200/x"),
            "https://es.example:9200/x"
        );
        assert_eq!(redacted("http://localhost:9200"), "http://localhost:9200");
        assert_eq!(
            redacted("http://localhost:9200/a@b"),
            "http://localhost:9200/a@b"
        );
    }
}
