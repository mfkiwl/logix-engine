//! The get_catalog tool (docs/tool-api.md, Tables 14 and 15).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::EngineTool;
use crate::catalog::{self, BoardDefinition, Dimension, FpgaDefinition, Variant};
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
    /// Board files, as make_board_file returned them, sent unchanged. When
    /// boards are fetched, their boards are returned too, by their board
    /// IDs, with custom true, so a board picker shows them with the
    /// catalog's. Omitted means none.
    #[serde(default)]
    #[schemars(with = "Vec<serde_json::Map<String, serde_json::Value>>")]
    pub board_files: Vec<serde_json::Value>,
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
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogBoard {
    /// The board's name for people, such as "Alhambra II".
    pub description: String,
    /// The board's FPGA, as a catalog ID of the fpgas section, such as
    /// ice40hx4k-tq144-8k. Every variant uses it.
    pub fpga_id: String,
    /// The standard dimensions, connection then memory, each with the values
    /// the board's variants use, with labels and help from the engine, for
    /// showing the choices.
    pub dimensions: Vec<Dimension>,
    /// The ways the board can be programmed, the board's default first.
    pub variants: Vec<CatalogVariant>,
    /// True for a board from one of the request's board_files, omitted for
    /// catalog boards.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub custom: bool,
}

/// One way to program a catalog board. Which USB devices are the board is
/// the engine's own matter: list_devices gives each device the boards it
/// may be.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
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

impl From<&BoardDefinition> for CatalogBoard {
    fn from(b: &BoardDefinition) -> Self {
        CatalogBoard {
            description: b.description.clone(),
            fpga_id: b.fpga_id.clone(),
            dimensions: b.dimensions.clone(),
            variants: b.variants.iter().map(Into::into).collect(),
            custom: false,
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
                    "{id:?} isn't in the catalog's {section} section. Fetch the section without ids to list them all."
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
    let c = catalog::builtin();
    let files = crate::board_file::read_all(&input.board_files)?;
    let fetch_boards = input.boards.as_ref().is_some_and(|q| q.fetch);
    let mut boards: BTreeMap<String, CatalogBoard> = select("boards", &c.boards, input.boards)?
        .iter()
        .map(|(id, b)| (id.clone(), b.into()))
        .collect();
    let mut fpgas = select("fpgas", &c.fpgas, input.fpgas)?;
    if fetch_boards {
        for f in &files {
            boards.insert(
                f.id.clone(),
                CatalogBoard {
                    description: f.name.clone(),
                    fpga_id: f.fpga_id.clone(),
                    dimensions: catalog::derive_dimensions(&f.variants),
                    variants: f.variants.iter().map(Into::into).collect(),
                    custom: true,
                },
            );
            // A board file's FPGA, for grouping, when this catalog lacks it.
            if !fpgas.is_empty() && !c.fpgas.contains_key(&f.fpga_id) {
                fpgas.insert(f.fpga_id.clone(), f.fpga.clone());
            }
        }
    }
    Ok(GetCatalogOutput {
        boards,
        fpgas,
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
    fn board_files_join_the_boards() {
        let d = crate::devices::tests::device("0403", "6010", Some("Digilent USB Device"), None, 4);
        let file = crate::board_file::make(&d, &[], "arty-a7-100t", Some("My Arty"), false)
            .unwrap()
            .file;
        let ask = |boards: bool| {
            let input: GetCatalogInput = serde_json::from_value(serde_json::json!({
                "boards": {"fetch": boards, "ids": ["alhambra-ii"]},
                "board_files": [file.to_value()]
            }))
            .unwrap();
            answer(input).unwrap()
        };
        let out = ask(true);
        let mine = &out.boards[&file.id];
        assert_eq!(mine.description, "My Arty");
        assert!(mine.custom);
        assert_eq!(mine.fpga_id, "xc7a100tcsg324-1");
        assert_eq!(mine.dimensions[1].values.len(), 2, "flash and RAM");
        assert!(!out.boards["alhambra-ii"].custom);
        let json = serde_json::to_value(&out).unwrap();
        assert!(json["boards"]["alhambra-ii"].get("custom").is_none());
        assert!(json["boards"][&file.id]["variants"][0].get("usb").is_none());
        // Only when boards are fetched.
        assert!(ask(false).boards.is_empty());
        // A bad file is a request error that names it.
        let input: GetCatalogInput =
            serde_json::from_value(serde_json::json!({"board_files": [{"x": 1}]})).unwrap();
        let e = answer(input).unwrap_err();
        assert!(e.message.starts_with("board_files[0]:"), "{}", e.message);
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
