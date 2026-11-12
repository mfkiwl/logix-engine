//! What an engine process knows from its launch. Requests are stateless: the
//! only state is the launch settings, which are fixed for the life of the
//! process.

use std::path::PathBuf;

use rmcp::model::ProtocolVersion;

/// The engine version as people see it, yyyy.mm.dd: its release date. Set
/// by build.rs.
pub const ENGINE_VERSION: &str = env!("LOGIX_ENGINE_BUILD_VERSION");

/// The MCP protocol versions the engine accepts, newest first. The engine is
/// pinned to 2026-07-28 and also accepts 2025-11-25 clients.
pub const PROTOCOL_VERSIONS: &[ProtocolVersion] =
    &[ProtocolVersion::V_2026_07_28, ProtocolVersion::V_2025_11_25];

/// The platform the engine was built for, such as `darwin-arm64`.
pub fn platform() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x86-64",
        other => other,
    };
    format!("{os}-{arch}")
}

#[derive(Debug, Clone)]
pub struct EngineState {
    /// The engine folder as given on the command line.
    pub engine_folder: String,
    /// The engine folder after resolving symbolic links. All disk changes
    /// use this path.
    pub engine_folder_resolved: PathBuf,
    pub auto_download: bool,
    /// The command-line arguments, as given, without the program name.
    pub command_line: Vec<String>,
    pub log_file: PathBuf,
    /// Downloads and finds the packages in the engine folder.
    pub packages: crate::packages::PackageManager,
}

impl EngineState {
    /// Makes sure the board catalog is loaded: the logix-catalog release the
    /// remote config names, downloaded first when needed and auto-download
    /// is on.
    /// A catalog folder from LOGIX_ENGINE_DEV_CATALOG_FOLDER, loaded at
    /// startup, stays.
    pub async fn ensure_catalog(
        &self,
        call: &crate::tools::Call,
    ) -> Result<(), crate::errors::ToolError> {
        use crate::packages::CATALOG;
        if crate::catalog::source().is_some_and(|s| s.local) {
            return Ok(());
        }
        let needed = self.packages.needing_download(&[CATALOG]).await?;
        let spec = self.packages.spec(CATALOG)?;
        if !needed.is_empty() {
            let phases = crate::progress::Phases::new(call.progress.clone(), 1);
            self.packages
                .ensure(&needed, self.auto_download, &phases, &call.cancel)
                .await?;
        }
        let folder = self.packages.package_dir(&spec);
        crate::catalog::load(&folder, &spec.version, false).map_err(|e| {
            crate::errors::ToolError::setup(format!("Can't read the board catalog: {e}"))
                .with_action(
                    crate::errors::ActionCode::DownloadPackages,
                    format!("Download the {CATALOG} package again, which may be damaged."),
                )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_version_is_a_dotted_date() {
        let v = ENGINE_VERSION;
        assert_eq!(v.len(), 10, "{v}");
        assert!(
            crate::packages::manifest::is_date_tag(&v.replace('.', "-")) && !v.contains('-'),
            "{v}"
        );
    }

    #[test]
    fn platform_name() {
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            assert_eq!(platform(), "darwin-arm64");
        }
    }
}
