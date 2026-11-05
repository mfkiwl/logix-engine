//! A plain-language summary of a build's resource use (docs/tool-api.md,
//! build tool), so hosts don't need to know each FPGA family's resource
//! names: "logic cells" instead of ICESTORM_LC.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::parse::ResourceUsage;
use crate::catalog::Architecture;

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
fn kinds(arch: Architecture) -> &'static [(&'static str, &'static str, &'static [&'static str])] {
    match arch {
        Architecture::Ice40 => &[
            ("logic_cells", "logic cells", &["ICESTORM_LC"]),
            ("ram_blocks", "RAM blocks", &["ICESTORM_RAM"]),
            ("multipliers", "multipliers", &["ICESTORM_DSP"]),
            ("plls", "PLLs", &["ICESTORM_PLL"]),
            ("io_pins", "I/O pins", &["SB_IO"]),
        ],
        Architecture::Ecp5 => &[
            ("luts", "LUTs", &["TRELLIS_COMB"]),
            ("flip_flops", "flip-flops", &["TRELLIS_FF"]),
            ("ram_blocks", "RAM blocks", &["DP16KD"]),
            ("multipliers", "multipliers", &["MULT18X18D"]),
            ("plls", "PLLs", &["EHXPLLL"]),
            ("io_pins", "I/O pins", &["TRELLIS_IO"]),
        ],
        Architecture::Gowin => &[
            ("luts", "LUTs", &["LUT4"]),
            ("flip_flops", "flip-flops", &["DFF"]),
            ("ram_blocks", "RAM blocks", &["BSRAM"]),
            ("multipliers", "multipliers", &["MULT18X18"]),
            ("plls", "PLLs", &["rPLL", "PLLVR"]),
            ("io_pins", "I/O pins", &["IOB"]),
        ],
        Architecture::Xilinx => &[
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
pub fn summarize(
    arch: Architecture,
    usage: &BTreeMap<String, ResourceUsage>,
) -> Vec<ResourceSummary> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let s = summarize(Architecture::Ice40, &ice40);
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
        let s = summarize(Architecture::Xilinx, &xilinx);
        let kinds: Vec<_> = s
            .iter()
            .map(|r| (r.kind.as_str(), r.used, r.available))
            .collect();
        assert_eq!(kinds, [("luts", 56, 20800), ("ram_blocks", 3, 150)]);
    }
}
