//! The "board specification" input type (docs/tool-api.md): a catalog ID, or
//! a full board definition whose FPGA is a catalog ID or a full definition.
//! It is resolved into a complete board, which the build record keeps.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

use crate::catalog::{self, Configuration, FpgaDefinition, Variants};
use crate::errors::ToolError;

/// A board: a catalog ID, or a full definition.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum BoardSpec {
    /// A board's catalog ID, such as alhambra-ii. get_catalog lists them.
    Id(#[schemars(pattern(r"^[a-z0-9-]+$"))] String),
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
    /// The board's choices and how to program it with each, as in the
    /// catalog.
    pub variants: Variants,
}

/// A board's variant: a value for some or all of its dimensions, by
/// dimension ID, such as {"connection": "ft232h", "memory": "ram"}. The first
/// configuration with these values is used; omitted dimensions match any
/// value, so an empty variant means the board's default configuration.
pub type VariantInput = BTreeMap<String, String>;

// A clearer error than serde's "did not match any variant".
impl<'de> Deserialize<'de> for BoardSpec {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        match v {
            serde_json::Value::String(s) => Ok(BoardSpec::Id(s)),
            serde_json::Value::Object(_) => serde_json::from_value(v)
                .map(|b| BoardSpec::Definition(Box::new(b)))
                .map_err(|e| serde::de::Error::custom(format!("board definition: {e}"))),
            _ => Err(serde::de::Error::custom(
                "board must be a catalog ID string or a board definition object",
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
    /// "ULX3S (85K)".
    pub name: String,
    /// The chosen configuration's values, by dimension ID. Empty for a board
    /// without dimensions.
    pub variant: BTreeMap<String, String>,
    /// The FPGA's catalog ID, when it is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fpga_id: Option<String>,
    pub fpga: FpgaDefinition,
    /// How to program the board with the chosen configuration.
    pub configuration: Configuration,
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

/// Picks the configuration for a variant, and names the board with it.
fn configure(
    description: &str,
    variants: &Variants,
    variant: &VariantInput,
) -> Result<(Configuration, String), ToolError> {
    let c = variants
        .select(variant)
        .map_err(|e| ToolError::request(format!("board_variant: {e}.")))?;
    // Only the dimensions the board has a choice in, e.g. "Icesugar Pro
    // (RAM)", not "Alhambra II (USB, Flash)".
    let choices: Vec<_> = variants
        .dimensions
        .iter()
        .filter(|d| d.values.len() > 1)
        .cloned()
        .collect();
    let labels = c.labels(&choices);
    let name = if labels.is_empty() {
        description.to_string()
    } else {
        format!("{description} ({})", labels.join(", "))
    };
    Ok((c.clone(), name))
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
            let (configuration, name) = configure(&b.description, &b.variants, variant)?;
            Ok(ResolvedBoard {
                board_id: Some(id.clone()),
                description: b.description.clone(),
                name,
                variant: configuration.when.clone(),
                fpga: fpga_by_id(&b.fpga_id)?,
                fpga_id: Some(b.fpga_id.clone()),
                configuration,
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
            let mut variants = d.variants.clone();
            variants
                .validate()
                .map_err(|e| ToolError::request(format!("Invalid board definition: {e}.")))?;
            variants.derive_dimensions();
            let (configuration, name) = configure(&d.description, &variants, variant)?;
            Ok(ResolvedBoard {
                board_id: None,
                description: d.description.clone(),
                name,
                variant: configuration.when.clone(),
                fpga_id,
                fpga,
                configuration,
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

    fn variant(v: serde_json::Value) -> VariantInput {
        serde_json::from_value(v).unwrap()
    }

    fn id(s: &str) -> BoardSpec {
        BoardSpec::Id(s.into())
    }

    #[test]
    fn catalog_id() {
        let b = resolve(&parse(json!("alhambra-ii")).unwrap(), &VariantInput::new()).unwrap();
        assert_eq!(b.board_id.as_deref(), Some("alhambra-ii"));
        assert_eq!(b.fpga.nextpnr_device, "hx8k");
        // Every board has the standard dimensions; one value each here, so
        // the name doesn't mention them.
        assert_eq!(b.name, "Alhambra II");
        assert_eq!(
            b.variant,
            variant(json!({"connection": "usb", "memory": "flash"}))
        );
        let err = resolve(&parse(json!("nope")).unwrap(), &VariantInput::new()).unwrap_err();
        assert!(err.message.contains("get_catalog"), "{}", err.message);
    }

    #[test]
    fn variants_pick_a_configuration() {
        // The default is the first configuration.
        let dfu = resolve(&id("butterstick-r10-2g-85k"), &VariantInput::new()).unwrap();
        assert_eq!(
            dfu.variant,
            variant(json!({"connection": "bootloader", "memory": "flash"}))
        );
        assert_eq!(dfu.name, "butterstick r1.0 (USB bootloader, Flash)");
        // RAM alone picks the first cable, the first configuration with it.
        let ram = resolve(
            &id("butterstick-r10-2g-85k"),
            &variant(json!({"memory": "ram"})),
        )
        .unwrap();
        assert_eq!(
            ram.variant,
            variant(json!({"connection": "ft2232h", "memory": "ram"}))
        );
        // The bootloader can't write RAM.
        let e = resolve(
            &id("butterstick-r10-2g-85k"),
            &variant(json!({"connection": "bootloader", "memory": "ram"})),
        )
        .unwrap_err();
        assert!(e.message.contains("no configuration"), "{}", e.message);
        let cable = resolve(
            &id("butterstick-r10-2g-85k"),
            &variant(json!({"connection": "ft232h"})),
        )
        .unwrap();
        assert_eq!(cable.configuration.programmer, "openfpgaloader");
        assert_eq!(cable.configuration.usb.pid, "6014");
        // Every configuration has the board's FPGA.
        assert_eq!(dfu.fpga, cable.fpga);
        // Memory changes only the programming, and still the checksum, so
        // the build starts over.
        let flash = resolve(&id("icesugar-pro"), &VariantInput::new()).unwrap();
        let ram = resolve(&id("icesugar-pro"), &variant(json!({"memory": "ram"}))).unwrap();
        assert_eq!(flash.fpga, ram.fpga);
        assert_ne!(flash.configuration.args, ram.configuration.args);
        assert_ne!(flash.checksum(), ram.checksum());
        assert_eq!(ram.name, "Icesugar Pro (RAM)");
        // Errors name what exists.
        let e = |b: &str, v| resolve(&id(b), &variant(v)).unwrap_err().message;
        assert!(e("icesugar-pro", json!({"memory": "eeprom"})).contains("flash, ram"));
        assert!(
            e("icesugar-pro", json!({"size": "1k"})).contains("dimensions: connection, memory")
        );
        assert!(e("alhambra-ii", json!({"memory": "ram"})).contains("Its values: flash"));
    }

    #[test]
    fn partial_variants_and_missing_combinations() {
        let mut variants: Variants = serde_json::from_value(json!({
            "configurations": [
                {"when": {"connection": "bootloader", "memory": "flash"}, "programmer": "dfu-util",
                 "args": "-D ${BITSTREAM}", "usb": {"vid": "1209", "pid": "5af0"}, "tested": true},
                {"when": {"connection": "ft232h", "memory": "flash"}, "programmer": "openfpgaloader",
                 "args": "-c ft232 -f ${BITSTREAM}", "usb": {"vid": "0403", "pid": "6014"}, "tested": true},
                {"when": {"connection": "ft232h", "memory": "ram"}, "programmer": "openfpgaloader",
                 "args": "-c ft232 ${BITSTREAM}", "usb": {"vid": "0403", "pid": "6014"}, "tested": true}
            ]
        }))
        .unwrap();
        variants.derive_dimensions();
        // The dimensions come from the configurations, in the standard order,
        // with the engine's labels.
        let dims: Vec<Vec<&str>> = variants
            .dimensions
            .iter()
            .map(|d| d.values.iter().map(|v| v.label.as_str()).collect())
            .collect();
        assert_eq!(
            dims,
            [
                vec!["USB bootloader", "FT232H adapter"],
                vec!["Flash", "RAM"]
            ]
        );
        variants.validate().unwrap();
        // RAM alone picks the only configuration with it.
        let c = variants.select(&variant(json!({"memory": "ram"}))).unwrap();
        assert_eq!(c.args, "-c ft232 ${BITSTREAM}");
        assert_eq!(c.labels(&variants.dimensions), ["FT232H adapter", "RAM"]);
        // A combination that isn't listed doesn't exist.
        let e = variants
            .select(&variant(
                json!({"connection": "bootloader", "memory": "ram"}),
            ))
            .unwrap_err();
        assert!(e.contains("no configuration"), "{e}");
        // The catalog's rules.
        let mut bad = variants.clone();
        bad.configurations[1].when.remove("memory");
        assert!(
            bad.validate()
                .unwrap_err()
                .contains("connection and a memory")
        );
        let mut bad = variants.clone();
        bad.configurations[1]
            .when
            .insert("memory".into(), "eeprom".into());
        assert!(bad.validate().unwrap_err().contains("flash, ram"));
        let mut bad = variants.clone();
        bad.configurations[2].when = bad.configurations[1].when.clone();
        assert!(bad.validate().is_err());
        let mut bad = variants;
        bad.configurations.clear();
        assert!(bad.validate().is_err());
    }

    fn custom(fpga: serde_json::Value) -> serde_json::Value {
        let mut v = json!({
            "description": "My board",
            "variants": {"configurations": [{
                "when": {"connection": "usb", "memory": "flash"},
                "programmer": "openfpgaloader",
                "args": "-b ice40_generic --vid ${VID} --pid ${PID} ${BITSTREAM}",
                "usb": {"vid": "0403", "pid": "6010"},
                "tested": false
            }]}
        });
        for (k, val) in fpga.as_object().unwrap() {
            v[k] = val.clone();
        }
        v
    }

    #[test]
    fn full_definitions() {
        let none = VariantInput::new();
        let by_id = resolve(
            &parse(custom(json!({"fpga_id": "ice40up5k-sg48"}))).unwrap(),
            &none,
        )
        .unwrap();
        assert_eq!(by_id.fpga.nextpnr_device, "up5k");
        let inline = resolve(
            &parse(custom(json!({"fpga": {
                "architecture": "ice40", "part_number": "X", "size": "1k",
                "nextpnr_device": "hx1k", "nextpnr_package": "tq144"
            }})))
            .unwrap(),
            &none,
        )
        .unwrap();
        assert_eq!(inline.fpga.package(), "tq144");
        assert_ne!(by_id.checksum(), inline.checksum());
        // Neither or both.
        assert!(resolve(&parse(custom(json!({}))).unwrap(), &none).is_err());
        // A bad configuration.
        let mut bad = custom(json!({"fpga_id": "ice40up5k-sg48"}));
        bad["variants"]["configurations"][0]["programmer"] = json!("tinyprog");
        assert!(resolve(&parse(bad).unwrap(), &none).is_err());
    }

    #[test]
    fn clear_parse_errors() {
        assert!(parse(json!(5)).unwrap_err().contains("catalog ID string"));
        let err = parse(json!({"description": "x"})).unwrap_err();
        assert!(err.contains("board definition"), "{err}");
    }
}
