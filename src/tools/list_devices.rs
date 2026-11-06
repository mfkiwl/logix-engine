//! The list_devices tool (docs/tool-api.md, Tables 11 to 13).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::EngineTool;
use crate::devices::{self, Candidate, Device, MatchLevel, UnreachableDevice};
use crate::errors::ToolError;
use crate::state::EngineState;

pub struct ListDevices;

impl EngineTool for ListDevices {
    const NAME: &'static str = "list_devices";
    const DESCRIPTION: &'static str = "Lists the USB devices connected to this computer, with \
        their USB IDs, names and serial numbers, and for each the boards it may be: catalog \
        boards and the request's custom_boards, best first, each with the variants it fits and \
        how well it matches: strong (very likely this board), possible (a few other boards look \
        the same) or weak (many boards fit). Show the level as it is: how the engine decides it \
        is internal and changes without notice. \
        program_board needs the id of one of these devices. Given a build_folder, it lists only \
        the devices that fit that build's board and variant, best first, each with its \
        build_match: program the only one, or ask the user when there are several. \
        Devices that need a driver or permissions first are listed apart, under unreachable, \
        with the fix.";
    const READ_ONLY: bool = true;
    type Input = ListDevicesInput;
    type Output = ListDevicesOutput;
}

/// Optionally, the host's own boards to match the devices against, besides
/// the catalog's, and a build whose board the devices must fit.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListDevicesInput {
    /// The host's own boards, in the catalog's format, each with its id.
    /// Devices are matched against them too; matches name them with custom
    /// true. One with a catalog board's ID replaces that board. Send the
    /// same list to every tool. Omitted means none.
    #[serde(default)]
    pub custom_boards: Vec<crate::board::CustomBoard>,
    /// The build folder of a successful build, as an absolute path. When
    /// given, only devices that fit the build's board in its variant are
    /// listed (in devices and unreachable), each with build_match, best
    /// first: the devices program_board can program that build into. The
    /// build's board comes from its build record, custom boards included.
    #[serde(default)]
    pub build_folder: Option<String>,
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
    crate::board::check_custom_boards(&input.custom_boards)?;
    let known = crate::board::known_boards(&input.custom_boards);
    let boards: Vec<Candidate> = known
        .values()
        .map(|b| Candidate {
            id: b.id,
            custom: b.custom,
            variants: b.variants,
        })
        .collect();

    let build_usb = match &input.build_folder {
        Some(folder) => Some(
            super::program_board::read_build(folder)?
                .1
                .board
                .variant
                .usb,
        ),
        None => None,
    };
    let mut out = ListDevicesOutput {
        devices: Vec::new(),
        unreachable: Vec::new(),
    };
    for mut d in scan()? {
        let build_level = match &build_usb {
            None => None,
            Some(usb) => {
                let fits = if d.reachable {
                    devices::matches(&d, usb)
                } else {
                    devices::same_ids(&d, usb)
                };
                if !fits {
                    continue;
                }
                Some(devices::match_level(&d, usb))
            }
        };
        let (matches, promoted) = devices::board_matches_and_promotion(&d, &boards);
        // The build's level is raised as the device's other matches are.
        let build_match = build_level.map(|l| {
            if promoted && l == MatchLevel::Weak {
                MatchLevel::Possible
            } else {
                l
            }
        });
        if d.reachable {
            d.boards = matches;
            d.build_match = build_match;
            out.devices.push(d);
        } else if !matches.is_empty() || build_match.is_some() {
            out.unreachable.push(UnreachableDevice {
                kind: d.kind,
                vid: d.vid,
                pid: d.pid,
                bus: d.bus,
                device: d.device,
                boards: matches,
                action: devices::unreachable_action(),
                build_match,
            });
        }
    }
    if build_usb.is_some() {
        // Best first; the sort is stable, so equal ones keep the USB order.
        out.devices
            .sort_by_key(|d| std::cmp::Reverse(d.build_match));
    }
    Ok(out)
}

pub fn run(_state: &EngineState, input: ListDevicesInput) -> Result<ListDevicesOutput, ToolError> {
    run_with(input, devices::scan)
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(bootloader.best_level(), MatchLevel::Strong);
        assert_eq!(out.devices[0].boards[1].board, "orangecrab-r02-85f");
        assert_eq!(
            bootloader.variants,
            [crate::devices::VariantMatch {
                id: "bootloader-flash".into(),
                level: MatchLevel::Strong,
            }]
        );
        // The FT232H adapter: the ButterStick's adapter variants, among others.
        let adapter = &out.devices[1].boards;
        let butterstick = adapter
            .iter()
            .find(|m| m.board == "butterstick-r10-2g-85k")
            .unwrap();
        assert_eq!(butterstick.best_level(), MatchLevel::Weak);
        assert_eq!(butterstick.variants[0].id, "ft232h-ram");
        assert_eq!(butterstick.variants[0].level, MatchLevel::Weak);
        assert!(out.devices[2].boards.is_empty());
        assert!(out.unreachable.is_empty());
    }

    #[test]
    fn custom_boards_are_matched_too() {
        let mine = |id: &str| {
            json!({
                "id": id, "description": "My Alhambra", "fpga_id": "ice40hx4k-tq144-8k",
                "variants": [{
                    "id": "usb-flash", "connection": "usb", "memory": "flash",
                    "programmer": "iceprog", "args": "-d d:${BUS}/${DEV} ${BITSTREAM}",
                    "usb": {"vid": "0403", "pid": "6010", "product_regex": "^Alhambra"},
                    "tested": true
                }]
            })
        };
        let alhambra = || vec![device("0403", "6010", Some("Alhambra II"), None, 3)];
        let out = list(json!({"custom_boards": [mine("mine")]}), alhambra()).unwrap();
        let strong: Vec<_> = out.devices[0]
            .boards
            .iter()
            .filter(|m| m.best_level() == MatchLevel::Strong)
            .map(|m| (m.board.as_str(), m.custom))
            .collect();
        assert_eq!(strong, [("alhambra-ii", false), ("mine", true)]);
        // One with a catalog board's ID replaces it.
        let out = list(json!({"custom_boards": [mine("alhambra-ii")]}), alhambra()).unwrap();
        let named: Vec<_> = out.devices[0]
            .boards
            .iter()
            .filter(|m| m.board == "alhambra-ii")
            .map(|m| m.custom)
            .collect();
        assert_eq!(named, [true]);
        // A bad list is a request error.
        let e = list(
            json!({"custom_boards": [{"id": "Bad ID", "description": "x", "fpga_id": "x", "variants": []}]}),
            vec![],
        )
        .unwrap_err();
        assert_eq!(e.category, crate::errors::Category::Request);
    }

    #[test]
    fn each_variant_has_its_own_level() {
        let config = |memory: &str, usb: serde_json::Value| {
            json!({
                "id": format!("usb-{memory}"), "connection": "usb", "memory": memory,
                "programmer": "iceprog",
                "args": "-d d:${BUS}/${DEV} ${BITSTREAM}",
                "usb": usb,
                "tested": true
            })
        };
        // Variants with different filters, as a board file or the catalog
        // may have.
        let variants: Vec<crate::catalog::Variant> = serde_json::from_value(json!([
            config(
                "flash",
                json!({"vid": "0403", "pid": "6010", "product_regex": "^Dual RS232-HS"})
            ),
            config("ram", json!({"vid": "0403", "pid": "6010"})),
        ]))
        .unwrap();
        let boards = [Candidate {
            id: "mine",
            custom: true,
            variants: &variants,
        }];
        let d = device("0403", "6010", Some("Dual RS232-HS"), None, 3);
        let (matches, _) = devices::board_matches_and_promotion(&d, &boards);
        let mine = &matches[0];
        let rated: Vec<_> = mine
            .variants
            .iter()
            .map(|v| (v.id.as_str(), v.level))
            .collect();
        assert_eq!(
            rated,
            [
                ("usb-flash", MatchLevel::Possible),
                ("usb-ram", MatchLevel::Weak)
            ]
        );
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
        assert!(u.boards.iter().all(|m| m.best_level() == MatchLevel::Weak));
        assert_eq!(u.action, devices::unreachable_action());
        let v = serde_json::to_value(&out).unwrap();
        assert!(
            v["unreachable"][0].get("id").is_none(),
            "can't be programmed"
        );
        assert!(
            v["unreachable"][0]["boards"][0].get("match").is_none(),
            "per variant only"
        );
        assert_eq!(
            v["unreachable"][0]["boards"][0]["variants"][0]["match"],
            "weak"
        );
    }
}
