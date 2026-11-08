//! A board's resources (Logix Catalog's docs/model.md, Resources): what the
//! board wires to FPGA pins, by port name, each of a type with its own
//! fields, such as a clock's pin and frequency or an RGB LED's three colors.
//! For creating designs and pin files for the board, such as a blinky;
//! builds never use them. Resources may share pins (an LED on a PMOD pin);
//! within one resource, pins are unique.

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::Architecture;

/// A board's resources: port name to resource, in the board file's order.
/// The name is the port a design uses, a Verilog identifier: one pin gives a
/// plain port, several a vector, bit 0 first.
pub type Resources = IndexMap<String, Resource>;

/// The logic level that turns something on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ActiveLevel {
    High,
    Low,
}

/// An I/O standard, one common name for every FPGA family; pin file writers
/// translate it (IO_TYPE for ECP5 and Gowin, IOSTANDARD for Xilinx). iCE40
/// pin files have none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum IoStandard {
    #[serde(rename = "LVCMOS33")]
    Lvcmos33,
    #[serde(rename = "LVCMOS25")]
    Lvcmos25,
    #[serde(rename = "LVCMOS18")]
    Lvcmos18,
    #[serde(rename = "LVCMOS15")]
    Lvcmos15,
    #[serde(rename = "LVCMOS12")]
    Lvcmos12,
    #[serde(rename = "LVTTL")]
    Lvttl,
    #[serde(rename = "LVDS")]
    Lvds,
    #[serde(rename = "SSTL15")]
    Sstl15,
    #[serde(rename = "SSTL135")]
    Sstl135,
}

impl IoStandard {
    /// Whether pin files for the architecture can name it: none on iCE40;
    /// ECP5 has no LVTTL.
    pub fn fits(self, arch: Architecture) -> bool {
        match arch {
            Architecture::Ice40 => false,
            Architecture::Ecp5 => self != IoStandard::Lvttl,
            Architecture::Gowin | Architecture::Xilinx => true,
        }
    }

    pub fn name(self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default()
    }
}

/// Where a resource's facts come from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// hardware (tried on the board), schematic, maker (the maker's docs,
    /// pin files or examples), litex or amaranth (imported from their board
    /// files), or user (reported, not yet checked); strongest first.
    pub kind: SourceKind,
    /// The document, file and commit, or report.
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Hardware,
    Schematic,
    Maker,
    Litex,
    Amaranth,
    User,
}

/// Defines a resource type's struct: its own fields, then the fields every
/// resource may have.
macro_rules! resource_type {
    ($(#[$m:meta])* $name:ident { $($(#[$fm:meta])* $field:ident : $ty:ty),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            $($(#[$fm])* pub $field: $ty,)*
            /// The pins' I/O standard, such as LVCMOS33, when known and
            /// the FPGA family's pin files name one (not iCE40).
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub io_standard: Option<IoStandard>,
            /// Where the facts come from.
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub source: Option<Source>,
            /// Warnings and missing information for users, such as a jumper
            /// a clock needs.
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub user_notes: Option<String>,
        }
    };
}

resource_type!(
    /// An oscillator wired to one FPGA pin.
    Clock {
        /// The FPGA pin, as the board's pin files name it, such as 35.
        pin: String,
        /// The frequency in MHz, such as 12.
        mhz: f64,
    }
);

resource_type!(
    /// One or more of a kind (LEDs, buttons, switches), a pin each, bit 0
    /// first.
    PinsActive {
        /// The FPGA pins, bit 0 first.
        #[schemars(length(min = 1))]
        pins: Vec<String>,
        /// The level that turns them on: high or low. Left out when not
        /// known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        active: Option<ActiveLevel>,
    }
);

resource_type!(
    /// One RGB LED: a pin per color.
    RgbLed {
        red: String,
        green: String,
        blue: String,
        /// The level that turns a color on. Left out when not known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        active: Option<ActiveLevel>,
    }
);

/// A pin per color.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Colors {
    pub red: String,
    pub green: String,
    pub blue: String,
}

resource_type!(
    /// RGB LEDs in a matrix: an LED lights when its pin and a color's pin
    /// are both active.
    LedMatrix {
        /// A pin per LED.
        #[schemars(length(min = 1))]
        leds: Vec<String>,
        /// A pin per color, shared by the LEDs.
        colors: Colors,
        /// The level that turns them on. Left out when not known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        active: Option<ActiveLevel>,
    }
);

resource_type!(
    /// A serial port as the FPGA sees it: tx is the FPGA's output.
    Uart {
        tx: String,
        rx: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rts: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cts: Option<String>,
    }
);

resource_type!(
    /// A list of pins: for pmod, the 8 signal pins in header order (1 to 4,
    /// then 7 to 10); for unclassified, every FPGA pin the board wires to
    /// something no other resource describes.
    Pins {
        #[schemars(length(min = 1))]
        pins: Vec<String>,
    }
);

resource_type!(
    /// Any other header: every position in order, null for power and ground.
    Header {
        #[schemars(length(min = 1))]
        pins: Vec<Option<String>>,
    }
);

/// What a board wires to FPGA pins, by type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Resource {
    /// An oscillator: pin, mhz.
    Clock(Clock),
    /// LEDs: pins, active.
    Leds(PinsActive),
    /// One RGB LED: red, green, blue, active.
    RgbLed(RgbLed),
    /// RGB LEDs in a matrix: leds, colors, active.
    LedMatrix(LedMatrix),
    /// Push buttons: pins, active.
    Buttons(PinsActive),
    /// Switches: pins, active.
    Switches(PinsActive),
    /// A serial port: tx, rx, rts, cts.
    Uart(Uart),
    /// A PMOD header: its 8 signal pins.
    Pmod(Pins),
    /// Another header: every position, null for power and ground.
    Header(Header),
    /// Every wired pin no other resource describes; a board with one lists
    /// all its wired pins.
    Unclassified(Pins),
}

impl Resource {
    /// Its pins as (field, pin), in order, nulls left out.
    pub fn pins(&self) -> Vec<(String, &str)> {
        fn list<'a>(name: &str, ps: &'a [String]) -> Vec<(String, &'a str)> {
            ps.iter()
                .enumerate()
                .map(|(i, p)| (format!("{name}/{i}"), p.as_str()))
                .collect()
        }
        match self {
            Resource::Clock(c) => vec![("pin".into(), c.pin.as_str())],
            Resource::Leds(p) | Resource::Buttons(p) | Resource::Switches(p) => {
                list("pins", &p.pins)
            }
            Resource::RgbLed(r) => vec![
                ("red".into(), r.red.as_str()),
                ("green".into(), r.green.as_str()),
                ("blue".into(), r.blue.as_str()),
            ],
            Resource::LedMatrix(m) => {
                let mut out = list("leds", &m.leds);
                out.push(("colors/red".into(), m.colors.red.as_str()));
                out.push(("colors/green".into(), m.colors.green.as_str()));
                out.push(("colors/blue".into(), m.colors.blue.as_str()));
                out
            }
            Resource::Uart(u) => {
                let mut out = vec![("tx".into(), u.tx.as_str()), ("rx".into(), u.rx.as_str())];
                for (n, p) in [("rts", &u.rts), ("cts", &u.cts)] {
                    if let Some(p) = p {
                        out.push((n.into(), p.as_str()));
                    }
                }
                out
            }
            Resource::Pmod(p) | Resource::Unclassified(p) => list("pins", &p.pins),
            Resource::Header(h) => h
                .pins
                .iter()
                .enumerate()
                .filter_map(|(i, p)| p.as_deref().map(|p| (format!("pins/{i}"), p)))
                .collect(),
        }
    }

    pub fn io_standard(&self) -> Option<IoStandard> {
        match self {
            Resource::Clock(c) => c.io_standard,
            Resource::Leds(p) | Resource::Buttons(p) | Resource::Switches(p) => p.io_standard,
            Resource::RgbLed(r) => r.io_standard,
            Resource::LedMatrix(m) => m.io_standard,
            Resource::Uart(u) => u.io_standard,
            Resource::Pmod(p) | Resource::Unclassified(p) => p.io_standard,
            Resource::Header(h) => h.io_standard,
        }
    }

    /// How many LEDs it is: a resource of LEDs counts each, an RGB LED one.
    pub fn led_count(&self) -> usize {
        match self {
            Resource::Leds(p) => p.pins.len(),
            Resource::RgbLed(_) => 1,
            Resource::LedMatrix(m) => m.leds.len(),
            _ => 0,
        }
    }

    /// A clock's frequency in MHz.
    pub fn clock_mhz(&self) -> Option<f64> {
        match self {
            Resource::Clock(c) => Some(c.mhz),
            _ => None,
        }
    }
}

/// A Verilog identifier: a letter or underscore, then letters, digits and
/// underscores.
fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A board's resource problems, each as (resource name, field path under it
/// such as "pins/1", or "" for the name itself, message), for the catalog's
/// checks and custom boards' diagnostics. `arch` is the board's FPGA family,
/// when known.
pub fn problems(
    resources: &Resources,
    arch: Option<Architecture>,
) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for (name, r) in resources {
        let mut add = |field: &str, text: String| out.push((name.clone(), field.to_string(), text));
        if !is_identifier(name) {
            add(
                "",
                format!(
                    "{name:?} isn't a port name: use letters, digits and underscores, starting with a letter."
                ),
            );
        }
        let pins = r.pins();
        let mut seen = std::collections::BTreeSet::new();
        for (field, p) in &pins {
            if p.is_empty() || p.chars().any(|c| c.is_whitespace() || c == '"' || c == ';') {
                add(field, format!("{p:?} isn't a pin name."));
            } else if !seen.insert(*p) {
                add(field, format!("Pin {p} is in this resource twice."));
            }
        }
        match r {
            Resource::Clock(c) if !(c.mhz > 0.0 && c.mhz.is_finite()) => {
                add("mhz", format!("{} isn't a frequency in MHz.", c.mhz));
            }
            Resource::Pmod(p) if p.pins.len() != 8 => {
                add(
                    "pins",
                    "A PMOD lists its 8 signal pins: 1 to 4, then 7 to 10.".into(),
                );
            }
            Resource::Leds(p) | Resource::Buttons(p) | Resource::Switches(p)
                if p.pins.is_empty() =>
            {
                add("pins", "At least one pin.".into());
            }
            Resource::Unclassified(p) if p.pins.is_empty() => {
                add("pins", "At least one pin.".into());
            }
            _ => {}
        }
        if let (Some(std), Some(arch)) = (r.io_standard(), arch)
            && !std.fits(arch)
        {
            add(
                "io_standard",
                format!(
                    "{} isn't an I/O standard for {}{}.",
                    std.name(),
                    arch.full_label(),
                    if arch == Architecture::Ice40 {
                        ": its pin files have none"
                    } else {
                        ""
                    }
                ),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn res(v: serde_json::Value) -> Resources {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn types_read_and_write_as_in_the_catalog() {
        let text = serde_json::json!({
            "clk": {"type": "clock", "pin": "35", "mhz": 12.5},
            "leds": {"type": "leds", "pins": ["11", "37"], "active": "low"},
            "rgb": {"type": "rgb_led", "red": "41", "green": "39", "blue": "40"},
            "m": {"type": "led_matrix", "leds": ["C13"], "colors": {"red": "T1", "green": "U1", "blue": "R1"},
                  "io_standard": "LVCMOS33", "source": {"kind": "schematic", "ref": "sheet 5"}},
            "uart": {"type": "uart", "tx": "9", "rx": "6"},
            "p": {"type": "pmod", "pins": ["1", "2", "3", "4", "5", "6", "7", "8"]},
            "h": {"type": "header", "pins": ["1", null]}
        });
        let r = res(text.clone());
        assert_eq!(serde_json::to_value(&r).unwrap(), text);
        // The file's order is kept.
        assert_eq!(r.keys().next().map(String::as_str), Some("clk"));
        assert_eq!(
            r["m"].led_count() + r["leds"].led_count() + r["rgb"].led_count(),
            4
        );
        assert_eq!(r["clk"].clock_mhz(), Some(12.5));
        // Unknown types and fields are errors.
        for bad in [
            serde_json::json!({"b": {"type": "lamp", "pins": ["1"]}}),
            serde_json::json!({"b": {"type": "leds", "pins": ["1"], "color": "red"}}),
            serde_json::json!({"b": {"type": "clock", "pin": "1"}}),
        ] {
            assert!(serde_json::from_value::<Resources>(bad).is_err());
        }
    }

    #[test]
    fn problems_are_found() {
        let t = |a: &str, b: &str, c: &str| (a.to_string(), b.to_string(), c.to_string());
        let r = res(serde_json::json!({
            "2clk": {"type": "clock", "pin": "35", "mhz": 0},
            "leds": {"type": "leds", "pins": ["1", "1"]},
            "p": {"type": "pmod", "pins": ["1"], "io_standard": "LVCMOS33"}
        }));
        assert_eq!(
            problems(&r, Some(Architecture::Ice40)),
            [
                t(
                    "2clk",
                    "",
                    "\"2clk\" isn't a port name: use letters, digits and underscores, starting with a letter."
                ),
                t("2clk", "mhz", "0 isn't a frequency in MHz."),
                t("leds", "pins/1", "Pin 1 is in this resource twice."),
                t(
                    "p",
                    "pins",
                    "A PMOD lists its 8 signal pins: 1 to 4, then 7 to 10."
                ),
                t(
                    "p",
                    "io_standard",
                    "LVCMOS33 isn't an I/O standard for Lattice iCE40: its pin files have none."
                ),
            ]
        );
        // Resources may share pins.
        let shared = res(serde_json::json!({
            "led": {"type": "leds", "pins": ["1"]},
            "p": {"type": "pmod", "pins": ["1", "2", "3", "4", "5", "6", "7", "8"]}
        }));
        assert!(problems(&shared, Some(Architecture::Ice40)).is_empty());
    }
}
