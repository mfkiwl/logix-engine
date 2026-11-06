//! Board files (docs/requirements.md, Board files): a user's own board, made
//! by the engine from a connected device and the catalog board the user says
//! it is. Frontends store a board file in the project and send it back
//! unchanged; only the engine reads it. A file holds everything to build and
//! program the board, so any engine version reads it the same way: it never
//! refers to the catalog for anything but information.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::catalog::{self, FpgaDefinition, UsbFilter, Variant};
use crate::devices::Device;
use crate::errors::ToolError;

/// The format version this engine writes. It reads this one and every
/// older one.
pub const FORMAT: u32 = 1;

/// A board file's contents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardFile {
    /// The format version, which also marks the object as a board file.
    pub logix_board_file: u32,
    /// The board's ID in requests and results, such as arty-a7-100t-3f9a2c.
    pub id: String,
    /// The board's name for people.
    pub name: String,
    /// The FPGA's catalog ID when the file was made, for grouping boards by
    /// family. The FPGA itself is in fpga.
    pub fpga_id: String,
    /// The board's FPGA, in full.
    pub fpga: FpgaDefinition,
    /// The ways to program the board, the default first, each with the
    /// device filter the engine made.
    pub variants: Vec<Variant>,
    /// Where the file came from. Information only: never used to fill in
    /// anything.
    pub made_from: MadeFrom,
    /// The variants a successful upload has confirmed, one entry each.
    #[serde(default)]
    pub verified: Vec<Verified>,
}

/// Where a board file came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MadeFrom {
    /// The catalog board the user said the device is.
    pub board: String,
    /// The engine version that made the file.
    pub engine: String,
    /// The device's signature.
    pub device: Signature,
}

/// What a device reports, as kept in a board file and its reports. The
/// serial number is kept in full only in the filter, and only when the user
/// pinned the board by it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signature {
    pub vid: String,
    pub pid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    /// The serial number's shape (see serial_shape), when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial_shape: Option<String>,
}

/// A variant a successful upload confirmed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verified {
    /// The variant's ID.
    pub variant: String,
    /// The engine version that programmed it.
    pub engine: String,
    /// The FPGA's ID code the upload read, on boards programmed over JTAG,
    /// such as 0x0362d093.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fpga_idcode: Option<String>,
}

/// A serial number's shape: its first characters (at most 6, at most half
/// of it), the rest as #. Enough to see a vendor's product code or a fixed
/// serial shared by every unit, not enough to name one device.
pub fn serial_shape(serial: &str) -> String {
    let n = serial.chars().count();
    let keep = (n / 2).min(6);
    serial
        .chars()
        .enumerate()
        .map(|(i, c)| if i < keep { c } else { '#' })
        .collect()
}

/// What make returns: the file, and notes for the user.
#[derive(Debug)]
pub struct Made {
    pub file: BoardFile,
    pub notes: Vec<String>,
}

/// Makes a board file from a connected device and the catalog board the user
/// says it is. Only the board's variants that use the device's USB IDs are
/// kept, each with a filter for this device: its exact product name and
/// release, and its exact serial number when pin_serial_number is set.
/// `connected` is every connected device, to tell whether a serial number
/// tells this board apart.
pub fn make(
    device: &Device,
    connected: &[Device],
    board_id: &str,
    name: Option<&str>,
    pin_serial_number: bool,
) -> Result<Made, ToolError> {
    let c = catalog::builtin();
    let base = c.boards.get(board_id).ok_or_else(|| {
        ToolError::request(format!(
            "{board_id:?} isn't a board in the catalog. Use get_catalog to list the boards."
        ))
    })?;
    let fpga = c.fpgas.get(&base.fpga_id).cloned().ok_or_else(|| {
        ToolError::engine(format!("The catalog's FPGA {:?} is missing.", base.fpga_id))
    })?;
    let mut notes = Vec::new();
    let serial_regex = if pin_serial_number {
        let Some(serial) = device.serial_number.as_deref() else {
            return Err(ToolError::request(
                "This device reports no serial number, so the board can't be told apart by it. Make the board file without pin_serial_number.",
            ));
        };
        let twins = connected
            .iter()
            .filter(|d| {
                d.location != device.location
                    && d.vid == device.vid
                    && d.pid == device.pid
                    && d.serial_number.as_deref() == Some(serial)
            })
            .count();
        if twins > 0 {
            notes.push(format!(
                "Another connected device has the same serial number, {serial:?}, so the serial number doesn't tell these boards apart: choose the device when uploading."
            ));
        }
        notes.push(
            "The board file holds this board's serial number, to tell it from identical boards. Keep that in mind before publishing the project; reports show only its shape."
                .into(),
        );
        Some(format!("^{}$", regex::escape(serial)))
    } else {
        None
    };
    let filter = UsbFilter {
        vid: device.vid.clone(),
        pid: device.pid.clone(),
        product_regex: device
            .product
            .as_deref()
            .map(|p| format!("^{}$", regex::escape(p))),
        serial_regex,
        release: device.release.clone(),
    };
    if filter.product_regex.is_none() {
        notes.push(
            "The device reports no product name, so the board file matches any device with its USB IDs and release."
                .into(),
        );
    }
    let variants: Vec<Variant> = base
        .variants
        .iter()
        .filter(|v| v.usb.vid == device.vid && v.usb.pid == device.pid)
        .map(|v| Variant {
            usb: filter.clone(),
            maintainer_notes: None,
            ..v.clone()
        })
        .collect();
    if variants.is_empty() {
        return Err(ToolError::request(format!(
            "The {} isn't programmed through a device with this one's USB IDs ({}:{}), so this device can't be that board. Choose another board.",
            base.description, device.vid, device.pid
        )));
    }
    let signature = Signature {
        vid: device.vid.clone(),
        pid: device.pid.clone(),
        release: device.release.clone(),
        manufacturer: device.manufacturer.clone(),
        product: device.product.clone(),
        serial_shape: device.serial_number.as_deref().map(serial_shape),
    };
    // The same board and device make the same ID, so making the file again
    // doesn't make a second board.
    let key = serde_json::to_string(&(&filter, board_id)).unwrap_or_default();
    let digest = hex::encode(Sha256::digest(key.as_bytes()));
    let file = BoardFile {
        logix_board_file: FORMAT,
        id: format!("{board_id}-{}", &digest[..6]),
        name: name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(String::from)
            .unwrap_or_else(|| format!("{} (my board)", base.description)),
        fpga_id: base.fpga_id.clone(),
        fpga,
        variants,
        made_from: MadeFrom {
            board: board_id.into(),
            engine: crate::state::ENGINE_VERSION.into(),
            device: signature,
        },
        verified: Vec::new(),
    };
    file.validate().map_err(ToolError::engine)?;
    Ok(Made { file, notes })
}

/// Reads a request's board_files, refusing bad ones with the field's name.
/// Files with the same ID are the same board: only the first is kept.
pub fn read_all(files: &[serde_json::Value]) -> Result<Vec<BoardFile>, ToolError> {
    let mut out: Vec<BoardFile> = Vec::new();
    for (i, v) in files.iter().enumerate() {
        let f = BoardFile::from_value(v).map_err(|mut e| {
            e.message = format!("board_files[{i}]: {}", e.message);
            e
        })?;
        if !out.iter().any(|o| o.id == f.id) {
            out.push(f);
        }
    }
    Ok(out)
}

impl BoardFile {
    /// Reads a board file from a request. Errors name what's wrong, for the
    /// user or the frontend's developer.
    pub fn from_value(value: &serde_json::Value) -> Result<BoardFile, ToolError> {
        let version = value
            .get("logix_board_file")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| {
                ToolError::request("This isn't a board file: make one with make_board_file.")
            })?;
        if version > FORMAT as u64 {
            return Err(ToolError::request(format!(
                "This board file was made by a newer Logix Engine (format {version}; this engine reads up to {FORMAT}). Update the engine, or make the board file again."
            )));
        }
        let file: BoardFile = serde_json::from_value(value.clone()).map_err(|e| {
            ToolError::request(format!("The board file is damaged: {e}. Make it again."))
        })?;
        file.validate().map_err(|e| {
            ToolError::request(format!("The board file is damaged: {e}. Make it again."))
        })?;
        Ok(file)
    }

    /// Records a successful upload with a variant: once per variant, with
    /// the FPGA ID code the upload confirmed, when it did. No dates or
    /// counts, which would tell when the user works if the project is
    /// published.
    pub fn record_upload(&mut self, variant: &str, fpga_idcode: Option<&str>) {
        let engine = crate::state::ENGINE_VERSION.to_string();
        match self.verified.iter_mut().find(|v| v.variant == variant) {
            Some(v) => {
                if v.fpga_idcode.is_none() && fpga_idcode.is_some() {
                    v.fpga_idcode = fpga_idcode.map(String::from);
                    v.engine = engine;
                }
            }
            None => self.verified.push(Verified {
                variant: variant.into(),
                engine,
                fpga_idcode: fpga_idcode.map(String::from),
            }),
        }
    }

    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }

    fn validate(&self) -> Result<(), String> {
        if !catalog::is_catalog_id(&self.id) {
            return Err(format!("{:?} isn't a board ID", self.id));
        }
        if self.name.trim().is_empty() {
            return Err("the board has no name".into());
        }
        self.fpga.validate()?;
        catalog::validate_variants(&self.variants)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::tests::device;

    fn arty() -> Device {
        let mut d = device(
            "0403",
            "6010",
            Some("Digilent USB Device"),
            Some("210319AB51BBA"),
            4,
        );
        d.manufacturer = Some("Digilent".into());
        d.release = Some("0700".into());
        d
    }

    #[test]
    fn serial_shapes_keep_only_a_prefix() {
        assert_eq!(serial_shape("210319AB51BBA"), "210319#######");
        assert_eq!(serial_shape("FactoryAIOT Pro"), "Factor#########");
        assert_eq!(serial_shape("FTATANKC"), "FTAT####");
        assert_eq!(serial_shape("A1"), "A#");
        assert_eq!(serial_shape(""), "");
    }

    #[test]
    fn a_board_file_from_a_device() {
        let d = arty();
        let made = make(&d, std::slice::from_ref(&d), "arty-a7-100t", None, false).unwrap();
        let f = &made.file;
        assert_eq!(f.logix_board_file, FORMAT);
        assert!(f.id.starts_with("arty-a7-100t-"), "{}", f.id);
        assert_eq!(f.name, "Arty A7-100T (Xilinx, Artix7) (my board)");
        assert_eq!(f.fpga_id, "xc7a100tcsg324-1");
        assert_eq!(f.variants.len(), 2, "flash and RAM");
        let usb = &f.variants[0].usb;
        assert_eq!(usb.product_regex.as_deref(), Some("^Digilent USB Device$"));
        assert_eq!(usb.release.as_deref(), Some("0700"));
        assert_eq!(usb.serial_regex, None);
        assert!(f.variants.iter().all(|v| v.maintainer_notes.is_none()));
        assert_eq!(f.made_from.board, "arty-a7-100t");
        assert_eq!(
            f.made_from.device.serial_shape.as_deref(),
            Some("210319#######")
        );
        assert!(made.notes.is_empty());
        // Nothing in the file names this exact device.
        let text = f.to_value().to_string();
        assert!(!text.contains("210319AB51BBA"), "{text}");

        // The same board and device: the same ID. A name of the user's.
        let again = make(&d, &[], "arty-a7-100t", Some(" My Arty "), false).unwrap();
        assert_eq!(again.file.id, f.id);
        assert_eq!(again.file.name, "My Arty");

        // The file reads back as it was written.
        assert_eq!(&BoardFile::from_value(&f.to_value()).unwrap(), f);
    }

    #[test]
    fn pinned_by_serial_number() {
        let d = arty();
        let made = make(&d, std::slice::from_ref(&d), "arty-a7-100t", None, true).unwrap();
        let usb = &made.file.variants[0].usb;
        assert_eq!(usb.serial_regex.as_deref(), Some("^210319AB51BBA$"));
        assert_eq!(made.notes.len(), 1, "{:?}", made.notes);
        assert!(made.notes[0].contains("publishing"), "{:?}", made.notes);
        let unpinned = make(&d, &[], "arty-a7-100t", None, false).unwrap();
        assert_ne!(made.file.id, unpinned.file.id, "another board");

        // A twin with the same serial number: said so.
        let mut twin = arty();
        twin.location = "1-9".into();
        let made = make(&d, &[d.clone(), twin], "arty-a7-100t", None, true).unwrap();
        assert!(
            made.notes[0].contains("same serial number"),
            "{:?}",
            made.notes
        );

        // No serial number to pin.
        let mut bare = arty();
        bare.serial_number = None;
        assert!(make(&bare, &[], "arty-a7-100t", None, true).is_err());
    }

    #[test]
    fn only_variants_through_this_device() {
        // ColorLight i9 v7.2: FT2232H, FT232H, USB-Blaster and CMSIS-DAP
        // adapters; an FT232H keeps only its own two variants.
        let ft232h = device("0403", "6014", Some("Single RS232-HS"), None, 3);
        let made = make(&ft232h, &[], "colorlight-i9-v7-2", None, false).unwrap();
        let ids: Vec<_> = made.file.variants.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["ft232h-flash", "ft232h-ram"]);

        // A device that can't be the board.
        let mouse = device("1234", "5678", Some("A mouse"), None, 5);
        let e = make(&mouse, &[], "arty-a7-100t", None, false).unwrap_err();
        assert!(e.message.contains("Choose another board"), "{}", e.message);
        assert!(make(&ft232h, &[], "nope", None, false).is_err());
    }

    #[test]
    fn uploads_are_recorded_once_per_variant() {
        let mut f = make(&arty(), &[], "arty-a7-100t", None, false)
            .unwrap()
            .file;
        f.record_upload("usb-ram", None);
        f.record_upload("usb-ram", None);
        assert_eq!(f.verified.len(), 1);
        assert_eq!(f.verified[0].fpga_idcode, None);
        f.record_upload("usb-ram", Some("0x03631093"));
        f.record_upload("usb-flash", Some("0x03631093"));
        let v: Vec<_> = f
            .verified
            .iter()
            .map(|v| (v.variant.as_str(), v.fpga_idcode.as_deref()))
            .collect();
        assert_eq!(
            v,
            [
                ("usb-ram", Some("0x03631093")),
                ("usb-flash", Some("0x03631093"))
            ]
        );
        assert_eq!(BoardFile::from_value(&f.to_value()).unwrap(), f);
    }

    #[test]
    fn bad_files_are_refused() {
        let d = arty();
        let good = make(&d, &[], "arty-a7-100t", None, false)
            .unwrap()
            .file
            .to_value();
        assert!(BoardFile::from_value(&serde_json::json!({"description": "x"})).is_err());
        let mut newer = good.clone();
        newer["logix_board_file"] = (FORMAT + 1).into();
        let e = BoardFile::from_value(&newer).unwrap_err();
        assert!(e.message.contains("newer"), "{}", e.message);
        let mut damaged = good.clone();
        damaged["variants"] = serde_json::json!([]);
        assert!(BoardFile::from_value(&damaged).is_err());
        let mut extra = good;
        extra["checksum"] = "x".into();
        assert!(BoardFile::from_value(&extra).is_err());
    }
}
