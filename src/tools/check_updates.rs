//! The check_updates tool (docs/tool-api.md, check_updates): reads the
//! engine's remote config now, downloads the package releases it names and
//! switches to them (Remote updates, in the website repo's CLAUDE.md).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::get_engine_info::{PackageInfo, RemoteConfigInfo, package_list, remote_config_info};
use super::{Call, EngineTool};
use crate::errors::ToolError;
use crate::packages::Update;
use crate::progress::Phases;
use crate::state::EngineState;

pub struct CheckUpdates;

impl EngineTool for CheckUpdates {
    const NAME: &'static str = "check_updates";
    const DESCRIPTION: &'static str = "Checks now for newer releases of the engine's packages \
        (the board catalog and the FPGA tools): reads the engine's remote config, downloads \
        what it names that isn't downloaded yet, and uses it from the next call. The engine \
        also checks on its own, before an operation that needs packages, when it last checked \
        more than 24 hours ago, so call this only to check now, such as when the user asks for \
        updates. It works even when automatic download is off. A download can take a few \
        minutes; the tool reports progress and can be cancelled.";
    const READ_ONLY: bool = false;
    const OPEN_WORLD: bool = true;
    type Input = CheckUpdatesInput;
    type Output = CheckUpdatesOutput;
}

/// The check_updates tool takes no inputs.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckUpdatesInput {}

/// What changed, and every package's status afterwards.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckUpdatesOutput {
    /// The packages whose release changed, each with the release used before
    /// (from) and now (to). Empty when nothing changed.
    pub updates: Vec<UpdateInfo>,
    /// Names of the packages this call downloaded or repaired.
    pub downloaded: Vec<String>,
    /// Every package the engine uses, with its status.
    pub packages: Vec<PackageInfo>,
    /// The remote config just read.
    pub remote_config: RemoteConfigInfo,
}

/// A package whose release changed.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateInfo {
    /// The package name, such as logix-catalog.
    pub name: String,
    /// The release used before, as yyyy.mm.dd; omitted for a new package.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// The release used now, as yyyy.mm.dd.
    pub to: String,
}

impl From<Update> for UpdateInfo {
    fn from(u: Update) -> Self {
        UpdateInfo {
            name: u.name,
            from: u.from,
            to: u.to,
        }
    }
}

pub async fn run(
    state: &EngineState,
    _input: CheckUpdatesInput,
    call: &Call,
) -> Result<CheckUpdatesOutput, ToolError> {
    let updates = state.packages.refresh(true, true).await?;
    let names: Vec<String> = state
        .packages
        .manifest()
        .packages
        .iter()
        .map(|p| p.name.clone())
        .collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let needed = state.packages.needing_download(&names).await?;
    let phases = Phases::new(call.progress.clone(), needed.len());
    // The host asked, so this downloads even when automatic download is off.
    state
        .packages
        .ensure(&needed, true, &phases, &call.cancel)
        .await?;
    Ok(CheckUpdatesOutput {
        updates: updates.into_iter().map(UpdateInfo::from).collect(),
        downloaded: needed.into_iter().map(|s| s.name).collect(),
        packages: package_list(state),
        remote_config: remote_config_info(&state.packages),
    })
}
