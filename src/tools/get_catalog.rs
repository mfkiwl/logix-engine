//! The get_catalog tool (docs/tool-api.md, Tables 14 and 15).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::EngineTool;
use crate::board::KnownBoard;
use crate::catalog::{self, Dimension, FpgaDefinition, Variant};
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
        ones. Boards include the request's custom_boards, marked custom, a custom board \
        replacing the catalog board with its ID. The architectures section gives each FPGA family's name for people and the \
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
    /// The host's own boards, by board ID, in the catalog's format.
    /// When boards are fetched, they are returned with the catalog's, with
    /// custom true; one with a catalog board's ID replaces that board. ids
    /// can name them. Send the same boards to every tool. Omitted means none.
    #[serde(default)]
    pub custom_boards: crate::board::CustomBoards,
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

/// A catalog board, as get_catalog returns it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogBoard {
    /// The board's name for people, such as "Alhambra II".
    pub description: String,
    /// The board's FPGA, as a catalog ID of the fpgas section, such as
    /// ice40hx4k-tq144-8k. Every variant uses it. Omitted for a custom
    /// board whose FPGA isn't in the catalog: see fpga.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fpga_id: Option<String>,
    /// The board's FPGA definition, only for a custom board whose FPGA isn't
    /// in the catalog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fpga: Option<FpgaDefinition>,
    /// The standard dimensions, connection then memory, each with the values
    /// the board's variants use, with labels and help from the engine, for
    /// showing the choices.
    pub dimensions: Vec<Dimension>,
    /// The ways the board can be programmed, the board's default first.
    pub variants: Vec<CatalogVariant>,
    /// True for a board from the request's custom_boards, omitted for
    /// catalog boards.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub custom: bool,
}

/// One way to program a catalog board. Which USB devices are the board is
/// the engine's own matter: list_devices gives each device the boards it
/// may be.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogVariant {
    /// The variant's ID within its board, <connection>-<memory>, such as
    /// usb-flash: what board_variant names in requests.
    pub id: String,
    /// How the computer reaches the board, a value of the connection
    /// dimension.
    pub connection: String,
    /// Where the design goes, a value of the memory dimension.
    pub memory: String,
    /// The programmer the engine runs: openfpgaloader, iceprog, dfu-util or
    /// icesprog.
    pub programmer: String,
    /// The programmer's arguments, as a template (see the variant fields of
    /// board definitions).
    pub args: String,
    /// Whether programming this way is known to work. False for ways that
    /// haven't been tried on hardware.
    pub tested: bool,
    /// Warnings and missing information for users.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_notes: Option<String>,
}

impl From<&Variant> for CatalogVariant {
    fn from(v: &Variant) -> Self {
        CatalogVariant {
            id: v.id.clone(),
            connection: v.connection.clone(),
            memory: v.memory.clone(),
            programmer: v.programmer.clone(),
            args: v.args.clone(),
            tested: v.tested,
            user_notes: v.user_notes.clone(),
        }
    }
}

impl From<&KnownBoard<'_>> for CatalogBoard {
    fn from(b: &KnownBoard<'_>) -> Self {
        CatalogBoard {
            description: b.description.to_string(),
            fpga_id: b.fpga_id.map(String::from),
            fpga: b.fpga_id.is_none().then(|| b.fpga.clone()),
            dimensions: catalog::derive_dimensions(b.variants),
            variants: b.variants.iter().map(Into::into).collect(),
            custom: b.custom,
        }
    }
}

/// The requested catalog entries. Sections that weren't fetched are empty.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCatalogOutput {
    /// Boards, by catalog ID.
    pub boards: BTreeMap<String, CatalogBoard>,
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
                    "{id:?} isn't in the {section} section. Fetch the section without ids to list them all."
                )));
            }
        }
    }
    Ok(out)
}

pub fn run(_state: &EngineState, input: GetCatalogInput) -> Result<GetCatalogOutput, ToolError> {
    answer(input)
}

fn answer(input: GetCatalogInput) -> Result<GetCatalogOutput, ToolError> {
    crate::board::check_custom_boards(&input.custom_boards)?;
    let known: BTreeMap<String, CatalogBoard> = crate::board::known_boards(&input.custom_boards)
        .iter()
        .map(|(id, b)| (id.to_string(), b.into()))
        .collect();
    Ok(GetCatalogOutput {
        boards: select("boards", &known, input.boards)?,
        fpgas: select("fpgas", &catalog::builtin().fpgas, input.fpgas)?,
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
    use serde_json::json;

    #[test]
    fn custom_boards_join_the_boards() {
        let ask = |v: serde_json::Value| {
            let input: GetCatalogInput = serde_json::from_value(v).unwrap();
            answer(input)
        };
        let mine = json!({
            "description": "My board",
            "fpga": {"architecture": "ice40", "part_number": "X", "size": "1k",
                     "nextpnr_device": "hx1k", "nextpnr_package": "tq144"},
            "variants": [{"id": "usb-flash", "connection": "usb", "memory": "flash",
                "programmer": "iceprog", "args": "-d d:${BUS}/${DEV} ${BITSTREAM}",
                "usb": {"vid": "0403", "pid": "6010"}, "tested": false}]
        });
        let mut over = mine.clone();
        over["description"] = json!("My Alhambra");
        let out = ask(json!({
            "boards": {"fetch": true, "ids": ["mine", "alhambra-ii", "icestick"]},
            "custom_boards": {"mine": mine, "alhambra-ii": over}
        }))
        .unwrap();
        let b = &out.boards["mine"];
        assert!(b.custom);
        assert_eq!(b.fpga_id, None);
        assert_eq!(b.fpga.as_ref().unwrap().part_number, "X");
        assert_eq!(b.dimensions.len(), 2);
        // Replaces the catalog board with its ID.
        assert!(out.boards["alhambra-ii"].custom);
        assert_eq!(out.boards["alhambra-ii"].description, "My Alhambra");
        let json = serde_json::to_value(&out).unwrap();
        assert!(json["boards"]["icestick"].get("custom").is_none());
        assert!(json["boards"]["icestick"].get("fpga").is_none());
        assert!(json["boards"]["mine"]["variants"][0].get("usb").is_none());
        // A bad board is a request error.
        let e =
            ask(json!({"custom_boards": {"x": {"description": "x", "variants": []}}})).unwrap_err();
        assert!(e.message.starts_with("custom_boards: x:"), "{}", e.message);
    }

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
