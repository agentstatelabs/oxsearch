//! The Elasticsearch specification the generator reads: the commit it is
//! pinned to, and downloading, verifying and reading that commit's
//! `schema.json`.

pub mod model;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use sha2::{Digest, Sha256};

pub use model::Schema;
use model::{Body, Stability, TypeDefinition, Variants};

use crate::{Error, workspace_root};

/// The pin, relative to the workspace root.
pub const PIN_FILE: &str = "spec/pin.toml";
/// The schema's path in the specification repository.
pub const SCHEMA_PATH: &str = "output/schema/schema.json";
/// The test fixture, relative to the workspace root. It must run the
/// pinned Elasticsearch minor.
pub const FIXTURE_FILE: &str = "integration/compose.yaml";
/// The specification repository, when no pin names one yet.
const DEFAULT_REPOSITORY: &str = "elastic/elasticsearch-specification";
/// The license the generator accepts input under: generated code must be
/// generated from Apache-2.0 sources only.
const LICENSE: &str = "Apache 2.0";
/// Larger than any `schema.json` so far (11 MB for 9.5).
const MAX_SCHEMA_BYTES: u64 = 128 * 1024 * 1024;

/// The specification commit oxsearch is generated from, on the branch for
/// one Elasticsearch minor.
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    /// The specification's GitHub repository, as `owner/name`.
    pub repository: String,
    /// The branch, named after an Elasticsearch minor, such as `9.5`.
    pub branch: String,
    /// The commit, in full.
    pub commit: String,
    /// The SHA-256 of `output/schema/schema.json` at that commit, in hex.
    pub schema_sha256: String,
}

impl Pin {
    /// Reads and checks `spec/pin.toml`.
    pub fn read() -> Result<Pin, Error> {
        let path = workspace_root().join(PIN_FILE);
        let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Pin::parse(&text)
    }

    /// Parses and checks a pin.
    pub fn parse(text: &str) -> Result<Pin, Error> {
        let pin: Pin = toml::from_str(text).map_err(|e| format!("{PIN_FILE}: {e}"))?;
        if !is_hex(&pin.commit, 40) {
            return Err(format!(
                "{PIN_FILE}: commit {:?} isn't a full commit hash",
                pin.commit
            )
            .into());
        }
        if !is_hex(&pin.schema_sha256, 64) {
            return Err(format!(
                "{PIN_FILE}: schema_sha256 {:?} isn't a SHA-256 in hex",
                pin.schema_sha256
            )
            .into());
        }
        if minor(&pin.branch) != Some(pin.branch.as_str()) {
            return Err(format!(
                "{PIN_FILE}: branch {:?} isn't an Elasticsearch minor such as 9.5",
                pin.branch
            )
            .into());
        }
        Ok(pin)
    }

    /// The pin as `spec/pin.toml` holds it.
    pub fn render(&self) -> String {
        format!(
            "# The elasticsearch-specification commit oxsearch is generated from, on the\n\
             # branch for one Elasticsearch minor. Move it with\n\
             # `cargo run -p xtask -- spec pin <branch> [<commit>]`, which records the\n\
             # checksum; `cargo run -p xtask -- spec check` verifies it.\n\
             repository = \"{}\"\n\
             branch = \"{}\"\n\
             commit = \"{}\"\n\
             # The SHA-256 of {SCHEMA_PATH} at that commit.\n\
             schema_sha256 = \"{}\"\n",
            self.repository, self.branch, self.commit, self.schema_sha256
        )
    }

    /// Where the pinned schema is downloaded from.
    pub fn schema_url(&self) -> String {
        format!(
            "https://raw.githubusercontent.com/{}/{}/{SCHEMA_PATH}",
            self.repository, self.commit
        )
    }

    /// Where the downloaded schema is kept, under the target directory.
    pub fn cache_path(&self) -> PathBuf {
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| workspace_root().join("target"));
        target
            .join("elasticsearch-specification")
            .join(&self.commit)
            .join("schema.json")
    }
}

/// The pinned `schema.json`, verified against the pin's checksum: the copy
/// under the target directory if it matches, a fresh download otherwise.
pub fn fetch(pin: &Pin) -> Result<Vec<u8>, Error> {
    let path = pin.cache_path();
    if let Ok(bytes) = fs::read(&path) {
        if sha256_hex(&bytes) == pin.schema_sha256 {
            return Ok(bytes);
        }
        eprintln!(
            "{} doesn't match the pinned checksum; downloading it again",
            path.display()
        );
    }
    let url = pin.schema_url();
    let bytes = download(&url)?;
    let actual = sha256_hex(&bytes);
    if actual != pin.schema_sha256 {
        return Err(format!(
            "{url} has SHA-256 {actual}, but {PIN_FILE} expects {}",
            pin.schema_sha256
        )
        .into());
    }
    store(&path, &bytes)?;
    Ok(bytes)
}

/// Reads a `schema.json` with the model, and checks it is published under
/// the license the generator accepts.
pub fn load(bytes: &[u8]) -> Result<Schema, Error> {
    let deserializer = &mut serde_json::Deserializer::from_slice(bytes);
    let schema: Schema = serde_path_to_error::deserialize(deserializer).map_err(|e| {
        format!(
            "schema.json doesn't fit the model at {}: {}",
            e.path(),
            e.inner()
        )
    })?;
    if schema.info.license.name != LICENSE {
        return Err(format!(
            "schema.json is published under {:?}; oxsearch generates only from {LICENSE} sources",
            schema.info.license.name
        )
        .into());
    }
    Ok(schema)
}

/// Moves the pin to `commit`, or to the head of `branch`, after checking the
/// model reads that commit's schema. Returns the new pin.
pub fn repin(branch: &str, commit: Option<&str>) -> Result<Pin, Error> {
    let repository = Pin::read()
        .map(|pin| pin.repository)
        .unwrap_or_else(|_| DEFAULT_REPOSITORY.to_owned());
    let commit = match commit {
        Some(commit) => commit.to_owned(),
        None => branch_head(&repository, branch)?,
    };
    let mut pin = Pin {
        repository,
        branch: branch.to_owned(),
        commit,
        schema_sha256: String::new(),
    };
    let bytes = download(&pin.schema_url())?;
    load(&bytes)?;
    pin.schema_sha256 = sha256_hex(&bytes);
    let pin = Pin::parse(&pin.render())?;
    store(&pin.cache_path(), &bytes)?;
    fs::write(workspace_root().join(PIN_FILE), pin.render())?;
    Ok(pin)
}

/// Checks the test fixture runs the pinned Elasticsearch minor, and returns
/// its version.
pub fn check_fixture(pin: &Pin) -> Result<String, Error> {
    let path = workspace_root().join(FIXTURE_FILE);
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let version = fixture_version(&text).ok_or_else(|| {
        format!("{FIXTURE_FILE}: no elasticsearch:${{ELASTICSEARCH_VERSION:-…}} image")
    })?;
    if minor(version) != Some(pin.branch.as_str()) {
        return Err(format!(
            "{FIXTURE_FILE} runs Elasticsearch {version}, but {PIN_FILE} pins the {} specification",
            pin.branch
        )
        .into());
    }
    Ok(version.to_owned())
}

/// The default Elasticsearch version in the fixture's image reference.
fn fixture_version(compose: &str) -> Option<&str> {
    const MARKER: &str = "elasticsearch:${ELASTICSEARCH_VERSION:-";
    let start = compose.find(MARKER)? + MARKER.len();
    let len = compose[start..].find('}')?;
    Some(&compose[start..start + len])
}

/// A summary of what the specification holds.
pub fn summary(schema: &Schema) -> String {
    let mut kinds = BTreeMap::<&str, usize>::new();
    let mut variants = BTreeMap::<&str, usize>::new();
    let mut body_kinds = BTreeMap::<&str, usize>::new();
    for definition in &schema.types {
        *kinds.entry(definition.kind()).or_default() += 1;
        let (found, body) = match definition {
            TypeDefinition::Interface(t) => (t.variants.as_ref(), None),
            TypeDefinition::TypeAlias(t) => (t.variants.as_ref(), None),
            TypeDefinition::Request(t) => (None, Some(&t.body)),
            _ => (None, None),
        };
        if let Some(found) = found {
            let kind = match found {
                Variants::Container { .. } => "container",
                Variants::InternalTag { .. } => "internally tagged",
                Variants::ExternalTag { .. } => "externally tagged",
                Variants::Untagged { .. } => "untagged",
            };
            *variants.entry(kind).or_default() += 1;
        }
        if let Some(body) = body {
            let kind = match body {
                Body::NoBody {} => "no body",
                Body::Properties { .. } => "properties",
                Body::Value { .. } => "a value",
            };
            *body_kinds.entry(kind).or_default() += 1;
        }
    }
    let mut stability = BTreeMap::<&str, usize>::new();
    let mut excluded = 0;
    for endpoint in &schema.endpoints {
        if endpoint.codegen_exclude {
            excluded += 1;
        }
        let level = match endpoint.availability.stack.as_ref() {
            None => "not on the stack",
            Some(stack) => match stack.stability {
                None | Some(Stability::Stable) => "stable",
                Some(Stability::Beta) => "beta",
                Some(Stability::Experimental) => "experimental",
                Some(Stability::TechPreview) => "tech preview",
            },
        };
        *stability.entry(level).or_default() += 1;
    }

    let mut out = String::new();
    let _ = writeln!(
        out,
        "{} endpoints ({}), {excluded} excluded from code generation",
        schema.endpoints.len(),
        list(&stability)
    );
    let _ = writeln!(out, "{} types ({})", schema.types.len(), list(&kinds));
    let _ = writeln!(out, "variants: {}", list(&variants));
    let _ = write!(out, "request bodies: {}", list(&body_kinds));
    out
}

fn list(counts: &BTreeMap<&str, usize>) -> String {
    let mut sorted: Vec<_> = counts.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    sorted
        .iter()
        .map(|(name, count)| format!("{count} {name}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The SHA-256 of `bytes`, in lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut hex, b| {
            let _ = write!(hex, "{b:02x}");
            hex
        })
}

/// `9.5` for `9.5`, `9.5.5` or `9.5.0-SNAPSHOT`.
fn minor(version: &str) -> Option<&str> {
    let mut parts = version.splitn(3, '.');
    let major = parts.next()?;
    let minor = parts.next()?;
    let minor = minor.split('-').next()?;
    let numeric = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !numeric(major) || !numeric(minor) {
        return None;
    }
    Some(&version[..major.len() + 1 + minor.len()])
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn download(url: &str) -> Result<Vec<u8>, Error> {
    eprintln!("downloading {url}");
    let mut response = ureq::get(url).call().map_err(|e| format!("{url}: {e}"))?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_SCHEMA_BYTES)
        .read_to_vec()
        .map_err(|e| format!("{url}: {e}"))?;
    Ok(bytes)
}

/// The commit at the head of a branch, from GitHub's API.
fn branch_head(repository: &str, branch: &str) -> Result<String, Error> {
    let url = format!("https://api.github.com/repos/{repository}/commits/{branch}");
    let sha = ureq::get(&url)
        .header("Accept", "application/vnd.github.sha")
        .call()
        .map_err(|e| format!("{url}: {e}"))?
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("{url}: {e}"))?;
    let sha = sha.trim();
    if !is_hex(sha, 40) {
        return Err(format!("{url} didn't return a commit hash: {sha:?}").into());
    }
    Ok(sha.to_owned())
}

/// Writes a file whole or not at all.
fn store(path: &std::path::Path, bytes: &[u8]) -> Result<(), Error> {
    let dir = path.parent().expect("the cache path has a parent");
    fs::create_dir_all(dir)?;
    let partial = path.with_extension("json.part");
    fs::write(&partial, bytes)?;
    fs::rename(&partial, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_committed_pin_is_valid() {
        let pin = Pin::read().unwrap();
        assert_eq!(Pin::parse(&pin.render()).unwrap(), pin);
    }

    #[test]
    fn a_pin_needs_a_full_commit_and_checksum() {
        let pin = |commit: &str, sha: &str, branch: &str| {
            Pin::parse(&format!(
                "repository = \"o/r\"\nbranch = \"{branch}\"\ncommit = \"{commit}\"\nschema_sha256 = \"{sha}\"\n"
            ))
        };
        let commit = "a".repeat(40);
        let sha = "b".repeat(64);
        assert!(pin(&commit, &sha, "9.5").is_ok());
        assert!(pin("abc1234", &sha, "9.5").is_err());
        assert!(pin(&commit, &"B".repeat(64), "9.5").is_err());
        assert!(pin(&commit, &sha, "main").is_err());
        assert!(pin(&commit, &sha, "9.5.1").is_err());
        assert!(
            Pin::parse(&format!(
                "repository = \"o/r\"\nbranch = \"9.5\"\ncommit = \"{commit}\"\nschema_sha256 = \"{sha}\"\nextra = 1\n"
            ))
            .is_err()
        );
    }

    #[test]
    fn the_fixture_runs_the_pinned_minor() {
        let pin = Pin::read().unwrap();
        check_fixture(&pin).unwrap();
    }

    #[test]
    fn reads_the_fixture_version() {
        let compose = "image: docker.elastic.co/elasticsearch/elasticsearch:${ELASTICSEARCH_VERSION:-9.5.5}\n";
        assert_eq!(fixture_version(compose), Some("9.5.5"));
        assert_eq!(fixture_version("image: elasticsearch:9.5.5"), None);
    }

    #[test]
    fn minors() {
        assert_eq!(minor("9.5"), Some("9.5"));
        assert_eq!(minor("9.5.5"), Some("9.5"));
        assert_eq!(minor("10.0.0-SNAPSHOT"), Some("10.0"));
        assert_eq!(minor("9"), None);
        assert_eq!(minor("main"), None);
        assert_eq!(minor("9.x"), None);
    }

    #[test]
    fn sha256_is_lowercase_hex() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn the_model_rejects_unknown_fields() {
        let schema = |extra: &str| {
            format!(
                r#"{{"_info": {{"title": "t", "license": {{"name": "Apache 2.0", "url": "u"}}}},
                    "endpoints": [],
                    "types": [{{"kind": "enum", "name": {{"namespace": "n", "name": "E"}},
                                "specLocation": "n.ts#L1-L2",
                                "members": [{{"name": "a"}}]{extra}}}]}}"#
            )
        };
        let loaded = load(schema("").as_bytes()).unwrap();
        assert_eq!(loaded.types[0].name().name, "E");
        let error = load(schema(r#", "colour": "red""#).as_bytes())
            .unwrap_err()
            .to_string();
        assert!(error.contains("colour"), "{error}");
    }

    #[test]
    fn the_model_reads_values() {
        let value: model::ValueOf = serde_json::from_str(
            r#"{"kind": "dictionary_of", "singleKey": true,
                "key": {"kind": "instance_of", "type": {"namespace": "_types", "name": "Field"}},
                "value": {"kind": "union_of", "items": [
                    {"kind": "literal_value", "value": 3},
                    {"kind": "array_of", "value": {"kind": "user_defined_value"}}]}}"#,
        )
        .unwrap();
        let model::ValueOf::DictionaryOf {
            single_key, value, ..
        } = value
        else {
            panic!("{value:?}")
        };
        assert!(single_key);
        assert!(matches!(*value, model::ValueOf::UnionOf { ref items } if items.len() == 2));

        let unknown = serde_json::from_str::<model::ValueOf>(
            r#"{"kind": "array_of", "value": {"kind": "user_defined_value"}, "nullable": true}"#,
        );
        assert!(unknown.is_err());
        let unknown_kind = serde_json::from_str::<model::ValueOf>(r#"{"kind": "tuple_of"}"#);
        assert!(unknown_kind.is_err());
    }

    #[test]
    fn only_apache_licensed_input() {
        let error = load(
            br#"{"_info": {"title": "t", "license": {"name": "Other", "url": "u"}},
                 "endpoints": [], "types": []}"#,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("Apache 2.0"), "{error}");
    }
}
