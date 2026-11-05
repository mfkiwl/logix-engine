//! The get_catalog tool (docs/tool-api.md, Tables 14 and 15).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::EngineTool;
use crate::catalog::{self, BoardDefinition, FpgaDefinition};
use crate::errors::ToolError;
use crate::state::EngineState;

pub struct GetCatalog;

impl EngineTool for GetCatalog {
    const NAME: &'static str = "get_catalog";
    const DESCRIPTION: &'static str = "Returns board and FPGA definitions from the engine's \
        read-only catalog. Use it to find a board's catalog ID, such as alhambra-ii, to pass as \
        the board of other tools, or to see a board's FPGA and how it is programmed. Each \
        section is returned only when asked for: {\"boards\": {\"fetch\": true}} returns all \
        boards, and {\"boards\": {\"fetch\": true, \"ids\": [\"alhambra-ii\"]}} only the listed \
        ones. The architectures section gives each FPGA family's name for people and the \
        extension of its pin constraint files. Board descriptions are the boards' names for \
        people.";
    const READ_ONLY: bool = true;
    type Input = GetCatalogInput;
    type Output = GetCatalogOutput;
}

/// Which catalog entries to return.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCatalogInput {
    /// Which boards to return. Omitted means none.
    #[serde(default)]
    pub boards: Option<CatalogQuery>,
    /// Which FPGAs to return. Omitted means none.
    #[serde(default)]
    pub fpgas: Option<CatalogQuery>,
    /// Which FPGA architectures (families) to return, by ID, such as ice40.
    /// Omitted means none.
    #[serde(default)]
    pub architectures: Option<CatalogQuery>,
}

/// An FPGA architecture (family), as FPGA definitions name it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureInfo {
    /// The family's name for people, with its vendor, such as "Lattice
    /// iCE40", for grouping boards.
    pub label: String,
    /// The family's short name, such as "iCE40", for messages.
    pub short_label: String,
    /// The extension of the family's pin constraint files, without the
    /// dot, such as pcf: the build takes only this format for the family's
    /// boards.
    pub constraint_extension: String,
    /// The family's place in the engine's order (0 first), for listing
    /// families in a fixed order.
    pub order: u32,
}

/// Selects entries from one catalog section.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogQuery {
    /// Whether to return this section. Defaults to false.
    #[serde(default)]
    pub fetch: bool,
    /// Catalog IDs of the entries to return. Omitted means all entries. An
    /// ID that isn't in the section is an error.
    #[serde(default)]
    pub ids: Option<Vec<String>>,
}

/// The requested catalog entries. Sections that weren't fetched are empty.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCatalogOutput {
    /// Board definitions, by catalog ID.
    pub boards: BTreeMap<String, BoardDefinition>,
    /// FPGA definitions, by catalog ID.
    pub fpgas: BTreeMap<String, FpgaDefinition>,
    /// FPGA architectures (families), by ID, such as ice40.
    #[serde(default)]
    pub architectures: BTreeMap<String, ArchitectureInfo>,
}

fn select<T: Clone>(
    section: &str,
    all: &BTreeMap<String, T>,
    query: Option<CatalogQuery>,
) -> Result<BTreeMap<String, T>, ToolError> {
    let Some(query) = query.filter(|q| q.fetch) else {
        return Ok(BTreeMap::new());
    };
    let Some(ids) = query.ids else {
        return Ok(all.clone());
    };
    let mut out = BTreeMap::new();
    for id in ids {
        match all.get(&id) {
            Some(entry) => {
                out.insert(id, entry.clone());
            }
            None => {
                return Err(ToolError::request(format!(
                    "{id:?} isn't in the catalog's {section} section. Fetch the section without ids to list them all."
                )));
            }
        }
    }
    Ok(out)
}

pub fn run(_state: &EngineState, input: GetCatalogInput) -> Result<GetCatalogOutput, ToolError> {
    let c = catalog::builtin();
    Ok(GetCatalogOutput {
        boards: select("boards", &c.boards, input.boards)?,
        fpgas: select("fpgas", &c.fpgas, input.fpgas)?,
        architectures: select("architectures", &architectures(), input.architectures)?,
    })
}

/// Every architecture the engine supports, by ID.
fn architectures() -> BTreeMap<String, ArchitectureInfo> {
    catalog::Architecture::ALL
        .iter()
        .enumerate()
        .map(|(i, a)| {
            (
                a.id().to_string(),
                ArchitectureInfo {
                    label: a.full_label().to_string(),
                    short_label: a.label().to_string(),
                    constraint_extension: a.constraint_extension().to_string(),
                    order: i as u32,
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Architecture;

    #[test]
    fn architectures_have_labels_and_pin_file_extensions() {
        let all = architectures();
        let q: CatalogQuery = serde_json::from_value(serde_json::json!({"fetch": true})).unwrap();
        let out = select("architectures", &all, Some(q)).unwrap();
        assert!(
            select("architectures", &all, None).unwrap().is_empty(),
            "only when asked"
        );
        let ice40 = &out["ice40"];
        assert_eq!(ice40.label, "Lattice iCE40");
        assert_eq!(ice40.short_label, "iCE40");
        assert_eq!(ice40.constraint_extension, "pcf");
        assert_eq!(ice40.order, 0);
        assert_eq!(out["xilinx"].constraint_extension, "xdc");
        assert_eq!(out.len(), Architecture::ALL.len());
        // Each ID is the one FPGA definitions use.
        for a in Architecture::ALL {
            assert_eq!(serde_json::to_value(a).unwrap(), a.id());
        }
    }
}
