//! The engine's remote config (Remote updates, in the website repo's
//! `CLAUDE.md`): which release of each package the engine uses. It's
//! `remote-config/logix-engine/config-NNN.jsonc` in this repo, read from
//! GitHub by the config version this engine hard-codes, and cached in the
//! engine folder. There's no built-in list: the repo's file is the pin list.
//!
//! A config names, per package, a GitHub repo, a release tag and a file
//! template (`${PLATFORM}`, `${YYYYMMDD}`). Resolving it for this platform
//! takes each file's SHA-256 from its release's `SHA256SUMS`, which gives the
//! package set the package manager downloads and checks.
//!
//! Cache: `remote-config/config.json` in the engine folder, the resolved set
//! with where it came from and when. It's used for 24 hours, and offline.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::manifest::{self, Manifest, PackageFile, PackageSpec};

/// The repo whose `remote-config/` holds the engine's config.
pub const CONFIG_REPO: &str = "openlogix/logix-engine";
/// The engine's config version: which `config-NNN.jsonc` it reads. Raise it
/// only when a new engine can't share the current file (a package added or
/// dropped, or a package release older engines can't use).
pub const CONFIG_VERSION: &str = "001";
/// The product folder in `remote-config/`.
const PRODUCT: &str = "logix-engine";
/// Where raw repo files are read from.
pub const RAW_BASE: &str = "https://raw.githubusercontent.com";
/// The diagnostic override: a URL, or a local file, read instead of the
/// regular config (as Apio's). A bad value is an error, never a fallback.
pub const DEV_REMOTE_CONFIG_VAR: &str = "LOGIX_ENGINE_DEV_REMOTE_CONFIG";
/// How long a fetched config is used before it's fetched again.
pub const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
/// The schema every remote config names.
pub const SCHEMA: &str = "https://www.openlogix.org/schemas/logix-remote-config-v1.json";
/// The packages every config of this config version must name, and no others.
pub const PACKAGES: [&str; 2] = [super::CATALOG, super::CORE];

/// The config's path in the repo.
pub fn repo_path() -> String {
    format!("remote-config/{PRODUCT}/config-{CONFIG_VERSION}.jsonc")
}

/// The regular config's address: the raw file on `main`.
pub fn default_url() -> String {
    format!("{RAW_BASE}/{CONFIG_REPO}/main/{}", repo_path())
}

/// The repo's own config file, as an override source: what the tests use,
/// so they run with the releases the repo names.
#[doc(hidden)]
pub fn repo_file_source() -> ConfigSource {
    ConfigSource {
        location: Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(repo_path())
            .display()
            .to_string(),
        is_override: true,
    }
}

/// The release the repo's config file names for a package (for tests).
#[doc(hidden)]
pub fn repo_release(name: &str) -> String {
    let text = std::fs::read_to_string(repo_file_source().location).expect("the repo's config");
    parse(&text).expect("a valid config")[name].release.clone()
}

/// Where the engine reads its remote config.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigSource {
    /// An http(s) URL or a local file path.
    pub location: String,
    /// True for the diagnostic override.
    pub is_override: bool,
}

impl ConfigSource {
    pub fn regular() -> Self {
        ConfigSource {
            location: default_url(),
            is_override: false,
        }
    }

    /// The override from `LOGIX_ENGINE_DEV_REMOTE_CONFIG`, if set.
    pub fn from_env() -> Option<Self> {
        std::env::var(DEV_REMOTE_CONFIG_VAR)
            .ok()
            .filter(|v| !v.is_empty())
            .map(|location| ConfigSource {
                location,
                is_override: true,
            })
    }

    fn is_url(&self) -> bool {
        self.location.starts_with("https://") || self.location.starts_with("http://")
    }
}

/// One dependency in a config.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub owner: String,
    pub repo: String,
    pub release: String,
    pub file: String,
}

/// JSONC to JSON: removes `//` and `/* */` comments outside strings.
pub fn strip_comments(text: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if c == '/' && chars.peek() == Some(&'/') {
            for n in chars.by_ref() {
                if n == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut prev = ' ';
            let mut closed = false;
            for n in chars.by_ref() {
                if n == '\n' {
                    out.push('\n');
                }
                if prev == '*' && n == '/' {
                    closed = true;
                    break;
                }
                prev = n;
            }
            if !closed {
                return Err("a /* comment isn't closed".into());
            }
        } else {
            out.push(c);
        }
    }
    Ok(out)
}

/// Reads a config's text: its dependencies, by package name. Checks what
/// the schema (logix-remote-config-v1) does that matters here, and that it
/// names exactly the packages this engine needs.
pub fn parse(text: &str) -> Result<BTreeMap<String, Entry>, String> {
    let json: serde_json::Value = serde_json::from_str(&strip_comments(text)?)
        .map_err(|e| format!("not valid JSONC: {e}"))?;
    let obj = json.as_object().ok_or("not a JSON object")?;
    if obj.get("$schema").and_then(|v| v.as_str()) != Some(SCHEMA) {
        return Err(format!("its $schema must be {SCHEMA}"));
    }
    let mut out = BTreeMap::new();
    for (name, dep) in obj {
        if name == "$schema" {
            continue;
        }
        let field = |key: &str| -> Result<String, String> {
            dep.get(key)
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .ok_or_else(|| format!("{name}: no {key}"))
        };
        if let Some(extra) = dep
            .as_object()
            .ok_or_else(|| format!("{name}: not an object"))?
            .keys()
            .find(|k| !["repo", "release", "file"].contains(&k.as_str()))
        {
            return Err(format!("{name}: unknown key {extra}"));
        }
        let repo = field("repo")?;
        let (owner, repo) = repo
            .split_once('/')
            .filter(|(o, r)| safe(o) && safe(r))
            .ok_or_else(|| format!("{name}: repo must be owner/name, not {repo:?}"))?;
        let release = field("release")?;
        if !manifest::is_date_tag(&release) {
            return Err(format!(
                "{name}: release must be yyyy-mm-dd, not {release:?}"
            ));
        }
        let file = field("file")?;
        let plain = file.replace("${PLATFORM}", "x").replace("${YYYYMMDD}", "x");
        if !safe(&plain) {
            return Err(format!("{name}: bad file {file:?}"));
        }
        out.insert(
            name.clone(),
            Entry {
                owner: owner.into(),
                repo: repo.into(),
                release,
                file,
            },
        );
    }
    let names: Vec<&str> = out.keys().map(String::as_str).collect();
    let mut wanted = PACKAGES.to_vec();
    wanted.sort();
    if names != wanted {
        return Err(format!(
            "it must name exactly {}, not {}",
            wanted.join(", "),
            names.join(", ")
        ));
    }
    Ok(out)
}

/// A single path component made of safe characters.
fn safe(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('.')
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
}

/// An HTTP client for the config and its checksums: short timeouts, since
/// these are small files read before an operation.
pub fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .user_agent(format!("logix-engine/{}", crate::state::ENGINE_VERSION))
        .build()
        .map_err(|e| e.to_string())
}

async fn get_text(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("{url}: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("{url}: HTTP {status}"));
    }
    response.text().await.map_err(|e| format!("{url}: {e}"))
}

/// Reads a config's text from its source.
pub async fn read(source: &ConfigSource, client: &reqwest::Client) -> Result<String, String> {
    if source.is_url() {
        get_text(client, &source.location).await
    } else {
        std::fs::read_to_string(&source.location).map_err(|e| format!("{}: {e}", source.location))
    }
}

/// Resolves a config for a platform: each package's file, with its SHA-256
/// from its release's `SHA256SUMS`, under `release_base` (manifest::GITHUB,
/// or a test server).
pub async fn resolve(
    entries: &BTreeMap<String, Entry>,
    platform: &str,
    release_base: &str,
    client: &reqwest::Client,
) -> Result<Manifest, String> {
    let mut packages = Vec::new();
    // The catalog first: it's small, so boards can be listed while the
    // tools download.
    for name in PACKAGES {
        let e = &entries[name];
        let per_platform = e.file.contains("${PLATFORM}");
        let file = e
            .file
            .replace("${PLATFORM}", platform)
            .replace("${YYYYMMDD}", &e.release.replace('-', ""));
        let sums_url = format!(
            "{}/{}/{}/releases/download/{}/SHA256SUMS",
            release_base.trim_end_matches('/'),
            e.owner,
            e.repo,
            e.release
        );
        let sums = get_text(client, &sums_url).await?;
        let sha256 = sums
            .lines()
            .filter_map(|l| l.split_once(char::is_whitespace))
            .find(|(_, f)| f.trim().trim_start_matches('*') == file)
            .map(|(sha, _)| sha.to_lowercase())
            .ok_or_else(|| format!("{sums_url} doesn't list {file}"))?;
        let key = if per_platform {
            platform.to_string()
        } else {
            super::ANY_PLATFORM.to_string()
        };
        packages.push(PackageSpec {
            name: name.to_string(),
            version: e.release.clone(),
            files: BTreeMap::from([(
                key,
                PackageFile {
                    owner: e.owner.clone(),
                    repo: e.repo.clone(),
                    tag: e.release.clone(),
                    file,
                    size: None,
                    sha256,
                },
            )]),
        });
    }
    let set = Manifest { packages };
    set.validate()?;
    Ok(set)
}

/// The cached, resolved config.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cached {
    /// Where it was read: a URL or a local file.
    pub source: String,
    pub config_version: String,
    /// When it was read, in seconds since 1970 (UTC).
    pub fetched: u64,
    pub set: Manifest,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Cached {
    /// Whether it's from this source and config version.
    pub fn matches(&self, source: &ConfigSource) -> bool {
        self.source == source.location && self.config_version == CONFIG_VERSION
    }

    /// Whether it's older than MAX_AGE.
    pub fn is_due(&self) -> bool {
        now().saturating_sub(self.fetched) >= MAX_AGE.as_secs()
    }
}

fn cache_file(engine_folder: &Path) -> PathBuf {
    engine_folder.join("remote-config").join("config.json")
}

/// The cached config, if there is a readable one.
pub fn load_cache(engine_folder: &Path) -> Option<Cached> {
    let text = std::fs::read_to_string(cache_file(engine_folder)).ok()?;
    let cached: Cached = serde_json::from_str(&text).ok()?;
    cached.set.validate().ok()?;
    Some(cached)
}

/// Writes the cache, replacing the old one in one step.
pub fn save_cache(engine_folder: &Path, cached: &Cached) -> Result<(), String> {
    let file = cache_file(engine_folder);
    let dir = file.parent().expect("the cache file has a folder");
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!(".config-{}.json", std::process::id()));
    let json = serde_json::to_string_pretty(cached).map_err(|e| e.to_string())? + "\n";
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &file).map_err(|e| e.to_string())
}

/// The UTC time of a `fetched` value, as `yyyy-mm-ddThh:mm:ssZ`.
pub fn utc_time(secs: u64) -> String {
    let t = time::OffsetDateTime::from_unix_timestamp(secs as i64)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    t.format(time::macros::format_description!(
        "[year]-[month]-[day]T[hour]:[minute]:[second]Z"
    ))
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"// What Logix Engine on config 001 uses.
{
  "$schema": "https://www.openlogix.org/schemas/logix-remote-config-v1.json",

  /* The FPGA tools. */
  "logix-core": {
    "repo": "openlogix/logix-core",
    "release": "2026-11-11",
    "file": "logix-core-${PLATFORM}-${YYYYMMDD}.tgz"
  },

  // A "//" in a string isn't a comment: "a//b".
  "logix-catalog": {
    "repo": "openlogix/logix-catalog",
    "release": "2026-11-11",
    "file": "logix-catalog-${YYYYMMDD}.tgz"
  }
}
"#;

    #[test]
    fn a_config_reads_with_its_comments() {
        let e = parse(CONFIG).unwrap();
        assert_eq!(
            e["logix-core"],
            Entry {
                owner: "openlogix".into(),
                repo: "logix-core".into(),
                release: "2026-11-11".into(),
                file: "logix-core-${PLATFORM}-${YYYYMMDD}.tgz".into(),
            }
        );
        assert_eq!(
            strip_comments(r#"{"a": "x//y" /* c */}"#).unwrap(),
            r#"{"a": "x//y" }"#
        );
    }

    #[test]
    fn bad_configs_are_refused() {
        for (from, to, why) in [
            (
                "2026-11-11\",\n    \"file\": \"logix-core",
                "latest\",\n    \"file\": \"logix-core",
                "release",
            ),
            ("openlogix/logix-core", "logix-core", "owner/name"),
            ("logix-catalog-${YYYYMMDD}.tgz", "../evil.tgz", "bad file"),
            ("\"logix-catalog\": {", "\"logix-other\": {", "exactly"),
            (
                "logix-remote-config-v1",
                "logix-remote-config-v9",
                "$schema",
            ),
            (
                "\"repo\": \"openlogix/logix-core\",",
                "\"repo\": \"openlogix/logix-core\", \"x\": 1,",
                "unknown key",
            ),
        ] {
            assert!(CONFIG.contains(from), "{from}");
            let err = parse(&CONFIG.replacen(from, to, 1)).unwrap_err();
            assert!(err.contains(why), "{why}: {err}");
        }
        assert!(parse("{ /* open").is_err());
    }

    #[test]
    fn the_repo_config_is_valid() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(repo_path());
        parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            default_url(),
            "https://raw.githubusercontent.com/openlogix/logix-engine/main/remote-config/logix-engine/config-001.jsonc"
        );
    }

    #[test]
    fn times_are_utc() {
        assert_eq!(utc_time(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_time(1_794_441_600), "2026-11-12T00:00:00Z");
    }
}
