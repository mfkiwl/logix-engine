//! A plain-language summary of a build's resource use (docs/tool-api.md,
//! build tool), so hosts don't need to know each FPGA family's resource
//! names: "logic cells" instead of ICESTORM_LC, and "CLK" instead of the
//! clock net name nextpnr reports, such as CLK$SB_IO_IN_$glb_clk.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::parse::ResourceUsage;
use crate::catalog::Toolchain;

/// One kind of FPGA resource, in plain words, with how much the design uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourceSummary {
    /// The kind: logic_cells, luts, flip_flops, ram_blocks, multipliers,
    /// plls or io_pins. More kinds may be added.
    pub kind: String,
    /// The kind in plain words, plural, such as "logic cells" or "LUTs", for
    /// showing to people.
    pub label: String,
    /// How many the design uses.
    pub used: u64,
    /// How many the FPGA has.
    pub available: u64,
}

/// Per family, the kinds of resources, the family's main capacity measure
/// first, each with the toolchain names whose counts are added up.
fn kinds(arch: Toolchain) -> &'static [(&'static str, &'static str, &'static [&'static str])] {
    match arch {
        Toolchain::Ice40 => &[
            ("logic_cells", "logic cells", &["ICESTORM_LC"]),
            ("ram_blocks", "RAM blocks", &["ICESTORM_RAM"]),
            ("multipliers", "multipliers", &["ICESTORM_DSP"]),
            ("plls", "PLLs", &["ICESTORM_PLL"]),
            ("io_pins", "I/O pins", &["SB_IO"]),
        ],
        Toolchain::Ecp5 => &[
            ("luts", "LUTs", &["TRELLIS_COMB"]),
            ("flip_flops", "flip-flops", &["TRELLIS_FF"]),
            ("ram_blocks", "RAM blocks", &["DP16KD"]),
            ("multipliers", "multipliers", &["MULT18X18D"]),
            ("plls", "PLLs", &["EHXPLLL"]),
            ("io_pins", "I/O pins", &["TRELLIS_IO"]),
        ],
        Toolchain::Gowin => &[
            ("luts", "LUTs", &["LUT4"]),
            ("flip_flops", "flip-flops", &["DFF"]),
            ("ram_blocks", "RAM blocks", &["BSRAM"]),
            ("multipliers", "multipliers", &["MULT18X18"]),
            ("plls", "PLLs", &["rPLL", "PLLVR"]),
            ("io_pins", "I/O pins", &["IOB"]),
        ],
        Toolchain::Openxc7 => &[
            ("luts", "LUTs", &["SLICE_LUTX"]),
            ("flip_flops", "flip-flops", &["SLICE_FFX"]),
            ("ram_blocks", "RAM blocks", &["RAMB36E1", "RAMB18E1"]),
            ("multipliers", "multipliers", &["DSP48E1"]),
            ("plls", "PLLs", &["PLLE2_ADV", "MMCME2_ADV"]),
            ("io_pins", "I/O pins", &["PAD"]),
        ],
    }
}

/// The summary of a build's resource use: the family's main capacity
/// measure first, then the other kinds, each when the FPGA has any. Kinds
/// whose resources the report doesn't list are left out.
pub fn summarize(arch: Toolchain, usage: &BTreeMap<String, ResourceUsage>) -> Vec<ResourceSummary> {
    let mut out = Vec::new();
    for (i, (kind, label, names)) in kinds(arch).iter().enumerate() {
        let found: Vec<&ResourceUsage> = names.iter().filter_map(|n| usage.get(*n)).collect();
        if found.is_empty() {
            continue;
        }
        let used = found.iter().map(|u| u.used).sum();
        let available = found.iter().map(|u| u.available).sum();
        // The main measure always; others only when the FPGA has them.
        if i == 0 || available > 0 {
            out.push(ResourceSummary {
                kind: kind.to_string(),
                label: label.to_string(),
                used,
                available,
            });
        }
    }
    out
}

/// One clock of the design, with a plain name and its maximum frequency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClockSummary {
    /// The clock's name for showing to people, usually the design's clock
    /// signal, such as "CLK". For Xilinx it is the name synthesis gave the
    /// clock net, such as "o100", since nextpnr-xilinx doesn't report the
    /// signal's own name.
    pub name: String,
    /// The clock net name nextpnr reports, the key in timing.
    pub net: String,
    /// The maximum frequency the clock can run at, in MHz.
    pub max_mhz: f64,
}

/// A plain name for a clock net, per family, as the toolchains name the
/// net after the signal (the same rules as Apio's build report):
///
/// - iCE40: the part before the first `$`, without trailing `_`
///   (`CLK$SB_IO_IN_$glb_clk` is CLK).
/// - ECP5: the third `$`-separated part (`$glbnet$CLK$TRELLIS_IO_IN` is CLK).
/// - Gowin: the part before the first `$`, without `_IBUF_I_O`
///   (`CLK_IBUF_I_O` is CLK).
/// - Xilinx: the last non-empty part (`$abc$21083$aiger$o100` is o100).
///   TODO(revisit): that is a name synthesis made up, not the design's clock
///   signal; a fix may come from the openxc7 tools.
///
/// A net that doesn't follow the pattern keeps its full name.
pub fn clock_name(arch: Toolchain, net: &str) -> String {
    let parts: Vec<&str> = net.split('$').collect();
    let name = match arch {
        Toolchain::Ice40 => parts[0].trim_end_matches('_'),
        Toolchain::Ecp5 => parts.get(2).copied().unwrap_or(""),
        Toolchain::Gowin => parts[0].trim_end_matches("_IBUF_I_O"),
        Toolchain::Openxc7 => parts
            .iter()
            .rev()
            .find(|p| !p.is_empty())
            .copied()
            .unwrap_or(""),
    };
    if name.is_empty() { net } else { name }.to_string()
}

/// The design's clocks with plain names, in the order of timing (by net).
pub fn summarize_clocks(arch: Toolchain, timing: &BTreeMap<String, f64>) -> Vec<ClockSummary> {
    timing
        .iter()
        .map(|(net, mhz)| ClockSummary {
            name: clock_name(arch, net),
            net: net.clone(),
            max_mhz: *mhz,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_clocks_as_the_design_does() {
        use Toolchain::*;
        assert_eq!(clock_name(Ice40, "CLK$SB_IO_IN_$glb_clk"), "CLK");
        assert_eq!(clock_name(Ice40, "sys_clk$SB_IO_IN"), "sys_clk");
        assert_eq!(clock_name(Ecp5, "$glbnet$CLK$TRELLIS_IO_IN"), "CLK");
        assert_eq!(clock_name(Gowin, "sys_clk_IBUF_I_O"), "sys_clk");
        assert_eq!(clock_name(Openxc7, "$abc$21083$aiger$o100"), "o100");
        assert_eq!(
            clock_name(Openxc7, "sys_clk_IBUF_BUFG"),
            "sys_clk_IBUF_BUFG"
        );
        // Nets that don't follow the pattern keep their name.
        assert_eq!(clock_name(Ecp5, "clk"), "clk");
        assert_eq!(clock_name(Ice40, "$glb$x"), "$glb$x");

        let timing = BTreeMap::from([("CLK_IBUF_I_O".to_string(), 199.5)]);
        assert_eq!(
            summarize_clocks(Gowin, &timing),
            [ClockSummary {
                name: "CLK".into(),
                net: "CLK_IBUF_I_O".into(),
                max_mhz: 199.5
            }]
        );
    }

    fn usage(pairs: &[(&str, u64, u64)]) -> BTreeMap<String, ResourceUsage> {
        pairs
            .iter()
            .map(|(n, used, available)| {
                (
                    n.to_string(),
                    ResourceUsage {
                        used: *used,
                        available: *available,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn names_resources_in_plain_words_main_first() {
        let ice40 = usage(&[
            ("ICESTORM_LC", 27, 7680),
            ("ICESTORM_RAM", 0, 32),
            ("SB_IO", 2, 107),
            ("SB_GB", 1, 8),
        ]);
        let s = summarize(Toolchain::Ice40, &ice40);
        let kinds: Vec<_> = s
            .iter()
            .map(|r| (r.kind.as_str(), r.used, r.available))
            .collect();
        assert_eq!(
            kinds,
            [
                ("logic_cells", 27, 7680),
                ("ram_blocks", 0, 32),
                ("io_pins", 2, 107)
            ]
        );
        assert_eq!(s[0].label, "logic cells");

        // Two toolchain names for one kind are added up; a kind the FPGA
        // lacks (available 0) is left out.
        let xilinx = usage(&[
            ("SLICE_LUTX", 56, 20800),
            ("RAMB36E1", 1, 50),
            ("RAMB18E1", 2, 100),
            ("DSP48E1", 0, 0),
        ]);
        let s = summarize(Toolchain::Openxc7, &xilinx);
        let kinds: Vec<_> = s
            .iter()
            .map(|r| (r.kind.as_str(), r.used, r.available))
            .collect();
        assert_eq!(kinds, [("luts", 56, 20800), ("ram_blocks", 3, 150)]);
    }
}
