//! The read-only catalog of boards and FPGAs (docs/tool-api.md). It is data,
//! not code: catalog/boards.json and catalog/fpgas.json, imported from Apio's
//! definitions by scripts/import-apio-catalog.py and built into the engine.
//! The catalog format is part of the tool API.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The programmers the engine can run, by catalog ID.
pub const PROGRAMMERS: &[&str] = &["openfpgaloader", "iceprog", "dfu-util", "icesprog"];

/// The variables a programming variant's argument template may use.
pub const TEMPLATE_VARIABLES: &[&str] = &["VID", "PID", "BUS", "DEV", "SERIAL", "BITSTREAM"];

/// A board: its FPGA and its variants. A board is a name: two boards with
/// different names are unrelated, even when their hardware is identical, as
/// are the sizes of a board sold with several FPGA sizes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoardDefinition {
    /// The board's name for people, such as "Alhambra II".
    pub description: String,
    /// The board's FPGA, as a catalog ID of the fpgas section, such as
    /// ice40hx4k-tq144-8k. Every configuration uses it.
    #[schemars(pattern(r"^[a-z0-9-]+$"))]
    pub fpga_id: String,
    /// The board's choices, such as the cable or the memory to program, and
    /// how to program it with each combination that exists.
    pub variants: Variants,
}

/// A board's choices (dimensions) and the combinations that exist
/// (configurations). They differ only in how the board is programmed: every
/// configuration has the same FPGA and pin file, so the same project works.
/// Every board has the two standard dimensions, connection and memory
/// (`STANDARD_DIMENSIONS`), whose values and their meanings the engine
/// defines; hosts decide how to show them (e.g. a choice only where a
/// dimension has more than one value).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Variants {
    /// The standard dimensions, connection then memory, each with the values
    /// the board's configurations use, with labels and help from the
    /// engine. Derived from the configurations: ignored in inputs and not
    /// written in the catalog file.
    #[serde(default)]
    pub dimensions: Vec<Dimension>,
    /// The combinations that exist, each with how to program the board. The
    /// first is the board's default. Combinations that aren't listed don't
    /// exist.
    #[schemars(length(min = 1))]
    pub configurations: Vec<Configuration>,
}

/// One kind of choice, such as the memory to program.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Dimension {
    /// The dimension's ID, used in configurations and variant inputs, such as
    /// connection or memory.
    #[schemars(pattern(r"^[a-z0-9-]+$"))]
    pub id: String,
    /// The dimension's name for people, such as "Connection".
    pub label: String,
    /// The possible values, in the order to show them.
    #[schemars(length(min = 1))]
    pub values: Vec<DimensionValue>,
}

/// One value of a dimension, such as RAM.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DimensionValue {
    /// The value's ID, such as ram.
    #[schemars(pattern(r"^[a-z0-9-]+$"))]
    pub id: String,
    /// The value's name for people, such as "RAM".
    pub label: String,
    /// A short explanation for people, such as "Faster to upload; lost when
    /// the board is powered off."
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

/// One combination of the board's dimension values, and how to program the
/// board with it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    /// The value of every dimension, by dimension ID, such as
    /// {"memory": "ram"}. Omitted or empty for a board without dimensions.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub when: BTreeMap<String, String>,
    /// The programmer to run: openfpgaloader, iceprog, dfu-util or icesprog.
    pub programmer: String,
    /// The programmer's arguments, as a template. The engine replaces
    /// ${VID}, ${PID}, ${BUS} and ${DEV} with the device's USB vendor ID,
    /// product ID, bus and device number, ${SERIAL} with its serial number,
    /// and ${BITSTREAM} with the bitstream path.
    pub args: String,
    /// The USB filter that finds the board's device.
    pub usb: UsbFilter,
    /// Whether programming this way is known to work: tested on the board,
    /// or used unchanged in a tool that relies on it. False for ways that
    /// haven't been tried on hardware, such as a board moved to a different
    /// programmer.
    pub tested: bool,
    /// Warnings and missing information for users, such as a USB filter that
    /// also matches other boards or a step needed before programming.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_notes: Option<String>,
    /// For the catalog's maintainers: where the settings come from and why
    /// they changed. Kept in catalog/boards.json only; get_catalog never
    /// returns it.
    #[serde(default, skip_serializing)]
    #[schemars(skip)]
    pub maintainer_notes: Option<String>,
}

/// Which USB devices are this board.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UsbFilter {
    /// USB vendor ID, four lowercase hex digits, such as 0403.
    #[schemars(pattern(r"^[0-9a-f]{4}$"))]
    pub vid: String,
    /// USB product ID, four lowercase hex digits, such as 6010.
    #[schemars(pattern(r"^[0-9a-f]{4}$"))]
    pub pid: String,
    /// A regular expression the device's product name must match, when
    /// several boards share the same USB IDs. Omitted means any product name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_regex: Option<String>,
    /// A regular expression the device's serial number must match, for
    /// boards that share USB IDs and product names but have serial numbers
    /// that tell them apart. Omitted means any serial number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial_regex: Option<String>,
}

/// An FPGA: its architecture, part and toolchain parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FpgaDefinition {
    /// The FPGA family: ice40, ecp5, gowin or xilinx.
    pub architecture: Architecture,
    /// The vendor's part number, such as ICE40HX4K-TQ144.
    pub part_number: String,
    /// The device size, such as 8k.
    pub size: String,
    /// The device nextpnr uses, such as hx8k for iCE40, 25k for ECP5,
    /// GW1NR-LV9QN88PC6/I5 for Gowin or xc7a35tcsg324-1 for Xilinx.
    pub nextpnr_device: String,
    /// The package nextpnr uses, such as tq144:4k for iCE40 or CABGA256 for
    /// ECP5. Required for iCE40 and ECP5; Gowin and Xilinx have none,
    /// since their nextpnr_device names the package.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nextpnr_package: Option<String>,
    /// The speed grade nextpnr uses, such as 6. ECP5 only, where it's
    /// required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nextpnr_speed: Option<String>,
    /// The device family nextpnr uses, such as GW1N-9C, when nextpnr can't
    /// tell it from nextpnr_device. Gowin only, optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nextpnr_family: Option<String>,
    /// The device the bitstream packer (gowin_pack) uses, such as GW1N-9C.
    /// Gowin only, where it's required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packer_device: Option<String>,
    /// The family yosys synthesizes for, such as gw2a. Gowin only,
    /// optional: omitted means yosys's default, the GW1N family.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yosys_family: Option<String>,
    /// The architecture yosys synthesizes for (synth_xilinx -arch), such
    /// as xc7. Xilinx only, where it's required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yosys_arch: Option<String>,
    /// The FPGA family's folder in the Project X-Ray database, which the
    /// bitstream tools read, such as artix7, spartan7 or zynq7. Xilinx
    /// only, where it's required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prjxray_family: Option<String>,
}

impl FpgaDefinition {
    /// Checks that the FPGA has the fields its architecture needs, and only
    /// those.
    pub fn validate(&self) -> Result<(), String> {
        let word = |name: &str, v: &str| {
            if v.is_empty()
                || !v
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b":-_/".contains(&c))
            {
                Err(format!("bad {name} {v:?}"))
            } else {
                Ok(())
            }
        };
        word("nextpnr_device", &self.nextpnr_device)?;
        let arch = format!("{:?}", self.architecture).to_lowercase();
        let fields = [
            ("nextpnr_package", &self.nextpnr_package),
            ("nextpnr_speed", &self.nextpnr_speed),
            ("nextpnr_family", &self.nextpnr_family),
            ("packer_device", &self.packer_device),
            ("yosys_family", &self.yosys_family),
            ("yosys_arch", &self.yosys_arch),
            ("prjxray_family", &self.prjxray_family),
        ];
        for (name, value) in fields {
            let (allowed, required) = match (self.architecture, name) {
                (Architecture::Ice40, "nextpnr_package") => (true, true),
                (Architecture::Ecp5, "nextpnr_package" | "nextpnr_speed") => (true, true),
                (Architecture::Gowin, "packer_device") => (true, true),
                (Architecture::Gowin, "nextpnr_family" | "yosys_family") => (true, false),
                (Architecture::Xilinx, "yosys_arch" | "prjxray_family") => (true, true),
                _ => (false, false),
            };
            match value {
                Some(v) if allowed => word(name, v)?,
                Some(_) => return Err(format!("{arch} FPGAs have no {name}")),
                None if required => return Err(format!("{arch} FPGAs need {name}")),
                None => {}
            }
        }
        // These name folders in the Project X-Ray database.
        if self.architecture == Architecture::Xilinx {
            for (name, v) in [
                ("nextpnr_device", Some(&self.nextpnr_device)),
                ("prjxray_family", self.prjxray_family.as_ref()),
            ] {
                if let Some(v) = v
                    && v.contains('/')
                {
                    return Err(format!("bad {name} {v:?}"));
                }
            }
        }
        Ok(())
    }

    /// The nextpnr package, empty when there is none.
    pub fn package(&self) -> &str {
        self.nextpnr_package.as_deref().unwrap_or_default()
    }
}

/// FPGA families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Architecture {
    /// Lattice iCE40.
    Ice40,
    /// Lattice ECP5.
    Ecp5,
    /// Gowin (GW1N, GW2A and GW5A).
    Gowin,
    /// AMD/Xilinx 7 Series (Artix-7, Spartan-7 and Zynq-7000).
    Xilinx,
}

impl Architecture {
    /// The family's name for people, such as "iCE40".
    pub fn label(self) -> &'static str {
        match self {
            Architecture::Ice40 => "iCE40",
            Architecture::Ecp5 => "ECP5",
            Architecture::Gowin => "Gowin",
            Architecture::Xilinx => "Xilinx",
        }
    }

    /// The extension of the family's constraint file format, without the dot.
    /// Each family's place-and-route tool reads only its own format, and
    /// takes the format from its command-line option, not the file name.
    pub fn constraint_extension(self) -> &'static str {
        match self {
            Architecture::Ice40 => "pcf",
            Architecture::Ecp5 => "lpf",
            Architecture::Gowin => "cst",
            Architecture::Xilinx => "xdc",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BoardsFile {
    #[allow(dead_code)]
    source: String,
    boards: BTreeMap<String, BoardDefinition>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FpgasFile {
    #[allow(dead_code)]
    source: String,
    fpgas: BTreeMap<String, FpgaDefinition>,
}

#[derive(Debug, Clone)]
pub struct Catalog {
    pub boards: BTreeMap<String, BoardDefinition>,
    pub fpgas: BTreeMap<String, FpgaDefinition>,
}

/// Lowercase letters, digits and hyphens.
pub fn is_catalog_id(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

fn is_usb_id(s: &str) -> bool {
    s.len() == 4 && s.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
}

/// Checks an argument template: only known ${...} variables, and
/// ${BITSTREAM} present.
pub fn check_template(args: &str) -> Result<(), String> {
    let mut rest = args;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        let end = after
            .find('}')
            .ok_or_else(|| format!("unclosed ${{ in {args:?}"))?;
        let name = &after[..end];
        if !TEMPLATE_VARIABLES.contains(&name) {
            return Err(format!(
                "unknown variable ${{{name}}} in {args:?}. Known: {}",
                TEMPLATE_VARIABLES
                    .iter()
                    .map(|v| format!("${{{v}}}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        rest = &after[end + 1..];
    }
    if !args.contains("${BITSTREAM}") {
        return Err(format!("{args:?} doesn't use ${{BITSTREAM}}"));
    }
    Ok(())
}

impl Configuration {
    /// Checks the programming fields.
    pub fn validate_programming(&self) -> Result<(), String> {
        if !PROGRAMMERS.contains(&self.programmer.as_str()) {
            return Err(format!(
                "unknown programmer {:?}. Known: {}",
                self.programmer,
                PROGRAMMERS.join(", ")
            ));
        }
        check_template(&self.args)?;
        if !is_usb_id(&self.usb.vid) || !is_usb_id(&self.usb.pid) {
            return Err("USB IDs must be four lowercase hex digits".into());
        }
        if let Some(re) = &self.usb.product_regex {
            regex::Regex::new(re).map_err(|e| format!("bad product_regex: {e}"))?;
        }
        if let Some(re) = &self.usb.serial_regex {
            regex::Regex::new(re).map_err(|e| format!("bad serial_regex: {e}"))?;
        }
        Ok(())
    }

    /// The configuration's values as people read them, such as
    /// "FT232H adapter, RAM", in the dimensions' order. Empty without
    /// dimensions.
    pub fn labels(&self, dimensions: &[Dimension]) -> Vec<String> {
        dimensions
            .iter()
            .filter_map(|d| {
                let v = self.when.get(&d.id)?;
                d.values
                    .iter()
                    .find(|x| &x.id == v)
                    .map(|x| x.label.clone())
            })
            .collect()
    }
}

/// A standard value: its ID, its label for people and a short explanation.
pub type StandardValue = (&'static str, &'static str, &'static str);

/// How the computer reaches the board.
pub const CONNECTIONS: &[StandardValue] = &[
    (
        "usb",
        "USB",
        "The board's own USB port, through the programmer chip on the board.",
    ),
    (
        "bootloader",
        "USB bootloader",
        "The board's own USB port, with the board in its bootloader mode.",
    ),
    (
        "ft2232h",
        "FT2232H adapter",
        "A USB adapter with an FTDI FT2232H chip, connected to the board's JTAG pins.",
    ),
    (
        "ft232h",
        "FT232H adapter",
        "A USB adapter with an FTDI FT232H chip, connected to the board's JTAG pins.",
    ),
    (
        "cmsis-dap",
        "CMSIS-DAP adapter",
        "A CMSIS-DAP (DAPLink) USB adapter, connected to the board's JTAG pins.",
    ),
    (
        "usb-blaster",
        "USB-Blaster",
        "An Altera USB-Blaster or compatible adapter, connected to the board's JTAG pins.",
    ),
    (
        "unspecified",
        "USB",
        "Programmed through USB; see the board's documentation for how to connect it.",
    ),
];

/// Where the design goes.
pub const MEMORIES: &[StandardValue] = &[
    ("flash", "Flash", "Kept when the board is powered off."),
    (
        "ram",
        "RAM",
        "Faster to upload; lost when the board is powered off.",
    ),
];

/// The standard dimensions, in order: ID, label and values.
pub const STANDARD_DIMENSIONS: &[(&str, &str, &[StandardValue])] = &[
    ("connection", "Connection", CONNECTIONS),
    ("memory", "Memory", MEMORIES),
];

impl Variants {
    /// Fills `dimensions` from the configurations: each standard dimension,
    /// with the values the configurations use, in the standard order.
    pub fn derive_dimensions(&mut self) {
        self.dimensions = STANDARD_DIMENSIONS
            .iter()
            .map(|(id, label, values)| Dimension {
                id: id.to_string(),
                label: label.to_string(),
                values: values
                    .iter()
                    .filter(|(v, _, _)| {
                        self.configurations
                            .iter()
                            .any(|c| c.when.get(*id).map(String::as_str) == Some(*v))
                    })
                    .map(|(v, l, h)| DimensionValue {
                        id: v.to_string(),
                        label: l.to_string(),
                        help: Some(h.to_string()),
                    })
                    .collect(),
            })
            .collect();
    }

    /// Checks the configurations: each gives a standard value for every
    /// standard dimension and no other, no two give the same values, and
    /// there is at least one.
    pub fn validate(&self) -> Result<(), String> {
        if self.configurations.is_empty() {
            return Err("no configurations".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for c in &self.configurations {
            let ctx = |e: String| format!("configuration {:?}: {e}", c.when);
            if c.when.len() != STANDARD_DIMENSIONS.len() {
                return Err(ctx("needs exactly a connection and a memory value".into()));
            }
            for (id, _, values) in STANDARD_DIMENSIONS {
                let Some(v) = c.when.get(*id) else {
                    return Err(ctx(format!("needs a {id} value")));
                };
                if !values.iter().any(|(x, _, _)| x == v) {
                    return Err(ctx(format!(
                        "{v:?} isn't a {id} value. Known: {}",
                        values
                            .iter()
                            .map(|(x, _, _)| *x)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )));
                }
            }
            if !seen.insert(&c.when) {
                return Err(ctx("is listed twice".into()));
            }
            c.validate_programming().map_err(ctx)?;
        }
        Ok(())
    }

    /// The first configuration whose values include all of `wanted`.
    /// Dimensions `wanted` doesn't name match any value. An unknown dimension
    /// or value, or values no configuration has, are errors for people.
    pub fn select(&self, wanted: &BTreeMap<String, String>) -> Result<&Configuration, String> {
        for (dim, value) in wanted {
            let d = self
                .dimensions
                .iter()
                .find(|d| &d.id == dim)
                .ok_or_else(|| {
                    if self.dimensions.is_empty() {
                        "the board has no variants to choose from".to_string()
                    } else {
                        format!(
                            "the board has no dimension {dim:?}. Its dimensions: {}",
                            self.dimensions
                                .iter()
                                .map(|d| d.id.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    }
                })?;
            if !d.values.iter().any(|v| &v.id == value) {
                return Err(format!(
                    "{value:?} isn't a value of {dim}. Its values: {}",
                    d.values
                        .iter()
                        .map(|v| v.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        self.configurations
            .iter()
            .find(|c| wanted.iter().all(|(k, v)| c.when.get(k) == Some(v)))
            .ok_or_else(|| {
                let list: Vec<String> = self
                    .configurations
                    .iter()
                    .map(|c| serde_json::to_string(&c.when).unwrap_or_default())
                    .collect();
                format!(
                    "no configuration has these values. The board's configurations: {}",
                    list.join(", ")
                )
            })
    }
}

impl BoardDefinition {
    /// Checks a board definition, apart from its FPGA references.
    pub fn validate(&self) -> Result<(), String> {
        if !is_catalog_id(&self.fpga_id) {
            return Err(format!("bad fpga_id {:?}", self.fpga_id));
        }
        self.variants.validate()
    }
}

impl Catalog {
    pub fn parse(boards_json: &str, fpgas_json: &str) -> Result<Catalog, String> {
        let mut boards: BoardsFile =
            serde_json::from_str(boards_json).map_err(|e| format!("boards: {e}"))?;
        for b in boards.boards.values_mut() {
            b.variants.derive_dimensions();
        }
        let fpgas: FpgasFile =
            serde_json::from_str(fpgas_json).map_err(|e| format!("fpgas: {e}"))?;
        let catalog = Catalog {
            boards: boards.boards,
            fpgas: fpgas.fpgas,
        };
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn validate(&self) -> Result<(), String> {
        for (id, f) in &self.fpgas {
            if !is_catalog_id(id) {
                return Err(format!("bad FPGA ID {id:?}"));
            }
            f.validate().map_err(|e| format!("FPGA {id}: {e}"))?;
        }
        for (id, b) in &self.boards {
            if !is_catalog_id(id) {
                return Err(format!("bad board ID {id:?}"));
            }
            b.validate().map_err(|e| format!("board {id}: {e}"))?;
            if !self.fpgas.contains_key(&b.fpga_id) {
                return Err(format!("board {id}: unknown FPGA {}", b.fpga_id));
            }
        }
        Ok(())
    }
}

/// The catalog built into this engine release.
pub fn builtin() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        Catalog::parse(
            include_str!("../catalog/boards.json"),
            include_str!("../catalog/fpgas.json"),
        )
        .expect("the built-in catalog is valid")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fpgas_need_their_architectures_fields() {
        let ecp5 = FpgaDefinition {
            architecture: Architecture::Ecp5,
            part_number: "LFE5U-25F-6BG256C".into(),
            size: "25k".into(),
            nextpnr_device: "25k".into(),
            nextpnr_package: Some("CABGA256".into()),
            nextpnr_speed: Some("6".into()),
            nextpnr_family: None,
            packer_device: None,
            yosys_family: None,
            yosys_arch: None,
            prjxray_family: None,
        };
        assert!(ecp5.validate().is_ok());
        let no_speed = FpgaDefinition {
            nextpnr_speed: None,
            ..ecp5.clone()
        };
        assert_eq!(
            no_speed.validate().unwrap_err(),
            "ecp5 FPGAs need nextpnr_speed"
        );
        let ice40 = FpgaDefinition {
            architecture: Architecture::Ice40,
            ..ecp5.clone()
        };
        assert_eq!(
            ice40.validate().unwrap_err(),
            "ice40 FPGAs have no nextpnr_speed"
        );
        let bad = FpgaDefinition {
            nextpnr_package: Some("a b".into()),
            ..ecp5.clone()
        };
        assert!(bad.validate().unwrap_err().contains("nextpnr_package"));

        let gowin = FpgaDefinition {
            architecture: Architecture::Gowin,
            part_number: "GW1NR-LV9QN88PC6/I5".into(),
            size: "9k".into(),
            nextpnr_device: "GW1NR-LV9QN88PC6/I5".into(),
            nextpnr_package: None,
            nextpnr_speed: None,
            nextpnr_family: Some("GW1N-9C".into()),
            packer_device: Some("GW1N-9C".into()),
            yosys_family: None,
            yosys_arch: None,
            prjxray_family: None,
        };
        assert!(gowin.validate().is_ok());
        let no_packer = FpgaDefinition {
            packer_device: None,
            ..gowin.clone()
        };
        assert_eq!(
            no_packer.validate().unwrap_err(),
            "gowin FPGAs need packer_device"
        );
        let with_package = FpgaDefinition {
            nextpnr_package: Some("QN88".into()),
            ..gowin.clone()
        };
        assert_eq!(
            with_package.validate().unwrap_err(),
            "gowin FPGAs have no nextpnr_package"
        );
        let ecp5_with_family = FpgaDefinition {
            yosys_family: Some("gw2a".into()),
            ..ecp5
        };
        let xilinx = FpgaDefinition {
            architecture: Architecture::Xilinx,
            part_number: "XC7A35T-1CSG324".into(),
            size: "35k".into(),
            nextpnr_device: "xc7a35tcsg324-1".into(),
            nextpnr_package: None,
            nextpnr_speed: None,
            nextpnr_family: None,
            packer_device: None,
            yosys_family: None,
            yosys_arch: Some("xc7".into()),
            prjxray_family: Some("artix7".into()),
        };
        assert!(xilinx.validate().is_ok());
        let no_family = FpgaDefinition {
            prjxray_family: None,
            ..xilinx.clone()
        };
        assert_eq!(
            no_family.validate().unwrap_err(),
            "xilinx FPGAs need prjxray_family"
        );
        let bad_family = FpgaDefinition {
            prjxray_family: Some("/etc".into()),
            ..xilinx
        };
        assert!(
            bad_family
                .validate()
                .unwrap_err()
                .contains("prjxray_family")
        );
        assert!(
            ecp5_with_family
                .validate()
                .unwrap_err()
                .contains("yosys_family")
        );
    }

    #[test]
    fn builtin_catalog_has_every_architectures_boards() {
        let c = builtin();
        let count = |a: Architecture| {
            let fpgas = c.fpgas.values().filter(|f| f.architecture == a).count();
            let boards = c
                .boards
                .values()
                .filter(|b| c.fpgas[&b.fpga_id].architecture == a)
                .count();
            (fpgas, boards)
        };
        assert_eq!(count(Architecture::Ice40), (40, 31));
        assert_eq!(count(Architecture::Ecp5), (39, 22));
        assert_eq!(count(Architecture::Gowin), (241, 8));
        assert_eq!(count(Architecture::Xilinx), (212, 23));
        let arty = &c.fpgas[&c.boards["arty-a7-35t"].fpga_id];
        assert_eq!(arty.nextpnr_device, "xc7a35tcsg324-1");
        assert_eq!(arty.yosys_arch.as_deref(), Some("xc7"));
        assert_eq!(arty.prjxray_family.as_deref(), Some("artix7"));
        let nano = &c.fpgas[&c.boards["sipeed-tang-nano-9k"].fpga_id];
        assert_eq!(nano.nextpnr_device, "GW1NR-LV9QN88PC6/I5");
        assert_eq!(nano.nextpnr_family.as_deref(), Some("GW1N-9C"));
        assert_eq!(nano.packer_device.as_deref(), Some("GW1N-9C"));
        assert_eq!(nano.yosys_family, None);
        let primer = &c.fpgas[&c.boards["sipeed-tang-primer-20k"].fpga_id];
        assert_eq!(primer.yosys_family.as_deref(), Some("gw2a"));
        let sugar = &c.boards["icesugar-pro"];
        let f = &c.fpgas[&sugar.fpga_id];
        assert_eq!(
            (
                f.nextpnr_device.as_str(),
                f.package(),
                f.nextpnr_speed.as_deref()
            ),
            ("25k", "CABGA256", Some("6"))
        );
        let alhambra = &c.boards["alhambra-ii"];
        assert_eq!(alhambra.fpga_id, "ice40hx4k-tq144-8k");
        let v = &alhambra.variants.configurations[0];
        assert_eq!(v.programmer, "openfpgaloader");
        assert_eq!(v.usb.vid, "0403");
        assert_eq!(v.usb.pid, "6010");
        // Every board has both standard dimensions, here with one value each.
        assert_eq!(
            alhambra
                .variants
                .dimensions
                .iter()
                .map(|d| (d.id.as_str(), d.values.len()))
                .collect::<Vec<_>>(),
            [("connection", 1), ("memory", 1)]
        );
        assert_eq!(v.when["connection"], "usb");
        assert_eq!(v.when["memory"], "flash");
        assert_eq!(c.fpgas["ice40hx4k-tq144-8k"].package(), "tq144:4k");
        let configurations = || c.boards.values().flat_map(|b| &b.variants.configurations);
        assert_eq!(c.boards.len(), 84);
        assert_eq!(configurations().count(), 131);
        // Users pick boards by description, so no two boards share one.
        let mut descriptions: Vec<&str> =
            c.boards.values().map(|b| b.description.as_str()).collect();
        descriptions.sort();
        descriptions.dedup();
        assert_eq!(descriptions.len(), c.boards.len());
        for p in PROGRAMMERS {
            assert!(
                configurations().any(|v| v.programmer == *p),
                "no board uses {p}"
            );
        }
        // Boards with a choice: connections and memories. Sizes are separate
        // boards, since the pins may differ.
        let memory_of =
            |id: &str, i: usize| c.boards[id].variants.configurations[i].when["memory"].clone();
        let dims = |id: &str| -> Vec<usize> {
            c.boards[id]
                .variants
                .dimensions
                .iter()
                .map(|d| d.values.len())
                .collect()
        };
        for (id, choices) in [
            ("icesugar-pro", [1, 2]),
            ("colorlight-i5-v7-0", [1, 2]),
            ("colorlight-5a-75e-v71", [3, 2]),
            ("colorlight-i9-v7-2", [4, 2]),
            ("fleafpga-ohm", [3, 2]),
            ("arty-a7-35t", [1, 2]),
            ("sipeed-tang-nano", [1, 2]),
            // The bootloader writes flash, the adapters RAM.
            ("butterstick-r10-2g-85k", [3, 2]),
            // Zynq: openFPGALoader can't write their flash.
            ("pynq-z1", [1, 1]),
            // The ULX3S may have a bootloader at the start of its flash.
            ("ulx3s-12f", [1, 1]),
        ] {
            assert_eq!(dims(id), choices, "{id}");
        }
        // Flash configurations added to RAM-only boards keep the RAM one as
        // the default, so a project without a variant uploads as before.
        assert_eq!(memory_of("arty-a7-35t", 0), "ram");
        assert_eq!(memory_of("arty-a7-35t", 1), "flash");
        // Where openFPGALoader can't tell the part (no --board), the flash
        // command names it, so it loads the right SPI-over-JTAG bridge.
        for b in c.boards.values() {
            if c.fpgas[&b.fpga_id].architecture != Architecture::Xilinx {
                continue;
            }
            for v in &b.variants.configurations {
                if v.when["memory"] == "flash" {
                    assert!(
                        v.args.contains("--board ") || v.args.contains("--fpga-part "),
                        "{}",
                        b.description
                    );
                }
            }
        }
        let with_choices = c
            .boards
            .values()
            .filter(|b| b.variants.dimensions.iter().any(|d| d.values.len() > 1));
        assert_eq!(with_choices.count(), 32);
        // Where the design goes follows from the command: openFPGALoader
        // writes RAM unless told to write flash (iCE40 always flash).
        let memory = |id: &str| c.boards[id].variants.configurations[0].when["memory"].clone();
        assert_eq!(memory("arty-a7-35t"), "ram");
        assert_eq!(memory("sipeed-tang-nano-9k"), "flash");
        assert_eq!(memory("sipeed-tang-nano"), "ram");
        assert_eq!(memory("ulx3s-12f"), "ram");
        assert_eq!(memory("icestick"), "flash");
        for b in c.boards.values() {
            for v in &b.variants.configurations {
                let flag = v.args.split(' ').any(|w| w == "-f" || w == "--write-flash");
                if v.programmer == "openfpgaloader"
                    && c.fpgas[&b.fpga_id].architecture != Architecture::Ice40
                {
                    assert_eq!(v.when["memory"] == "flash", flag, "{}", b.description);
                }
            }
        }
        // An unspecified connection says why, for maintainers.
        for (id, b) in &c.boards {
            for v in &b.variants.configurations {
                if v.when["connection"] == "unspecified" {
                    assert!(v.maintainer_notes.is_some(), "{id}");
                }
            }
        }
        // Labels and help come from the engine.
        let ft = &c.boards["butterstick-r10-2g-85k"].variants.dimensions[0].values[2];
        assert_eq!(
            (ft.id.as_str(), ft.label.as_str()),
            ("ft232h", "FT232H adapter")
        );
        assert!(ft.help.as_deref().unwrap().contains("JTAG pins"));
        // Boards moved to another programmer are marked untested, with a note.
        for id in ["ulx3s-12f", "ulx3s-85f", "odt-icyblue-feather"] {
            for v in &c.boards[id].variants.configurations {
                assert!(!v.tested, "{id}");
                assert!(v.user_notes.is_some(), "{id}");
                assert!(v.maintainer_notes.is_some(), "{id}");
            }
        }
        // Every untested configuration says why, for maintainers.
        for (id, b) in &c.boards {
            for v in &b.variants.configurations {
                assert!(v.tested || v.maintainer_notes.is_some(), "{id}");
            }
        }
        // Maintainer notes stay in the catalog file.
        let json = serde_json::to_value(&c.boards["ulx3s-12f"]).unwrap();
        let first = &json["variants"]["configurations"][0];
        assert!(first.get("maintainer_notes").is_none());
        assert!(first.get("user_notes").is_some());
        assert!(c.boards["alhambra-ii"].variants.configurations[0].tested);
        // The iCESugar boards share USB IDs; their serial numbers start with
        // a board ID (icesprog's source): 0700, 0710 Pro, 0720 nano.
        for (id, re) in [
            ("icesugar-1-5", "^0700"),
            ("icesugar-pro", "^0710"),
            ("icesugar-nano", "^0720"),
        ] {
            for v in &c.boards[id].variants.configurations {
                assert_eq!(v.usb.serial_regex.as_deref(), Some(re), "{id}");
            }
        }
    }

    #[test]
    fn templates() {
        assert!(check_template("-d d:${BUS}/${DEV} ${BITSTREAM}").is_ok());
        assert!(check_template("--vid ${VID}").is_err());
        assert!(check_template("${BIN_FILE}").is_err());
        assert!(check_template("${BITSTREAM").is_err());
    }

    #[test]
    fn bad_boards_are_refused() {
        let ok = builtin().boards["alhambra-ii"].clone();
        fn first(b: &mut BoardDefinition) -> &mut Configuration {
            &mut b.variants.configurations[0]
        }
        let mut b = ok.clone();
        b.variants.configurations.clear();
        assert!(b.validate().is_err());
        let mut b = ok.clone();
        first(&mut b).programmer = "iceburn".into();
        assert!(b.validate().is_err());
        let mut b = ok.clone();
        first(&mut b).usb.vid = "0403A".into();
        assert!(b.validate().is_err());
        let mut b = ok.clone();
        first(&mut b).usb.product_regex = Some("(".into());
        assert!(b.validate().is_err());
        let mut b = ok.clone();
        first(&mut b).usb.serial_regex = Some("(".into());
        assert!(b.validate().is_err());
        // A value without a dimension.
        let mut b = ok.clone();
        first(&mut b).when.insert("size".into(), "1k".into());
        assert!(b.validate().is_err());
        let mut b = ok;
        first(&mut b)
            .when
            .insert("connection".into(), "serial".into());
        assert!(b.validate().unwrap_err().contains("unspecified"));
    }
}
