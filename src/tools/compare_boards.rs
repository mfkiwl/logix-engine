//! The compare_boards tool (docs/tool-api.md, compare_boards tool): what
//! differs between two boards, as structured facts a host decides what to
//! do with, such as warning the user or adapting a project. Every aspect is
//! always listed, in a fixed order; one the catalog doesn't describe yet is
//! `unknown`, so new catalog facts only add aspects or settle unknown ones.
//! Boards only: a board's variants differ only in how it's programmed, so
//! they never change the comparison.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::EngineTool;
use crate::board::ResolvedBoard;
use crate::catalog;
use crate::errors::ToolError;

pub struct CompareBoards;

impl EngineTool for CompareBoards {
    const NAME: &'static str = "compare_boards";
    const DESCRIPTION: &'static str = "Compares two boards, such as a project's board and one \
        it may move to. Returns every aspect (board, architecture, fpga, size, \
        constraint_format, pins), each the same, different or unknown (the catalog doesn't \
        describe it yet), with both values and a sentence for people, and same_build: whether \
        a build for one board is valid for the other. Facts only: what they mean for a project, \
        such as files to change, is the host's decision. Boards can be catalog boards or the \
        request's custom_boards_file.";
    const READ_ONLY: bool = true;
    type Input = CompareBoardsInput;
    type Output = CompareBoardsOutput;
}

/// The two boards.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompareBoardsInput {
    /// The first board's ID, such as the project's board now.
    pub from_board: String,
    /// The second board's ID, such as the board the project may move to.
    pub to_board: String,
    /// The host's own boards: the absolute path of a JSON file that maps
    /// board IDs to definitions in the catalog's format. Either board can be
    /// one of them; one with a catalog board's ID replaces that board in
    /// this request. Omitted means none.
    #[serde(default)]
    pub custom_boards_file: Option<String>,
}

/// Whether an aspect is the same for both boards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Same {
    Yes,
    No,
    /// The catalog doesn't describe it yet.
    Unknown,
}

/// One aspect of the two boards.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Aspect {
    /// What is compared: board, architecture, fpga, size, constraint_format
    /// or pins. Later versions add aspects; hosts skip those they don't
    /// know, or show their text.
    pub aspect: String,
    /// yes, no or unknown.
    pub same: Same,
    /// The first board's value, when known. Its type depends on the aspect:
    /// text for every aspect today; later aspects may give numbers, lists
    /// or objects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Value>,
    /// The second board's value, when known, of the same type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<Value>,
    /// The comparison for people, such as "FPGA: ICE40HX4K-TQ144 →
    /// ICE40UP5K-SG48.".
    pub text: String,
}

/// The two boards compared.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompareBoardsOutput {
    /// True when a build for one board is valid for the other: their FPGAs
    /// have the same build settings, so the same bitstream fits both.
    pub same_build: bool,
    /// Every aspect, in a fixed order: board, architecture, fpga, size,
    /// constraint_format, pins.
    pub aspects: Vec<Aspect>,
}

pub fn run(input: CompareBoardsInput) -> Result<CompareBoardsOutput, ToolError> {
    let custom = crate::custom_boards::load(input.custom_boards_file.as_deref())?;
    let a = crate::board::resolve(&input.from_board, &None, &custom)?;
    let b = crate::board::resolve(&input.to_board, &None, &custom)?;
    Ok(compare(&a, &b))
}

/// An aspect with a text value on each side: the same or not, and its
/// sentence ("Same FPGA: x." or "FPGA: x → y.").
fn text_aspect(
    name: &str,
    (label, same_label): (&str, &str),
    x: &str,
    y: &str,
    same: bool,
) -> Aspect {
    Aspect {
        aspect: name.into(),
        same: if same { Same::Yes } else { Same::No },
        from: Some(x.into()),
        to: Some(y.into()),
        text: if same {
            format!("{same_label}: {x}.")
        } else {
            format!("{label}: {x} → {y}.")
        },
    }
}

/// An aspect without values.
fn plain_aspect(name: &str, same: Same, text: &str) -> Aspect {
    Aspect {
        aspect: name.into(),
        same,
        from: None,
        to: None,
        text: text.into(),
    }
}

/// Two boards compared.
pub fn compare(a: &ResolvedBoard, b: &ResolvedBoard) -> CompareBoardsOutput {
    let same_board = a.board_id == b.board_id;
    let same_build = a.build_key() == b.build_key();
    let same_fpga = match (&a.fpga_id, &b.fpga_id) {
        (Some(x), Some(y)) if x == y => true,
        _ => same_build,
    };
    let (arch_a, arch_b) = (a.fpga.architecture, b.fpga.architecture);
    let ext = |x: catalog::Architecture| format!(".{}", x.constraint_extension());
    CompareBoardsOutput {
        same_build,
        aspects: vec![
            text_aspect(
                "board",
                ("Board", "Same board"),
                &a.description,
                &b.description,
                same_board,
            ),
            text_aspect(
                "architecture",
                ("Architecture", "Same architecture"),
                arch_a.full_label(),
                arch_b.full_label(),
                arch_a == arch_b,
            ),
            text_aspect(
                "fpga",
                ("FPGA", "Same FPGA"),
                &a.fpga.part_number,
                &b.fpga.part_number,
                same_fpga,
            ),
            text_aspect(
                "size",
                ("Size", "Same size"),
                &a.fpga.size,
                &b.fpga.size,
                a.fpga.size == b.fpga.size,
            ),
            text_aspect(
                "constraint_format",
                ("Pin file format", "Same pin file format"),
                &ext(arch_a),
                &ext(arch_b),
                arch_a.constraint_extension() == arch_b.constraint_extension(),
            ),
            if same_board {
                plain_aspect("pins", Same::Yes, "Same pins: the same board.")
            } else {
                plain_aspect(
                    "pins",
                    Same::Unknown,
                    "Pins: not known whether they match (the catalog doesn't describe boards' pins yet).",
                )
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cmp(input: Value) -> CompareBoardsOutput {
        run(serde_json::from_value(input).unwrap()).unwrap()
    }

    type Row = (String, Same, Option<Value>, Option<Value>);

    /// Each aspect as (aspect, same, from, to).
    fn table(out: &CompareBoardsOutput) -> Vec<Row> {
        out.aspects
            .iter()
            .map(|a| (a.aspect.clone(), a.same, a.from.clone(), a.to.clone()))
            .collect()
    }

    fn row(aspect: &str, same: Same, from: Option<&str>, to: Option<&str>) -> Row {
        (
            aspect.into(),
            same,
            from.map(Into::into),
            to.map(Into::into),
        )
    }

    #[test]
    fn another_fpga_of_the_same_architecture() {
        let out = cmp(json!({"from_board": "alhambra-ii", "to_board": "icebreaker"}));
        assert!(!out.same_build);
        use Same::*;
        assert_eq!(
            table(&out),
            [
                row("board", No, Some("Alhambra II"), Some("iCEBreaker")),
                row(
                    "architecture",
                    Yes,
                    Some("Lattice iCE40"),
                    Some("Lattice iCE40")
                ),
                row("fpga", No, Some("ICE40HX4K-TQ144"), Some("ICE40UP5K-SG48")),
                row("size", No, Some("8k"), Some("5k")),
                row("constraint_format", Yes, Some(".pcf"), Some(".pcf")),
                row("pins", Unknown, None, None),
            ]
        );
        let texts: Vec<_> = out.aspects.iter().map(|a| a.text.as_str()).collect();
        assert_eq!(texts[0], "Board: Alhambra II → iCEBreaker.");
        assert_eq!(texts[1], "Same architecture: Lattice iCE40.");
        assert_eq!(texts[2], "FPGA: ICE40HX4K-TQ144 → ICE40UP5K-SG48.");
        assert_eq!(texts[4], "Same pin file format: .pcf.");
    }

    #[test]
    fn another_architecture() {
        let out = cmp(json!({"from_board": "alhambra-ii", "to_board": "ulx3s-85f"}));
        let by = |name: &str| {
            out.aspects
                .iter()
                .find(|a| a.aspect == name)
                .unwrap()
                .clone()
        };
        assert_eq!(by("architecture").same, Same::No);
        assert_eq!(
            by("constraint_format").text,
            "Pin file format: .pcf → .lpf."
        );
    }

    #[test]
    fn the_same_fpga_builds_the_same() {
        // Boards with the same FPGA: a build for one is valid for the other.
        let out = cmp(json!({"from_board": "alhambra-ii", "to_board": "azukar2"}));
        assert!(out.same_build);
        assert_eq!(out.aspects[2].same, Same::Yes);
        assert_eq!(out.aspects[2].text, "Same FPGA: ICE40HX4K-TQ144.");
        // The same board: everything the same.
        let out = cmp(json!({"from_board": "icesugar-pro", "to_board": "icesugar-pro"}));
        assert!(out.same_build);
        assert!(
            out.aspects.iter().all(|a| a.same == Same::Yes),
            "{:?}",
            out.aspects
        );
        assert_eq!(out.aspects[5].text, "Same pins: the same board.");
    }

    #[test]
    fn custom_boards_and_unknown_ids() {
        let (_tmp, file) = crate::custom_boards::test_file(&json!({
            "my-board": {
                "description": "My board", "fpga_id": "ice40hx4k-tq144-8k",
                "variants": [{"selector": {"connection": "usb", "memory": "flash"},
                  "programmer": "iceprog", "args": "${BITSTREAM}",
                  "usb": {"vid": "0403", "pid": "6010"}, "tested": false}]
            }
        }));
        let out = cmp(json!({
            "from_board": "alhambra-ii", "to_board": "my-board", "custom_boards_file": file
        }));
        assert!(out.same_build);
        let e = run(serde_json::from_value(
            json!({"from_board": "alhambra-ii", "to_board": "nope"}),
        )
        .unwrap())
        .unwrap_err();
        assert_eq!(e.category, crate::errors::Category::Request);
        // Variants aren't inputs: they never change the comparison.
        assert!(
            serde_json::from_value::<CompareBoardsInput>(json!({
                "from_board": "a", "to_board": "b",
                "to_variant": {"connection": "usb", "memory": "ram"}
            }))
            .is_err()
        );
    }
}
