//! The list_devices tool (docs/tool-api.md, Tables 11 to 13).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::EngineTool;
use crate::board::{self, BoardSpec, VariantInput};
use crate::devices::{self, Device};
use crate::errors::ToolError;
use crate::state::EngineState;

pub struct ListDevices;

impl EngineTool for ListDevices {
    const NAME: &'static str = "list_devices";
    const DESCRIPTION: &'static str = "Lists the USB devices connected to this computer, with \
        their USB IDs, names and serial numbers. Given a board, each device also says whether it \
        matches that board, which shows whether the board is connected and whether several \
        are. Use it before program_board, or to find a board's serial number when several \
        boards of the same kind are connected.";
    const READ_ONLY: bool = true;
    type Input = ListDevicesInput;
    type Output = ListDevicesOutput;
}

/// Optionally, a board to match the devices against.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListDevicesInput {
    /// A board, as a catalog ID such as alhambra-ii or a full definition.
    /// When given, each device reports whether it matches this board.
    #[serde(default)]
    pub board: Option<BoardSpec>,
    /// The board's variant, such as {"size": "85k"} or {"memory": "ram"}:
    /// values of the board's dimensions, which get_catalog lists with the
    /// configurations that exist. The first configuration with these values
    /// is used; omitted dimensions match any value. Omitted means the
    /// board's first configuration. Its USB filter is the one matched.
    #[serde(default)]
    pub variant: VariantInput,
}

/// The connected devices.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListDevicesOutput {
    /// The connected USB devices, sorted by vendor ID, product ID, bus and
    /// device number.
    pub devices: Vec<Device>,
}

pub fn run_with(
    input: ListDevicesInput,
    scan: impl FnOnce() -> Result<Vec<Device>, ToolError>,
) -> Result<ListDevicesOutput, ToolError> {
    let filter = match &input.board {
        Some(spec) => {
            let b = board::resolve(spec, &input.variant)?;
            Some(b.configuration.usb)
        }
        None => None,
    };
    let mut list = scan()?;
    if let Some(f) = &filter {
        for d in &mut list {
            d.matches_board = Some(devices::matches(d, f));
        }
    }
    Ok(ListDevicesOutput { devices: list })
}

pub fn run(_state: &EngineState, input: ListDevicesInput) -> Result<ListDevicesOutput, ToolError> {
    run_with(input, devices::scan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::DeviceKind;
    use serde_json::json;

    fn usb(vid: &str, pid: &str) -> Device {
        Device {
            kind: DeviceKind::Usb,
            vid: vid.into(),
            pid: pid.into(),
            manufacturer: None,
            product: None,
            serial_number: None,
            bus: 1,
            device: 2,
            matches_board: None,
        }
    }

    #[test]
    fn the_variant_chooses_the_usb_filter() {
        let scan = || Ok(vec![usb("0403", "6010"), usb("0403", "6014")]);
        let matches = |input: serde_json::Value| -> Vec<Option<bool>> {
            let input: ListDevicesInput = serde_json::from_value(input).unwrap();
            run_with(input, scan)
                .unwrap()
                .devices
                .iter()
                .map(|d| d.matches_board)
                .collect()
        };
        assert_eq!(matches(json!({})), [None, None]);
        let board = json!("butterstick-r10-2g-85k");
        // The default is the USB bootloader, which isn't connected.
        assert_eq!(matches(json!({"board": board})), [Some(false), Some(false)]);
        assert_eq!(
            matches(json!({"board": board, "variant": {"connection": "ft232h"}})),
            [Some(false), Some(true)]
        );
        let input: ListDevicesInput =
            serde_json::from_value(json!({"board": board, "variant": {"connection": "jtag"}}))
                .unwrap();
        assert!(run_with(input, scan).is_err());
    }
}
