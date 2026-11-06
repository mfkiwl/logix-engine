//! The "board specification" input type (docs/tool-api.md): a catalog ID, or
//! a full board definition whose FPGA is a catalog ID or a full definition.
//! It is resolved into a complete board, which the build record keeps.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

use crate::catalog::{self, Dimension, FpgaDefinition, Variant};
use crate::errors::ToolError;

/// A board: a catalog ID, a board file, or a full definition.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum BoardSpec {
    /// A board's catalog ID, such as alhambra-ii. get_catalog lists them.
    Id(#[schemars(pattern(r"^[a-z0-9-]+$"))] String),
    /// A board file, as make_board_file returned it: pass it unchanged.
    File(
        #[schemars(with = "serde_json::Map<String, serde_json::Value>")]
        Box<crate::board_file::BoardFile>,
    ),
    /// A full board definition, for boards that aren't in the catalog.
    Definition(Box<BoardInputDefinition>),
}

/// A board definition in a request. Like a catalog entry, except that the
/// FPGA can be given either as fpga_id or as a full fpga definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoardInputDefinition {
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
    /// so a definition copied from get_catalog can be sent back as is.
    #[serde(default, skip_serializing)]
    #[schemars(skip)]
    pub dimensions: Vec<Dimension>,
    /// The ways the board can be programmed, as in the catalog. The first is
    /// the default.
    pub variants: Vec<Variant>,
}

/// A board variant in a request: the variant's ID, such as "ft232h-ram".
/// None means the board's first variant, its default.
pub type VariantInput = Option<String>;

// A clearer error than serde's "did not match any variant".
impl<'de> Deserialize<'de> for BoardSpec {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        match v {
            serde_json::Value::String(s) => Ok(BoardSpec::Id(s)),
            serde_json::Value::Object(ref o) if o.contains_key("logix_board_file") => {
                crate::board_file::BoardFile::from_value(&v)
                    .map(|f| BoardSpec::File(Box::new(f)))
                    .map_err(|e| serde::de::Error::custom(e.message))
            }
            serde_json::Value::Object(_) => serde_json::from_value(v)
                .map(|b| BoardSpec::Definition(Box::new(b)))
                .map_err(|e| serde::de::Error::custom(format!("board definition: {e}"))),
            _ => Err(serde::de::Error::custom(
                "board must be a catalog ID string, a board file or a board definition object",
            )),
        }
    }
}

/// A board with everything spelled out: what a build used.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedBoard {
    /// The board's catalog ID, when it came from the catalog.
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
    /// SHA-256 of the board as JSON, which is stable: struct fields and map
    /// keys have a fixed order.
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
    let v = catalog::select_variant(variants, variant.as_deref())
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

/// Resolves a board input and its variant against the catalog. Unknown IDs,
/// bad definitions and variants the board doesn't have are request errors.
pub fn resolve(spec: &BoardSpec, variant: &VariantInput) -> Result<ResolvedBoard, ToolError> {
    match spec {
        BoardSpec::Id(id) => {
            let b = catalog::builtin().boards.get(id).ok_or_else(|| {
                ToolError::request(format!(
                    "{id:?} isn't a board in the catalog. Use get_catalog to list the boards, or give a full board definition."
                ))
            })?;
            let (variant, name) = configure(&b.description, &b.variants, variant)?;
            Ok(ResolvedBoard {
                board_id: Some(id.clone()),
                description: b.description.clone(),
                name,
                fpga: fpga_by_id(&b.fpga_id)?,
                fpga_id: Some(b.fpga_id.clone()),
                variant,
            })
        }
        BoardSpec::File(f) => {
            let (variant, name) = configure(&f.name, &f.variants, variant)?;
            Ok(ResolvedBoard {
                board_id: None,
                description: f.name.clone(),
                name,
                fpga_id: Some(f.fpga_id.clone()),
                fpga: f.fpga.clone(),
                variant,
            })
        }
        BoardSpec::Definition(d) => {
            let (fpga_id, fpga) = match (&d.fpga_id, &d.fpga) {
                (Some(id), None) => (Some(id.clone()), fpga_by_id(id)?),
                (None, Some(f)) => {
                    f.validate().map_err(|e| {
                        ToolError::request(format!("Invalid FPGA definition: {e}."))
                    })?;
                    (None, f.clone())
                }
                _ => {
                    return Err(ToolError::request(
                        "A board definition needs exactly one of fpga_id and fpga.",
                    ));
                }
            };
            catalog::validate_variants(&d.variants)
                .map_err(|e| ToolError::request(format!("Invalid board definition: {e}.")))?;
            let (variant, name) = configure(&d.description, &d.variants, variant)?;
            Ok(ResolvedBoard {
                board_id: None,
                description: d.description.clone(),
                name,
                fpga_id,
                fpga,
                variant,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(v: serde_json::Value) -> Result<BoardSpec, String> {
        serde_json::from_value(v).map_err(|e| e.to_string())
    }

    fn id(s: &str) -> BoardSpec {
        BoardSpec::Id(s.into())
    }

    fn pick(v: &str) -> VariantInput {
        Some(v.to_string())
    }

    #[test]
    fn board_file() {
        let d = crate::devices::tests::device("0403", "6010", Some("Digilent USB Device"), None, 4);
        let file = crate::board_file::make(&d, &[], "arty-a7-100t", Some("My Arty"), false)
            .unwrap()
            .file;
        let spec = parse(file.to_value()).unwrap();
        let b = resolve(&spec, &pick("usb-ram")).unwrap();
        assert_eq!(b.description, "My Arty");
        assert_eq!(b.name, "My Arty (RAM)");
        assert_eq!(b.board_id, None);
        assert_eq!(b.fpga.part_number, file.fpga.part_number);
        assert_eq!(
            b.variant.usb.product_regex.as_deref(),
            Some("^Digilent USB Device$")
        );
        // A damaged file says so.
        let mut bad = file.to_value();
        bad["variants"] = json!([]);
        let e = parse(bad).unwrap_err();
        assert!(e.contains("damaged"), "{e}");
    }

    #[test]
    fn catalog_id() {
        let b = resolve(&parse(json!("alhambra-ii")).unwrap(), &None).unwrap();
        assert_eq!(b.board_id.as_deref(), Some("alhambra-ii"));
        assert_eq!(b.fpga.nextpnr_device, "hx8k");
        // One value per dimension here, so the name doesn't mention them.
        assert_eq!(b.name, "Alhambra II");
        assert_eq!(b.variant.id, "usb-flash");
        let err = resolve(&parse(json!("nope")).unwrap(), &None).unwrap_err();
        assert!(err.message.contains("get_catalog"), "{}", err.message);
    }

    #[test]
    fn board_variants_are_picked_by_id() {
        // The default is the first variant.
        let dfu = resolve(&id("butterstick-r10-2g-85k"), &None).unwrap();
        assert_eq!(dfu.variant.id, "bootloader-flash");
        assert_eq!(dfu.name, "butterstick r1.0 (USB bootloader, Flash)");
        let cable = resolve(&id("butterstick-r10-2g-85k"), &pick("ft232h-ram")).unwrap();
        assert_eq!(cable.variant.programmer, "openfpgaloader");
        assert_eq!(cable.variant.usb.pid, "6014");
        // Every variant has the board's FPGA.
        assert_eq!(dfu.fpga, cable.fpga);
        // Memory changes only the programming, and still the checksum, so
        // the build starts over.
        let flash = resolve(&id("icesugar-pro"), &None).unwrap();
        let ram = resolve(&id("icesugar-pro"), &pick("usb-ram")).unwrap();
        assert_eq!(flash.fpga, ram.fpga);
        assert_ne!(flash.variant.args, ram.variant.args);
        assert_ne!(flash.checksum(), ram.checksum());
        assert_eq!(ram.name, "Icesugar Pro (RAM)");
        // An unknown variant is a request error that lists the variants.
        let e = resolve(&id("butterstick-r10-2g-85k"), &pick("bootloader-ram")).unwrap_err();
        assert_eq!(e.category, crate::errors::Category::Request);
        assert!(
            e.message
                .contains("bootloader-flash, ft2232h-ram, ft232h-ram"),
            "{}",
            e.message
        );
    }

    fn custom(fpga: serde_json::Value) -> serde_json::Value {
        let mut v = json!({
            "description": "My board",
            "variants": [{
                "id": "usb-flash", "connection": "usb", "memory": "flash",
                "programmer": "openfpgaloader",
                "args": "-b ice40_generic --vid ${VID} --pid ${PID} ${BITSTREAM}",
                "usb": {"vid": "0403", "pid": "6010"},
                "tested": false
            }]
        });
        for (k, val) in fpga.as_object().unwrap() {
            v[k] = val.clone();
        }
        v
    }

    #[test]
    fn full_definitions() {
        let by_id = resolve(
            &parse(custom(json!({"fpga_id": "ice40up5k-sg48"}))).unwrap(),
            &None,
        )
        .unwrap();
        assert_eq!(by_id.fpga.nextpnr_device, "up5k");
        let inline = resolve(
            &parse(custom(json!({"fpga": {
                "architecture": "ice40", "part_number": "X", "size": "1k",
                "nextpnr_device": "hx1k", "nextpnr_package": "tq144"
            }})))
            .unwrap(),
            &None,
        )
        .unwrap();
        assert_eq!(inline.fpga.package(), "tq144");
        assert_ne!(by_id.checksum(), inline.checksum());
        // Neither or both.
        assert!(resolve(&parse(custom(json!({}))).unwrap(), &None).is_err());
        // A bad variant.
        let mut bad = custom(json!({"fpga_id": "ice40up5k-sg48"}));
        bad["variants"][0]["programmer"] = json!("tinyprog");
        assert!(resolve(&parse(bad).unwrap(), &None).is_err());
        // A definition copied from get_catalog, dimensions and all, is fine.
        let mut copied = serde_json::to_value(&catalog::builtin().boards["icesugar-pro"]).unwrap();
        assert!(copied.get("dimensions").is_some());
        copied["fpga_id"] = json!("lfe5u-25f-6bg256c");
        let b = resolve(&parse(copied).unwrap(), &pick("usb-ram")).unwrap();
        assert_eq!(b.variant.memory, "ram");
    }

    #[test]
    fn clear_parse_errors() {
        assert!(parse(json!(5)).unwrap_err().contains("catalog ID string"));
        let err = parse(json!({"description": "x"})).unwrap_err();
        assert!(err.contains("board definition"), "{err}");
    }
}
