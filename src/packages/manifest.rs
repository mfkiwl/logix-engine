//! The package manifest (docs/requirements.md, "Versioning" and "Packages"):
//! the exact release and checksum of every package this engine release was
//! tested with, for each platform. It is built into the engine from
//! manifest.json.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

/// Where package files are downloaded from: GitHub releases.
pub const GITHUB: &str = "https://github.com";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub packages: Vec<PackageSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageSpec {
    /// The package name, logix-core or logix-catalog.
    pub name: String,
    /// The package release, a date tag.
    pub version: String,
    pub description: String,
    /// The package file for each platform, such as darwin-arm64, or for
    /// "any" platform.
    pub files: BTreeMap<String, PackageFile>,
}

/// One package file, a GitHub release asset.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageFile {
    pub owner: String,
    pub repo: String,
    pub tag: String,
    /// The asset file name, a .tgz.
    pub file: String,
    /// The file size in bytes.
    pub size: u64,
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
                if f.size == 0 {
                    return Err(format!("{ctx}: size must be set"));
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

/// The manifest built into this engine release.
pub fn builtin() -> &'static Manifest {
    static MANIFEST: OnceLock<Manifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        Manifest::parse(include_str!("manifest.json")).expect("the built-in manifest is valid")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_manifest_is_valid_and_covers_every_platform() {
        let m = builtin();
        let names: Vec<&str> = m.packages.iter().map(|p| p.name.as_str()).collect();
        // The catalog first: it's small, so boards can be listed while the
        // tools download.
        assert_eq!(names, ["logix-catalog", "logix-core"]);
        let core = m.get("logix-core").unwrap();
        for platform in ["darwin-arm64", "linux-x86-64", "windows-x86-64"] {
            let f = &core.files[platform];
            assert_eq!(
                f.url(GITHUB),
                format!(
                    "https://github.com/openlogix/logix-core/releases/download/{}/logix-core-{platform}-{}.tgz",
                    core.version,
                    core.version.replace('-', "")
                )
            );
        }
        assert!(m.get("logix-catalog").unwrap().files.contains_key("any"));
        // The file name carries the same date as the package version.
        for p in &m.packages {
            let compact = p.version.replace('-', "");
            for f in p.files.values() {
                assert!(f.file.contains(&compact), "{} {}", p.name, f.file);
            }
        }
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
        let ok = include_str!("manifest.json");
        let core = builtin().get("logix-core").unwrap();
        let version = format!("\"{}\",\n      \"description\"", core.version);
        let file = &core.files["darwin-arm64"];
        let sha = &file.sha256;
        for (from, to) in [
            ("\"logix-core\"".to_string(), "\"../x\"".to_string()),
            (version, "\"latest\",\n      \"description\"".to_string()),
            (file.file.clone(), "../evil.tgz".to_string()),
            ("\"openlogix\"".to_string(), "\"a/b\"".to_string()),
            (sha.to_string(), sha.to_uppercase()),
        ] {
            assert!(ok.contains(&from), "{from}");
            let bad = ok.replacen(&from, &to, 1);
            assert!(Manifest::parse(&bad).is_err(), "{to} should be refused");
        }
    }
}
