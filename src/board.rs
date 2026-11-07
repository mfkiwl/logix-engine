//! Boards in requests (docs/tool-api.md, Board specification): every tool
//! that deals with boards names one by ID (`board`) and takes the same
//! optional custom boards file (`custom_boards_file`, read by
//! `custom_boards`): the host's own boards by ID, in the catalog's format. A custom board with a catalog board's ID replaces that
//! board for the request. The chosen board is resolved into a complete
//! board, which the build record keeps.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::catalog::{self, Dimension, FpgaDefinition, Variant};
use crate::errors::ToolError;

/// The host's own boards, by board ID (lowercase letters, digits and
/// hyphens), as board names them in requests, in the catalog's format. A
/// catalog board's ID replaces that catalog board in the request.
pub type CustomBoards = BTreeMap<String, CustomBoard>;

/// One of the host's own boards, in the catalog's format. Like a catalog
/// entry, except that the FPGA can be given either as fpga_id or as a full
/// fpga definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CustomBoard {
    /// The board's name for people.
    pub description: String,
    /// The board's FPGA as a catalog ID, such as ice40hx4k-tq144-8k. Give
    /// either fpga_id or fpga.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fpga_id: Option<String>,
    /// The board's FPGA as a full definition. Give either fpga_id or fpga.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fpga: Option<FpgaDefinition>,
    /// Ignored: the engine derives the dimensions from the variants. Here
    /// so a board copied from get_catalog can be sent back as is.
    #[serde(default, skip_serializing)]
    #[schemars(skip)]
    pub dimensions: Vec<Dimension>,
    /// The ways the board can be programmed, as in the catalog. The first is
    /// the default.
    pub variants: Vec<Variant>,
}

/// A board variant in a request: its selector, such as
/// {"connection": "ft232h", "memory": "ram"}. None means the board's first
/// variant, its default.
pub type VariantInput = Option<catalog::Selector>;

/// A board as matching and listing need it: the catalog's, or a custom one.
#[derive(Debug, Clone)]
pub struct KnownBoard<'a> {
    pub id: &'a str,
    pub custom: bool,
    pub description: &'a str,
    /// The FPGA's catalog ID, when it is one.
    pub fpga_id: Option<&'a str>,
    /// The FPGA definition (from the catalog for a catalog ID).
    pub fpga: FpgaDefinition,
    pub variants: &'a [Variant],
}

/// A custom board's FPGA: its catalog ID and definition.
fn custom_fpga(b: &CustomBoard) -> Result<(Option<String>, FpgaDefinition), ToolError> {
    match (&b.fpga_id, &b.fpga) {
        (Some(id), None) => Ok((Some(id.clone()), fpga_by_id(id)?)),
        (None, Some(f)) => {
            f.validate()
                .map_err(|e| ToolError::request(format!("invalid FPGA definition: {e}.")))?;
            Ok((None, f.clone()))
        }
        _ => Err(ToolError::request("give exactly one of fpga_id and fpga.")),
    }
}

/// Every board a request knows, by ID: the catalog's, with the request's
/// custom boards replacing catalog boards of the same ID and added to the
/// rest. The custom boards come from `custom_boards::load`, which checks
/// them.
pub fn known_boards(custom: &CustomBoards) -> BTreeMap<&str, KnownBoard<'_>> {
    let c = catalog::builtin();
    let mut all: BTreeMap<&str, KnownBoard<'_>> = c
        .boards
        .iter()
        .filter_map(|(id, b)| {
            Some((
                id.as_str(),
                KnownBoard {
                    id,
                    custom: false,
                    description: &b.description,
                    fpga_id: Some(&b.fpga_id),
                    fpga: c.fpgas.get(&b.fpga_id)?.clone(),
                    variants: &b.variants,
                },
            ))
        })
        .collect();
    for (id, b) in custom {
        let Ok((_, fpga)) = custom_fpga(b) else {
            continue;
        };
        all.insert(
            id,
            KnownBoard {
                id,
                custom: true,
                description: &b.description,
                fpga_id: b.fpga_id.as_deref(),
                fpga,
                variants: &b.variants,
            },
        );
    }
    all
}

/// A board with everything spelled out: what a build used.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedBoard {
    /// The board's ID. Optional only so older build records still read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board_id: Option<String>,
    pub description: String,
    /// The board's name with its variant, for messages, such as
    /// "Icesugar Pro (RAM)".
    pub name: String,
    /// The FPGA's catalog ID, when it is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fpga_id: Option<String>,
    pub fpga: FpgaDefinition,
    /// The chosen variant, with how to program the board that way.
    pub variant: Variant,
}

impl ResolvedBoard {
    /// What the build depends on: the FPGA's tool settings only. The board's
    /// name, notes, tested flag and its variant (how it's programmed) don't
    /// change the bitstream, so changing them never rebuilds.
    pub fn build_key(&self) -> serde_json::Value {
        self.fpga.build_key()
    }

    /// SHA-256 of the board as JSON, which is stable: struct fields and map
    /// keys have a fixed order. It guards the build record against damage;
    /// whether a build or an upload still fits uses `build_key`.
    pub fn checksum(&self) -> String {
        let json = serde_json::to_string(self).unwrap_or_default();
        hex::encode(Sha256::digest(json.as_bytes()))
    }
}

fn fpga_by_id(id: &str) -> Result<FpgaDefinition, ToolError> {
    catalog::builtin().fpgas.get(id).cloned().ok_or_else(|| {
        ToolError::request(format!(
            "{id:?} isn't an FPGA in the catalog. Use get_catalog to list them."
        ))
    })
}

/// Picks the variant, and names the board with it.
fn configure(
    description: &str,
    variants: &[Variant],
    variant: &VariantInput,
) -> Result<(Variant, String), ToolError> {
    let v = catalog::select_variant(variants, variant.as_ref())
        .map_err(|e| ToolError::request(format!("board_variant: {e}.")))?;
    // Only the dimensions the board has a choice in, e.g. "Icesugar Pro
    // (RAM)", not "Alhambra II (USB, Flash)".
    let choices: Vec<_> = catalog::derive_dimensions(variants)
        .into_iter()
        .filter(|d| d.values.len() > 1)
        .collect();
    let labels = v.labels(&choices);
    let name = if labels.is_empty() {
        description.to_string()
    } else {
        format!("{description} ({})", labels.join(", "))
    };
    Ok((v.clone(), name))
}

/// Resolves the board named `id`, and its variant: one of the request's
/// custom boards (from `custom_boards::load`), else a catalog board. Unknown IDs, bad
/// custom boards and variants the board doesn't have are request errors.
pub fn resolve(
    id: &str,
    variant: &VariantInput,
    custom: &CustomBoards,
) -> Result<ResolvedBoard, ToolError> {
    if let Some(b) = custom.get(id) {
        let (fpga_id, fpga) = custom_fpga(b)?;
        let (variant, name) = configure(&b.description, &b.variants, variant)?;
        return Ok(ResolvedBoard {
            board_id: Some(id.to_string()),
            description: b.description.clone(),
            name,
            fpga_id,
            fpga,
            variant,
        });
    }
    let b = catalog::builtin().boards.get(id).ok_or_else(|| {
        ToolError::request(format!(
            "{id:?} isn't a board in the catalog or in the custom boards file. Use get_catalog to list the boards, or add the board to the file custom_boards_file names."
        ))
    })?;
    let (variant, name) = configure(&b.description, &b.variants, variant)?;
    Ok(ResolvedBoard {
        board_id: Some(id.to_string()),
        description: b.description.clone(),
        name,
        fpga: fpga_by_id(&b.fpga_id)?,
        fpga_id: Some(b.fpga_id.clone()),
        variant,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pick(connection: &str, memory: &str) -> VariantInput {
        Some(catalog::Selector::new(connection, memory))
    }

    fn catalog_board(id: &str, variant: &VariantInput) -> Result<ResolvedBoard, ToolError> {
        resolve(id, variant, &CustomBoards::new())
    }

    /// Custom boards by ID.
    fn boards<const N: usize>(list: [(&str, CustomBoard); N]) -> CustomBoards {
        list.into_iter()
            .map(|(id, b)| (id.to_string(), b))
            .collect()
    }

    #[test]
    fn catalog_id() {
        let b = catalog_board("alhambra-ii", &None).unwrap();
        assert_eq!(b.board_id.as_deref(), Some("alhambra-ii"));
        assert_eq!(b.fpga.nextpnr_device, "hx8k");
        // One value per dimension here, so the name doesn't mention them.
        assert_eq!(b.name, "Alhambra II");
        assert_eq!(b.variant.selector, catalog::Selector::new("usb", "flash"));
        let err = catalog_board("nope", &None).unwrap_err();
        assert!(
            err.message.contains("custom boards file"),
            "{}",
            err.message
        );
    }

    #[test]
    fn board_variants_are_picked_by_selector() {
        // The default is the first variant.
        let dfu = catalog_board("butterstick-r10-2g-85k", &None).unwrap();
        assert_eq!(
            dfu.variant.selector,
            catalog::Selector::new("bootloader", "flash")
        );
        assert_eq!(dfu.name, "butterstick r1.0 (USB bootloader, Flash)");
        let cable = catalog_board("butterstick-r10-2g-85k", &pick("ft232h", "ram")).unwrap();
        assert_eq!(cable.variant.programmer, "openfpgaloader");
        assert_eq!(cable.variant.usb.pid, "6014");
        // Every variant has the board's FPGA.
        assert_eq!(dfu.fpga, cable.fpga);
        // Memory changes only the programming, not the build.
        let flash = catalog_board("icesugar-pro", &None).unwrap();
        let ram = catalog_board("icesugar-pro", &pick("usb", "ram")).unwrap();
        assert_eq!(flash.fpga, ram.fpga);
        assert_ne!(flash.variant.args, ram.variant.args);
        assert_eq!(flash.build_key(), ram.build_key());
        assert_ne!(flash.checksum(), ram.checksum(), "the record keeps it");
        assert_eq!(ram.name, "Icesugar Pro (RAM)");
        // An unknown variant is a request error that lists the variants.
        let e = catalog_board("butterstick-r10-2g-85k", &pick("bootloader", "ram")).unwrap_err();
        assert_eq!(e.category, crate::errors::Category::Request);
        assert!(
            e.message.contains(
                "(connection bootloader, memory flash), (connection ft2232h, memory ram), (connection ft232h, memory ram)"
            ),
            "{}",
            e.message
        );
    }

    pub(crate) fn custom(fpga: serde_json::Value) -> CustomBoard {
        let mut v = json!({
            "description": "My board",
            "variants": [{
                "selector": {"connection": "usb", "memory": "flash"},
                "programmer": "openfpgaloader",
                "args": "-b ice40_generic --vid ${VID} --pid ${PID} ${BITSTREAM}",
                "usb": {"vid": "0403", "pid": "6010"},
                "tested": false
            }]
        });
        for (k, val) in fpga.as_object().unwrap() {
            v[k] = val.clone();
        }
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn custom_boards() {
        let by_id = custom(json!({"fpga_id": "ice40up5k-sg48"}));
        let b = resolve("mine", &None, &boards([("mine", by_id.clone())])).unwrap();
        assert_eq!(b.fpga.nextpnr_device, "up5k");
        assert_eq!(b.board_id.as_deref(), Some("mine"));
        let inline = custom(json!({"fpga": {
            "architecture": "ice40", "part_number": "X", "size": "1k",
            "nextpnr_device": "hx1k", "nextpnr_package": "tq144"
        }}));
        let list = boards([("mine", by_id), ("inline", inline)]);
        let b2 = resolve("inline", &None, &list).unwrap();
        assert_eq!(b2.fpga.package(), "tq144");
        assert_eq!(b2.fpga_id, None);
        // The list doesn't change catalog boards it doesn't name.
        assert_eq!(
            resolve("alhambra-ii", &None, &list).unwrap(),
            catalog_board("alhambra-ii", &None).unwrap()
        );
        // A custom board replaces the catalog board with its ID.
        let over = boards([("alhambra-ii", custom(json!({"fpga_id": "ice40up5k-sg48"})))]);
        let b3 = resolve("alhambra-ii", &None, &over).unwrap();
        assert_eq!(b3.description, "My board");
        assert_ne!(
            b3.checksum(),
            catalog_board("alhambra-ii", &None).unwrap().checksum()
        );
    }

    #[test]
    fn a_board_copied_from_the_catalog_can_be_sent_back() {
        // The catalog file's entry, as is, under any ID.
        let copied = serde_json::to_value(&catalog::builtin().boards["icesugar-pro"]).unwrap();
        assert!(copied.get("dimensions").is_some());
        let b: CustomBoard = serde_json::from_value(copied).unwrap();
        let r = resolve("my-sugar", &pick("usb", "ram"), &boards([("my-sugar", b)])).unwrap();
        assert_eq!(r.variant.selector.memory, "ram");
    }

    #[test]
    fn a_custom_board_identical_to_the_catalog_board_is_the_same_board() {
        // As a host would send it: the catalog's board with its ID and
        // FPGA ID, the variants written out in full.
        let c = catalog::builtin();
        let b = &c.boards["icesugar-pro"];
        let same = CustomBoard {
            description: b.description.clone(),
            fpga_id: Some(b.fpga_id.clone()),
            fpga: None,
            dimensions: Vec::new(),
            variants: b.variants.clone(),
        };
        let ram = pick("usb", "ram");
        let from_list = resolve("icesugar-pro", &ram, &boards([("icesugar-pro", same)])).unwrap();
        let from_catalog = catalog_board("icesugar-pro", &ram).unwrap();
        assert_eq!(from_list.checksum(), from_catalog.checksum());
    }

    #[test]
    fn only_the_fpga_tool_settings_change_the_build() {
        let base = catalog_board("arty-a7-35t", &None).unwrap();
        let same = |f: &dyn Fn(&mut ResolvedBoard)| {
            let mut b = base.clone();
            f(&mut b);
            b.build_key() == base.build_key()
        };
        assert!(same(&|b| b.description = "Renamed".into()));
        assert!(same(&|b| b.fpga.part_number = "X".into()));
        assert!(same(&|b| b.fpga.size = "1k".into()));
        assert!(same(&|b| b.fpga.jtag_idcode = None));
        assert!(same(&|b| b.variant.args = "--other".into()));
        assert!(same(&|b| b.variant.tested = false));
        assert!(!same(&|b| b.fpga.nextpnr_device = "xc7a100tcsg324-1".into()));
        assert!(!same(&|b| b.fpga.prjxray_family = Some("spartan7".into())));
        assert!(!same(&|b| b.fpga.architecture = catalog::Architecture::Ecp5));
    }

    #[test]
    fn known_boards_put_custom_boards_first() {
        let list = boards([
            ("mine", custom(json!({"fpga_id": "ice40up5k-sg48"}))),
            ("alhambra-ii", custom(json!({"fpga_id": "ice40up5k-sg48"}))),
        ]);
        let all = known_boards(&list);
        assert!(all["mine"].custom);
        assert!(all["alhambra-ii"].custom, "replaced");
        assert!(!all["icestick"].custom);
        assert_eq!(all.len(), catalog::builtin().boards.len() + 1);
    }
}
