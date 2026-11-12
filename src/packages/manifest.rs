//! The package set: the release, file and checksum of every package the
//! engine uses on this platform. It's resolved from the engine's remote
//! config (`remote_config.rs`): there's no built-in list (docs/requirements.md,
//! "Versioning" and "Packages").

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Where package files are downloaded from: GitHub releases.
pub const GITHUB: &str = "https://github.com";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub packages: Vec<PackageSpec>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageSpec {
    /// The package name, logix-core or logix-catalog.
    pub name: String,
    /// The package release, a date tag.
    pub version: String,
    /// The package file for each platform, such as darwin-arm64, or for
    /// "any" platform.
    pub files: BTreeMap<String, PackageFile>,
}

/// One package file, a GitHub release asset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageFile {
    pub owner: String,
    pub repo: String,
    pub tag: String,
    /// The asset file name, a .tgz.
    pub file: String,
    /// The file size in bytes, when known; otherwise the download's own
    /// length is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// The SHA-256 checksum of the file, as lowercase hex.
    pub sha256: String,
}

impl PackageFile {
    /// The download URL, under `base` (GITHUB, or a test server).
    pub fn url(&self, base: &str) -> String {
        format!(
            "{}/{}/{}/releases/download/{}/{}",
            base.trim_end_matches('/'),
            self.owner,
            self.repo,
            self.tag,
            self.file
        )
    }
}

impl Manifest {
    pub fn parse(json: &str) -> Result<Manifest, String> {
        let manifest: Manifest = serde_json::from_str(json).map_err(|e| e.to_string())?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn get(&self, name: &str) -> Option<&PackageSpec> {
        self.packages.iter().find(|p| p.name == name)
    }

    /// Checks the rules that keep package paths and versions safe.
    pub fn validate(&self) -> Result<(), String> {
        let mut names = std::collections::BTreeSet::new();
        for p in &self.packages {
            if !is_id(&p.name) {
                return Err(format!("bad package name {:?}", p.name));
            }
            if !names.insert(&p.name) {
                return Err(format!("package {} is listed twice", p.name));
            }
            if !is_date_tag(&p.version) {
                return Err(format!("{}: bad version {:?}", p.name, p.version));
            }
            for (platform, f) in &p.files {
                let ctx = format!("{} ({platform})", p.name);
                if !is_id(platform) {
                    return Err(format!("{ctx}: bad platform name"));
                }
                if f.sha256.len() != 64
                    || !f
                        .sha256
                        .bytes()
                        .all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
                {
                    return Err(format!("{ctx}: sha256 must be 64 lowercase hex digits"));
                }
                if !f.file.ends_with(".tgz") || !is_safe_name(&f.file) {
                    return Err(format!("{ctx}: bad file name {:?}", f.file));
                }
                for (what, v) in [("owner", &f.owner), ("repo", &f.repo), ("tag", &f.tag)] {
                    if !is_safe_name(v) {
                        return Err(format!("{ctx}: bad {what} {v:?}"));
                    }
                }
                if f.size == Some(0) {
                    return Err(format!("{ctx}: size can't be 0"));
                }
            }
        }
        Ok(())
    }
}

/// Lowercase letters, digits and hyphens, like a catalog ID.
fn is_id(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

/// A single path component made of safe characters.
fn is_safe_name(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('.')
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
}

/// A release tag, `yyyy-mm-dd` (at most one release a day).
pub fn is_date_tag(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    digits(0..4) && digits(5..7) && digits(8..10)
}

/// The version people see for a release tag: `yyyy.mm.dd` for `yyyy-mm-dd`
/// (the Open Logix versioning rule). Anything else is returned as is.
pub fn shown_version(tag: &str) -> String {
    if is_date_tag(tag) {
        tag.replace('-', ".")
    } else {
        tag.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SET: &str = r#"{
  "packages": [
    {
      "name": "logix-catalog",
      "version": "2026-11-11",
      "files": {
        "any": {
          "owner": "openlogix",
          "repo": "logix-catalog",
          "tag": "2026-11-11",
          "file": "logix-catalog-20261111.tgz",
          "sha256": "431d4a9eefdea9d0baa84bc7f26d0556b75a2cca83ca6ce45a97ad79bb08d05a"
        }
      }
    }
  ]
}"#;

    #[test]
    fn a_set_reads_and_gives_its_urls() {
        let m = Manifest::parse(SET).unwrap();
        let f = &m.get("logix-catalog").unwrap().files["any"];
        assert_eq!(
            f.url(GITHUB),
            "https://github.com/openlogix/logix-catalog/releases/download/2026-11-11/logix-catalog-20261111.tgz"
        );
        assert_eq!(f.size, None);
    }

    #[test]
    fn date_tags() {
        assert!(is_date_tag("2026-10-22"));
        assert!(!is_date_tag("2026-10-22-2"));
        assert_eq!(shown_version("2026-10-22"), "2026.10.22");
        assert_eq!(shown_version("local"), "local");
        assert!(!is_date_tag("2026.10.22"));
        assert!(!is_date_tag("20261022"));
        assert!(!is_date_tag("2026-10-2x"));
    }

    #[test]
    fn unsafe_values_are_refused() {
        for (from, to) in [
            (
                "\"logix-catalog\",\n      \"version\"",
                "\"../x\",\n      \"version\"",
            ),
            ("\"version\": \"2026-11-11\"", "\"version\": \"latest\""),
            (
                "\"file\": \"logix-catalog-20261111.tgz\"",
                "\"file\": \"../evil.tgz\"",
            ),
            ("\"owner\": \"openlogix\"", "\"owner\": \"a/b\""),
            ("431d4a9eef", "431D4A9EEF"),
        ] {
            assert!(SET.contains(from), "{from}");
            let bad = SET.replacen(from, to, 1);
            assert!(Manifest::parse(&bad).is_err(), "{to} should be refused");
        }
    }
}
