//! USB device scanning, device IDs and board matching (docs/requirements.md,
//! "Device selection"). The engine uses libusb, built in, like the
//! programmers do, so bus and device numbers mean the same to both.
//!
//! Matching runs on every request against the boards it is given (the
//! catalog and the request's own definitions), never against precomputed
//! data, since hosts can define boards at any time.

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{UsbFilter, Variant};
use crate::errors::{Action, ActionCode, ToolError};

/// A connected device that the engine can open.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Device {
    /// The device's ID, to pass to program_board. Opaque: don't parse it.
    /// It names the device on this computer for a short time: it stays
    /// valid while the device stays on the same USB port, so list the
    /// devices again before asking the user to choose.
    pub id: String,
    /// The kind of device: usb.
    pub kind: DeviceKind,
    /// USB vendor ID, four lowercase hex digits.
    pub vid: String,
    /// USB product ID, four lowercase hex digits.
    pub pid: String,
    /// The manufacturer name the device reports, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    /// The product name the device reports, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    /// The serial number the device reports, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial_number: Option<String>,
    /// USB bus number.
    pub bus: u8,
    /// USB device number on its bus. It changes when the device is plugged
    /// in again.
    pub device: u8,
    /// The boards whose USB filters this device matches, best first. Empty
    /// for a device that is no known board.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub boards: Vec<BoardMatch>,
    /// How sure the engine is that the device is the board of the given
    /// build, in the build's variant, from 1 to 3 as in boards. Present
    /// only when list_devices was given a build_folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 3))]
    pub build_confidence: Option<u8>,
    /// Where the device is plugged in, such as 1-2.3 (bus, then ports).
    #[serde(skip)]
    #[schemars(skip)]
    pub location: String,
    /// Whether the engine could open the device and read its names. Devices
    /// it can't open need a driver or permissions first.
    #[serde(skip)]
    #[schemars(skip)]
    pub reachable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    /// A USB device.
    Usb,
}

/// A board a device may be.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoardMatch {
    /// The board's ID: a catalog ID, or a key of the request's
    /// custom_boards when custom is true.
    pub board: String,
    /// True when the board is one of the request's custom_boards rather
    /// than the catalog's. A custom board can have a catalog board's ID; the
    /// host decides which one it means.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub custom: bool,
    /// The board's variants this device fits, in the board's order, each
    /// with its own confidence. Use the confidence of the variant you mean,
    /// such as the build's.
    pub variants: Vec<VariantMatch>,
}

impl BoardMatch {
    /// The highest confidence of the board's variants, for sorting.
    pub fn best_confidence(&self) -> u8 {
        self.variants
            .iter()
            .map(|v| v.confidence)
            .max()
            .unwrap_or(0)
    }
}

/// A variant of a board that a device fits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VariantMatch {
    /// The variant's ID within the board, such as usb-flash.
    pub id: String,
    /// How sure the engine is that the device is the board in this variant,
    /// from 1 to 3: higher is surer, and 3 is always the highest. Compare it,
    /// such as to pre-select a device at 3; how the engine rates may improve
    /// between versions. No rating proves which board is connected, and
    /// several boards can have the same one.
    #[schemars(range(min = 1, max = 3))]
    pub confidence: u8,
}

/// The highest confidence: the device reports a name or serial number
/// specific to the board.
pub const CONFIDENCE_HIGH: u8 = 3;
/// The board's name pattern matched, but the name is a USB chip's factory
/// default, which many boards report.
pub const CONFIDENCE_MEDIUM: u8 = 2;
/// Only the USB IDs matched, or the device's names can't be read.
pub const CONFIDENCE_LOW: u8 = 1;

/// Product names that USB chips report out of the factory, which many boards
/// keep. A match on one of these says little about the board.
pub const FACTORY_DEFAULT_NAMES: &[&str] = &[
    "Dual RS232-HS",            // FTDI FT2232H
    "Single RS232-HS",          // FTDI FT232H
    "Quad RS232-HS",            // FTDI FT4232H
    "USB <-> Serial Converter", // FTDI FT2232D
    "USB <-> Serial",
    "FT232R USB UART",
    "FT231X USB UART",
    "FT230X Basic UART",
    "DAPLink CMSIS-DAP",
    "CMSIS-DAP",
];

/// Whether a product name is a USB chip's factory default.
pub fn is_factory_default_name(name: &str) -> bool {
    let name = name.trim();
    FACTORY_DEFAULT_NAMES
        .iter()
        .any(|d| d.eq_ignore_ascii_case(name))
}

/// How sure a match of a device to a variant's filter is (the device
/// matches it): high for a matched serial pattern or a board-specific name,
/// medium for a matched factory default name, low for USB IDs only.
pub fn confidence(device: &Device, filter: &UsbFilter) -> u8 {
    if !device.reachable {
        return CONFIDENCE_LOW;
    }
    if filter.serial_regex.is_some() {
        return CONFIDENCE_HIGH;
    }
    match (&filter.product_regex, &device.product) {
        (Some(_), Some(name)) if is_factory_default_name(name) => CONFIDENCE_MEDIUM,
        (Some(_), Some(_)) => CONFIDENCE_HIGH,
        _ => CONFIDENCE_LOW,
    }
}

/// A device the engine sees but can't open, so it can't read its names or
/// program it: it needs a driver or permissions first. Listed only when its
/// USB IDs fit a board.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnreachableDevice {
    /// The kind of device: usb.
    pub kind: DeviceKind,
    /// USB vendor ID, four lowercase hex digits.
    pub vid: String,
    /// USB product ID, four lowercase hex digits.
    pub pid: String,
    /// USB bus number.
    pub bus: u8,
    /// USB device number on its bus.
    pub device: u8,
    /// The boards whose USB IDs it has. Every variant's confidence is the
    /// lowest, 1, since its names can't be read.
    pub boards: Vec<BoardMatch>,
    /// What makes the device usable, such as fix_device_permissions.
    pub action: Action,
    /// The confidence that it is the given build's board, always 1 since
    /// its names can't be read. Present only when list_devices was given a
    /// build_folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 3))]
    pub build_confidence: Option<u8>,
}

/// A board to match devices against: its ID, whether it came from the
/// request, and its variants.
pub struct Candidate<'a> {
    pub id: &'a str,
    pub custom: bool,
    pub variants: &'a [Variant],
}

/// Lists the connected USB devices, the ones the engine can't open
/// included, sorted by vendor, product, bus and device.
pub fn scan() -> Result<Vec<Device>, ToolError> {
    let list = rusb::devices().map_err(|e| {
        ToolError::setup(format!("Can't list the USB devices: {e}")).with_action(
            ActionCode::FixDevicePermissions,
            "Check that this program may access USB devices.",
        )
    })?;
    let timeout = Duration::from_millis(500);
    let mut out = Vec::new();
    for d in list.iter() {
        let Ok(desc) = d.device_descriptor() else {
            continue;
        };
        let (mut manufacturer, mut product, mut serial_number) = (None, None, None);
        let mut reachable = false;
        if let Ok(handle) = d.open() {
            reachable = true;
            let lang = handle
                .read_languages(timeout)
                .ok()
                .and_then(|l| l.first().copied());
            if let Some(lang) = lang {
                let clean = |s: Option<String>| {
                    s.map(|s| crate::text::to_text(&s))
                        .filter(|s| !s.is_empty())
                };
                manufacturer = clean(handle.read_manufacturer_string(lang, &desc, timeout).ok());
                product = clean(handle.read_product_string(lang, &desc, timeout).ok());
                serial_number = clean(handle.read_serial_number_string(lang, &desc, timeout).ok());
            }
        }
        // The port path stays the same when the device is plugged in again
        // in the same port; the device number doesn't.
        let location = match d.port_numbers() {
            Ok(ports) if !ports.is_empty() => format!(
                "{}-{}",
                d.bus_number(),
                ports
                    .iter()
                    .map(u8::to_string)
                    .collect::<Vec<_>>()
                    .join(".")
            ),
            _ => format!("{}-a{}", d.bus_number(), d.address()),
        };
        out.push(Device::usb(
            format!("{:04x}", desc.vendor_id()),
            format!("{:04x}", desc.product_id()),
            manufacturer,
            product,
            serial_number,
            d.bus_number(),
            d.address(),
            location,
            reachable,
        ));
    }
    out.sort_by(|a, b| (&a.vid, &a.pid, a.bus, a.device).cmp(&(&b.vid, &b.pid, b.bus, b.device)));
    Ok(out)
}

impl Device {
    /// A USB device, with its ID made from where it is and what it is.
    #[allow(clippy::too_many_arguments)]
    pub fn usb(
        vid: String,
        pid: String,
        manufacturer: Option<String>,
        product: Option<String>,
        serial_number: Option<String>,
        bus: u8,
        device: u8,
        location: String,
        reachable: bool,
    ) -> Device {
        let id = format!(
            "usb:{location}:{vid}:{pid}:{}",
            serial_number.as_deref().unwrap_or("")
        );
        Device {
            id,
            kind: DeviceKind::Usb,
            vid,
            pid,
            manufacturer,
            product,
            serial_number,
            bus,
            device,
            boards: Vec::new(),
            build_confidence: None,
            location,
            reachable,
        }
    }
}

/// Whether a device has the filter's USB IDs (case doesn't matter).
pub fn same_ids(device: &Device, filter: &UsbFilter) -> bool {
    device.vid.eq_ignore_ascii_case(&filter.vid) && device.pid.eq_ignore_ascii_case(&filter.pid)
}

/// Whether a device passes a board's USB filter: same IDs, a product name
/// matching product_regex and a serial number matching serial_regex, when
/// the filter has them.
pub fn matches(device: &Device, filter: &UsbFilter) -> bool {
    let fits = |re: &Option<String>, value: &Option<String>| match re {
        None => true,
        Some(re) => match (regex::Regex::new(re), value) {
            (Ok(re), Some(v)) => re.is_match(v),
            _ => false,
        },
    };
    same_ids(device, filter)
        && fits(&filter.product_regex, &device.product)
        && fits(&filter.serial_regex, &device.serial_number)
}

/// The boards a device may be, best first: by their best variant's
/// confidence, then catalog boards before custom ones, then by board ID.
/// Each variant has its own confidence. For a device the engine can't open,
/// only the USB IDs are compared (its names are unknown), so every match is
/// low.
pub fn board_matches(device: &Device, boards: &[Candidate]) -> Vec<BoardMatch> {
    let mut out = Vec::new();
    for b in boards {
        let mut variants = Vec::new();
        for c in b.variants {
            let hit = if device.reachable {
                matches(device, &c.usb)
            } else {
                same_ids(device, &c.usb)
            };
            if hit {
                let confidence = confidence(device, &c.usb);
                variants.push(VariantMatch {
                    id: c.id.clone(),
                    confidence,
                });
            }
        }
        if !variants.is_empty() {
            out.push(BoardMatch {
                board: b.id.to_string(),
                custom: b.custom,
                variants,
            });
        }
    }
    out.sort_by(|a, b| {
        (b.best_confidence(), a.custom, &a.board).cmp(&(a.best_confidence(), b.custom, &b.board))
    });
    out
}

/// What makes a device the engine can't open usable, on this platform.
pub fn unreachable_action() -> Action {
    if cfg!(windows) {
        Action {
            code: ActionCode::InstallDrivers,
            description: "Install the driver this device needs, then list the devices again."
                .into(),
        }
    } else {
        Action {
            code: ActionCode::FixDevicePermissions,
            description: "Allow this user to access the USB device, then list the devices again."
                .into(),
        }
    }
}

/// Finds the device a device ID names among the connected devices. The ID
/// must come from list_devices on this computer; the device must still be on
/// the same port, with the same USB IDs and serial number. Errors: a request
/// error for an ID that isn't one, a setup error with connect_board when the
/// device is gone or another one is in its place, and a setup error with
/// fix_device_permissions (or install_drivers) when it can't be opened.
pub fn find(devices: &[Device], id: &str, board_name: &str) -> Result<Device, ToolError> {
    let parts: Option<Vec<&str>> = id
        .strip_prefix("usb:")
        .map(|rest| rest.splitn(4, ':').collect());
    let Some([location, vid, pid, _serial]) = parts.as_deref() else {
        return Err(ToolError::request(format!(
            "device: {id:?} isn't a device ID. Use the id of a device from list_devices."
        )));
    };
    let gone = || {
        ToolError::setup(format!(
            "The chosen device isn't connected anymore. Is the {board_name} still plugged in?"
        ))
        .with_action(
            ActionCode::ConnectBoard,
            format!("Connect the {board_name} by USB, list the devices again and choose it."),
        )
    };
    let Some(d) = devices.iter().find(|d| d.location == *location) else {
        return Err(gone());
    };
    if d.vid != *vid || d.pid != *pid || d.id != id {
        return Err(gone());
    }
    if !d.reachable {
        let action = unreachable_action();
        return Err(
            ToolError::setup(format!("The {board_name} was found but can't be opened."))
                .with_action(action.code, action.description),
        );
    }
    Ok(d.clone())
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn device(
        vid: &str,
        pid: &str,
        product: Option<&str>,
        serial: Option<&str>,
        dev: u8,
    ) -> Device {
        Device::usb(
            vid.into(),
            pid.into(),
            None,
            product.map(Into::into),
            serial.map(Into::into),
            1,
            dev,
            format!("1-{dev}"),
            true,
        )
    }

    fn filter(re: Option<&str>) -> UsbFilter {
        UsbFilter {
            vid: "0403".into(),
            pid: "6010".into(),
            product_regex: re.map(Into::into),
            serial_regex: None,
        }
    }

    #[test]
    fn matching() {
        let alhambra = device("0403", "6010", Some("Alhambra II v2.0"), Some("A1"), 3);
        let icestick = device("0403", "6010", Some("Dual RS232-HS"), None, 4);
        let f = filter(Some("^Alhambra II.*"));
        assert!(matches(&alhambra, &f));
        assert!(!matches(&icestick, &f));
        assert!(matches(&icestick, &filter(None)));
        assert!(matches(
            &device("0403", "6010", None, None, 5),
            &filter(None)
        ));
        assert!(
            !matches(&device("0403", "6010", None, None, 5), &f),
            "no name, no match"
        );
        assert!(!matches(
            &device("0403", "6014", Some("Alhambra II"), None, 6),
            &f
        ));
        // iCESugar boards share IDs and product name; the serial tells them apart.
        let nano_filter = UsbFilter {
            vid: "1d50".into(),
            pid: "602b".into(),
            product_regex: Some("^DAPLink CMSIS-DAP".into()),
            serial_regex: Some("^0720".into()),
        };
        let pro = device(
            "1d50",
            "602b",
            Some("DAPLink CMSIS-DAP"),
            Some("0710000100380027"),
            7,
        );
        let nano = device(
            "1d50",
            "602b",
            Some("DAPLink CMSIS-DAP"),
            Some("0720000100380027"),
            8,
        );
        let no_serial = device("1d50", "602b", Some("DAPLink CMSIS-DAP"), None, 9);
        assert!(matches(&nano, &nano_filter));
        assert!(!matches(&pro, &nano_filter));
        assert!(!matches(&no_serial, &nano_filter), "no serial, no match");
    }

    #[test]
    fn ids_name_where_and_what_the_device_is() {
        let d = device("0403", "6010", Some("Alhambra II"), Some("A:1"), 3);
        assert_eq!(d.id, "usb:1-3:0403:6010:A:1");
        assert_eq!(
            device("0403", "6010", None, None, 3).id,
            "usb:1-3:0403:6010:"
        );
    }

    #[test]
    fn finding_a_device_by_id() {
        let a = device("0403", "6010", Some("Alhambra II"), Some("A1"), 3);
        let b = device("0403", "6010", Some("Alhambra II"), Some("B2"), 4);
        let list = [a.clone(), b.clone()];
        assert_eq!(find(&list, &b.id, "Alhambra II").unwrap(), b);

        // Plugged in again in the same port: a new device number, same ID.
        let mut again = a.clone();
        again.device = 9;
        assert_eq!(find(&[again.clone()], &a.id, "Alhambra II").unwrap(), again);

        // Gone, or another device in its port.
        let err = find(std::slice::from_ref(&b), &a.id, "Alhambra II").unwrap_err();
        assert_eq!(err.action.unwrap().code, ActionCode::ConnectBoard);
        let mut other = device("0403", "6010", Some("Alhambra II"), Some("C3"), 3);
        other.location = a.location.clone();
        let err = find(&[other], &a.id, "Alhambra II").unwrap_err();
        assert_eq!(err.action.unwrap().code, ActionCode::ConnectBoard);

        // Not an ID.
        for bad in ["", "1-3", "usb:1-3", "usb:1-3:0403"] {
            let err = find(&list, bad, "Alhambra II").unwrap_err();
            assert_eq!(err.category, crate::errors::Category::Request, "{bad}");
        }

        // Can't be opened.
        let mut locked = a.clone();
        locked.reachable = false;
        let err = find(&[locked], &a.id, "Alhambra II").unwrap_err();
        assert_eq!(err.category, crate::errors::Category::Setup);
        assert_eq!(err.action.unwrap().code, unreachable_action().code);
    }

    #[test]
    fn matching_boards_with_confidence() {
        let catalog = crate::catalog::builtin();
        let boards: Vec<Candidate> = catalog
            .boards
            .iter()
            .map(|(id, b)| Candidate {
                id,
                custom: false,
                variants: &b.variants,
            })
            .collect();
        let ids = |m: &[BoardMatch]| m.iter().map(|m| m.board.clone()).collect::<Vec<_>>();

        let alhambra = device("0403", "6010", Some("Alhambra II v1.0A"), Some("FT1"), 3);
        let m = board_matches(&alhambra, &boards);
        assert_eq!(m[0].board, "alhambra-ii");
        assert_eq!(m[0].best_confidence(), CONFIDENCE_HIGH);
        assert!(m[1..].iter().all(|m| m.best_confidence() == CONFIDENCE_LOW));
        assert!(
            ids(&m).contains(&"alchitry-au".to_string()),
            "a generic FTDI filter"
        );
        assert!(!ids(&m).contains(&"icezum".to_string()), "another name");

        // Both iCESugar-Pro variants, and not the other iCESugar boards.
        let pro = device(
            "1d50",
            "602b",
            Some("DAPLink CMSIS-DAP"),
            Some("0710000100380027"),
            7,
        );
        let m = board_matches(&pro, &boards);
        assert_eq!(ids(&m), ["icesugar-pro"]);
        assert_eq!(m[0].variants.len(), 2);
        assert_eq!(m[0].best_confidence(), CONFIDENCE_HIGH, "a serial pattern");

        // A factory default name: medium for the boards that expect it,
        // before the low ones; a board-specific name in the same pattern
        // is high.
        let ftdi = device("0403", "6010", Some("Dual RS232-HS"), None, 4);
        let m = board_matches(&ftdi, &boards);
        let go = m.iter().find(|m| m.board == "go-board").unwrap();
        assert_eq!(go.best_confidence(), CONFIDENCE_MEDIUM);
        assert_eq!(m[0].best_confidence(), CONFIDENCE_MEDIUM);
        assert_eq!(m.last().unwrap().best_confidence(), CONFIDENCE_LOW);
        let icebreaker = device("0403", "6010", Some("iCEBreaker V1.0e"), None, 5);
        let m = board_matches(&icebreaker, &boards);
        assert_eq!(
            (m[0].board.as_str(), m[0].best_confidence()),
            ("icebreaker", CONFIDENCE_HIGH)
        );

        // Can't be opened: every board with these USB IDs, as chip.
        let mut locked = pro.clone();
        locked.reachable = false;
        let m = board_matches(&locked, &boards);
        assert_eq!(ids(&m), ["icesugar-1-5", "icesugar-nano", "icesugar-pro"]);
        assert!(m.iter().all(|m| m.best_confidence() == CONFIDENCE_LOW));

        assert!(board_matches(&device("1234", "5678", None, None, 1), &boards).is_empty());
    }

    #[test]
    fn factory_default_names() {
        assert!(is_factory_default_name("Dual RS232-HS"));
        assert!(is_factory_default_name(" dual rs232-hs "));
        assert!(!is_factory_default_name("Alhambra II v1.0A"));
        assert!(!is_factory_default_name("Dual RS232-HS v2"));
    }

    #[test]
    fn scanning_works_here() {
        // Just checks libusb starts; the list depends on the machine.
        let devices = scan().unwrap();
        for d in &devices {
            assert_eq!(d.vid.len(), 4);
            assert!(d.id.starts_with("usb:"));
        }
    }
}
