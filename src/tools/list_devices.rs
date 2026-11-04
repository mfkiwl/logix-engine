//! The list_devices tool (docs/tool-api.md, Tables 11 to 13).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::EngineTool;
use crate::board::BoardInputDefinition;
use crate::catalog;
use crate::devices::{self, Candidate, Device, UnreachableDevice};
use crate::errors::ToolError;
use crate::state::EngineState;

pub struct ListDevices;

impl EngineTool for ListDevices {
    const NAME: &'static str = "list_devices";
    const DESCRIPTION: &'static str = "Lists the USB devices connected to this computer, with \
        their USB IDs, names and serial numbers, and for each the boards it may be: catalog \
        boards, and the request's custom_boards, each with a confidence (named: the device's \
        name matched the board's; chip: only its USB chip did) and the variants it fits. \
        program_board needs the id of one of these devices: pick the device whose boards \
        include the build's board. Devices that need a driver or permissions first are listed \
        apart, under unreachable, with the fix.";
    const READ_ONLY: bool = true;
    type Input = ListDevicesInput;
    type Output = ListDevicesOutput;
}

/// Optionally, the host's own boards to match the devices against, besides
/// the catalog's.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListDevicesInput {
    /// The host's own board definitions, by board ID (lowercase letters,
    /// digits and hyphens), in the same format as the board field of other
    /// tools. Devices are matched against them as well as the catalog's
    /// boards; matches name them with custom true. Omitted means none.
    #[serde(default)]
    pub custom_boards: BTreeMap<String, BoardInputDefinition>,
}

/// The connected devices.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListDevicesOutput {
    /// The connected USB devices the engine can open, sorted by vendor ID,
    /// product ID, bus and device number.
    pub devices: Vec<Device>,
    /// Devices the engine sees but can't open, so it can't program them or
    /// read their names, and whose USB IDs fit a board: each with the fix,
    /// such as permissions or a driver. Other devices it can't open are left
    /// out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unreachable: Vec<UnreachableDevice>,
}

pub fn run_with(
    input: ListDevicesInput,
    scan: impl FnOnce() -> Result<Vec<Device>, ToolError>,
) -> Result<ListDevicesOutput, ToolError> {
    let mut custom = Vec::new();
    for (id, b) in &input.custom_boards {
        if !catalog::is_catalog_id(id) {
            return Err(ToolError::request(format!(
                "custom_boards: {id:?} isn't a board ID: use lowercase letters, digits and hyphens."
            )));
        }
        b.variants.validate().map_err(|e| {
            ToolError::request(format!(
                "custom_boards: {id}: invalid board definition: {e}."
            ))
        })?;
        custom.push((id, b));
    }
    let mut boards: Vec<Candidate> = catalog::builtin()
        .boards
        .iter()
        .map(|(id, b)| Candidate {
            id,
            custom: false,
            variants: &b.variants,
        })
        .collect();
    boards.extend(custom.iter().map(|(id, b)| Candidate {
        id,
        custom: true,
        variants: &b.variants,
    }));

    let mut out = ListDevicesOutput {
        devices: Vec::new(),
        unreachable: Vec::new(),
    };
    for mut d in scan()? {
        let matches = devices::board_matches(&d, &boards);
        if d.reachable {
            d.boards = matches;
            out.devices.push(d);
        } else if !matches.is_empty() {
            out.unreachable.push(UnreachableDevice {
                kind: d.kind,
                vid: d.vid,
                pid: d.pid,
                bus: d.bus,
                device: d.device,
                boards: matches,
                action: devices::unreachable_action(),
            });
        }
    }
    Ok(out)
}

pub fn run(_state: &EngineState, input: ListDevicesInput) -> Result<ListDevicesOutput, ToolError> {
    run_with(input, devices::scan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::Confidence;
    use crate::devices::tests::device;
    use serde_json::json;

    fn list(
        input: serde_json::Value,
        devices: Vec<Device>,
    ) -> Result<ListDevicesOutput, ToolError> {
        let input: ListDevicesInput = serde_json::from_value(input).unwrap();
        run_with(input, move || Ok(devices))
    }

    #[test]
    fn devices_come_with_the_boards_they_may_be() {
        let out = list(
            json!({}),
            vec![
                device(
                    "1209",
                    "5af0",
                    Some("OrangeCrab r0.2 DFU bootloader"),
                    None,
                    2,
                ),
                device("0403", "6014", Some("Single RS232-HS"), None, 3),
                device("1234", "5678", Some("A mouse"), None, 4),
            ],
        )
        .unwrap();
        let first = |i: usize| out.devices[i].boards.first().cloned();
        let bootloader = first(0).unwrap();
        // Two boards share the OrangeCrab bootloader's name: both named.
        assert_eq!(bootloader.board, "orangecrab-r02-25f");
        assert_eq!(bootloader.confidence, Confidence::Named);
        assert_eq!(out.devices[0].boards[1].board, "orangecrab-r02-85f");
        assert_eq!(
            bootloader.variants,
            [BTreeMap::from([
                ("connection".to_string(), "bootloader".to_string()),
                ("memory".to_string(), "flash".to_string())
            ])]
        );
        // The FT232H adapter: the ButterStick's adapter variants, among others.
        let adapter = &out.devices[1].boards;
        let butterstick = adapter
            .iter()
            .find(|m| m.board == "butterstick-r10-2g-85k")
            .unwrap();
        assert_eq!(butterstick.confidence, Confidence::Chip);
        assert_eq!(butterstick.variants[0]["connection"], "ft232h");
        assert!(out.devices[2].boards.is_empty());
        assert!(out.unreachable.is_empty());
    }

    #[test]
    fn custom_boards_are_matched_too() {
        let custom = json!({"custom_boards": {"alhambra-ii": {
            "description": "My Alhambra",
            "fpga_id": "ice40hx4k-tq144-8k",
            "variants": {"configurations": [{
                "when": {"connection": "usb", "memory": "flash"},
                "programmer": "iceprog",
                "args": "-d d:${BUS}/${DEV} ${BITSTREAM}",
                "usb": {"vid": "0403", "pid": "6010", "product_regex": "^Alhambra"},
                "tested": true
            }]}
        }}});
        let out = list(
            custom,
            vec![device("0403", "6010", Some("Alhambra II"), None, 3)],
        )
        .unwrap();
        let named: Vec<_> = out.devices[0]
            .boards
            .iter()
            .filter(|m| m.confidence == Confidence::Named)
            .map(|m| (m.board.as_str(), m.custom))
            .collect();
        assert_eq!(named, [("alhambra-ii", false), ("alhambra-ii", true)]);

        let bad = list(
            json!({"custom_boards": {"Bad ID": {
                "description": "x", "fpga_id": "x", "variants": {"configurations": []}
            }}}),
            vec![],
        );
        assert_eq!(bad.unwrap_err().category, crate::errors::Category::Request);
        let bad = list(
            json!({"custom_boards": {"mine": {
                "description": "x", "fpga_id": "x", "variants": {"configurations": []}
            }}}),
            vec![],
        );
        assert!(bad.unwrap_err().message.contains("mine"));
    }

    #[test]
    fn devices_that_cant_be_opened_are_listed_apart_when_they_may_be_boards() {
        let mut ftdi = device("0403", "6010", None, None, 3);
        ftdi.reachable = false;
        let mut mouse = device("1234", "5678", None, None, 4);
        mouse.reachable = false;
        let out = list(json!({}), vec![ftdi, mouse]).unwrap();
        assert!(out.devices.is_empty());
        assert_eq!(out.unreachable.len(), 1);
        let u = &out.unreachable[0];
        assert_eq!((u.vid.as_str(), u.pid.as_str()), ("0403", "6010"));
        assert!(u.boards.iter().any(|m| m.board == "alhambra-ii"));
        assert!(u.boards.iter().all(|m| m.confidence == Confidence::Chip));
        assert_eq!(u.action, devices::unreachable_action());
        let v = serde_json::to_value(&out).unwrap();
        assert!(
            v["unreachable"][0].get("id").is_none(),
            "can't be programmed"
        );
    }
}
