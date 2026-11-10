//! The get_engine_info tool (docs/tool-api.md, Tables 16 and 17).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::EngineTool;
use crate::errors::ToolError;
use crate::packages::PackageManager;
use crate::packages::manifest::PackageSpec;
use crate::state::{self, EngineState};

pub struct GetEngineInfo;

impl EngineTool for GetEngineInfo {
    const NAME: &'static str = "get_engine_info";
    const DESCRIPTION: &'static str = "Returns the engine's version, platform, launch settings and \
        package status. Call it first, to check that the engine is new enough and to see which \
        toolchain packages are downloaded. It is fast and local: it never uses the network. \
        Missing packages are downloaded automatically by the tools that need them, unless \
        auto_download is off.";
    const READ_ONLY: bool = true;
    type Input = GetEngineInfoInput;
    type Output = GetEngineInfoOutput;
}

/// The get_engine_info tool takes no inputs.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetEngineInfoInput {}

/// The engine's version, platform, launch settings and package status.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetEngineInfoOutput {
    /// The engine release, as a date tag (yyyy-mm-dd, or yyyy-mm-dd-N for a
    /// later release on the same day). Compare versions by date.
    #[schemars(pattern(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}(-[1-9][0-9]*)?$"))]
    pub engine_version: String,
    /// The MCP protocol versions the engine accepts, newest first.
    pub protocol_versions: Vec<String>,
    /// The platform the engine runs on, such as darwin-arm64.
    pub platform: String,
    /// The engine folder, as the host set it with the --engine-folder
    /// command-line argument. The engine keeps its packages and logs there.
    pub engine_folder: String,
    /// Whether the engine downloads and repairs packages automatically when a
    /// tool needs them. On by default. The host turns it off with the
    /// --no-auto-download command-line argument.
    pub auto_download: bool,
    /// The command-line arguments the engine was launched with, as given.
    pub command_line: Vec<String>,
    /// The log file this engine process writes, in the logs subfolder of the
    /// engine folder. Useful when reporting a problem.
    pub log_file: String,
    /// Every package in the engine's manifest, with its status.
    pub packages: Vec<PackageInfo>,
    /// The address of the board schema this engine reads (Logix Catalog's):
    /// the JSON Schema of each board file, in the catalog and in custom
    /// boards folders, which may name it in a "$schema" key so editors can
    /// check them. A file that names another schema is refused.
    pub boards_schema: String,
    /// The board catalog built into this engine: its Logix Catalog version.
    pub catalog: CatalogInfo,
    /// The facts above as plain text lines for people, the engine's part of
    /// a frontend's system info (for bug reports), so every frontend shows
    /// them the same way: the version, platform, engine folder, log file,
    /// automatic download, command line, each package with its version and
    /// status, the board catalog and the boards schema. Show it as is; its wording can change
    /// in any version, so read the fields above, not this text.
    pub report: String,
}

/// The board catalog built into the engine (Logix Catalog,
/// github.com/openlogix/logix-catalog).
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogInfo {
    /// The catalog's version: the date (yyyy-mm-dd) of its Logix Catalog
    /// commit.
    pub version: String,
    /// The Logix Catalog commit, a short hash such as 3cdae93.
    pub commit: String,
    /// True when this engine was built from a local catalog clone (a
    /// developer build), which may differ from the commit; omitted
    /// otherwise.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub local: bool,
}

/// A toolchain package and whether it's downloaded.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackageInfo {
    /// The package name, such as oss-cad-suite.
    pub name: String,
    /// The package release, as a date tag (yyyy-mm-dd).
    #[schemars(pattern(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}(-[1-9][0-9]*)?$"))]
    pub version: String,
    pub status: PackageStatus,
    /// The folder with the package's programs, as an absolute path, when
    /// the package is downloaded. For hosts that run one of its programs
    /// directly, such as Verible's language server verible-verilog-ls from
    /// the verible package. Omitted when the package isn't downloaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin_folder: Option<String>,
}

/// Whether a package is ready to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PackageStatus {
    /// Downloaded and intact.
    Ready,
    /// Not downloaded.
    Missing,
    /// Downloaded but incomplete or damaged. It is repaired when a tool needs
    /// it, unless auto-download is off.
    Broken,
}

pub fn run(
    state: &EngineState,
    _input: GetEngineInfoInput,
) -> Result<GetEngineInfoOutput, ToolError> {
    let mut out = GetEngineInfoOutput {
        engine_version: state::ENGINE_VERSION.to_string(),
        protocol_versions: state::PROTOCOL_VERSIONS
            .iter()
            .map(|v| v.to_string())
            .collect(),
        platform: state::platform(),
        engine_folder: state.engine_folder.clone(),
        auto_download: state.auto_download,
        command_line: state.command_line.clone(),
        log_file: state.log_file.display().to_string(),
        packages: package_list(state),
        boards_schema: crate::catalog::BOARDS_SCHEMA.to_string(),
        catalog: {
            let (commit, date, local) = crate::catalog::source();
            CatalogInfo {
                version: date.to_string(),
                commit: commit.to_string(),
                local,
            }
        },
        report: String::new(),
    };
    out.report = report(&out);
    Ok(out)
}

/// The engine's part of a system info report (`report`).
fn report(info: &GetEngineInfoOutput) -> String {
    let mut lines = vec![
        format!("Logix Engine {}", info.engine_version),
        format!("Platform: {}", info.platform),
        format!("Engine folder: {}", info.engine_folder),
        format!("Log file: {}", info.log_file),
        format!(
            "Automatic package download: {}",
            if info.auto_download { "on" } else { "off" }
        ),
        format!("Command line: {}", info.command_line.join(" ")),
        "Packages:".to_string(),
    ];
    // In columns: name, version, status.
    let name_width = info
        .packages
        .iter()
        .map(|p| p.name.len())
        .max()
        .unwrap_or(0);
    let version_width = info
        .packages
        .iter()
        .map(|p| p.version.len())
        .max()
        .unwrap_or(0);
    for p in &info.packages {
        let status = match p.status {
            PackageStatus::Ready => "ready",
            PackageStatus::Missing => "missing",
            PackageStatus::Broken => "broken",
        };
        lines.push(format!(
            "  {:name_width$}  {:version_width$}  {status}",
            p.name, p.version
        ));
    }
    let c = &info.catalog;
    lines.push(format!(
        "Board catalog: Logix Catalog {} ({}{})",
        c.version,
        c.commit,
        if c.local { ", local build" } else { "" }
    ));
    lines.push(format!("Boards schema: {}", info.boards_schema));
    lines.join("\n")
}

/// Every package in the manifest, with its status.
pub fn package_list(state: &EngineState) -> Vec<PackageInfo> {
    state
        .packages
        .manifest()
        .packages
        .iter()
        .map(|spec| package_info(&state.packages, spec))
        .collect()
}

/// One package's name, version, status and, when downloaded, its programs'
/// folder.
pub fn package_info(packages: &PackageManager, spec: &PackageSpec) -> PackageInfo {
    let status = packages.status(spec);
    PackageInfo {
        name: spec.name.clone(),
        version: spec.version.clone(),
        status,
        bin_folder: (status == PackageStatus::Ready)
            .then(|| packages.package_dir(spec).join("bin").display().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_lists_the_facts_for_people() {
        let mut info = GetEngineInfoOutput {
            engine_version: "2026-11-08".into(),
            protocol_versions: vec!["2025-11-25".into()],
            platform: "darwin-arm64".into(),
            engine_folder: "/Users/you/.logix/ide/engine".into(),
            auto_download: true,
            command_line: vec![
                "--engine-folder".into(),
                "/Users/you/.logix/ide/engine".into(),
            ],
            log_file: "/Users/you/.logix/ide/engine/logs/20261108-1.log".into(),
            packages: vec![
                PackageInfo {
                    name: "oss-cad-suite".into(),
                    version: "2026-11-20".into(),
                    status: PackageStatus::Ready,
                    bin_folder: Some("/x/bin".into()),
                },
                PackageInfo {
                    name: "verible".into(),
                    version: "2026-10-22".into(),
                    status: PackageStatus::Missing,
                    bin_folder: None,
                },
            ],
            boards_schema: crate::catalog::BOARDS_SCHEMA.into(),
            catalog: CatalogInfo {
                version: "2026-11-08".into(),
                commit: "3cdae93".into(),
                local: false,
            },
            report: String::new(),
        };
        info.report = report(&info);
        assert_eq!(
            info.report,
            "Logix Engine 2026-11-08
Platform: darwin-arm64
Engine folder: /Users/you/.logix/ide/engine
Log file: /Users/you/.logix/ide/engine/logs/20261108-1.log
Automatic package download: on
Command line: --engine-folder /Users/you/.logix/ide/engine
Packages:
  oss-cad-suite  2026-11-20  ready
  verible        2026-10-22  missing
Board catalog: Logix Catalog 2026-11-08 (3cdae93)
Boards schema: https://www.openlogix.org/schemas/logix-board-v1.json"
        );
    }
}
