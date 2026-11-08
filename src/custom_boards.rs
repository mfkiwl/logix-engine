//! The custom boards file (docs/tool-api.md, Board specification): every
//! tool that deals with boards takes `custom_boards_file`, the absolute path
//! of a JSON file that maps board IDs to definitions in the catalog's
//! format. The engine reads only the file the request names, and reports
//! every problem in it as a diagnostic at file, line and column, as it
//! reports problems in source files.

use std::collections::BTreeSet;
use std::path::Path;

use crate::board::{CustomBoard, CustomBoards};
use crate::catalog::{self, PROGRAMMERS, STANDARD_DIMENSIONS, Variant};
use crate::errors::{Diagnostic, Severity, ToolError};
use crate::json_pos::{Position, PositionIndex, segment};

/// The custom boards of a request: none without a file, else the file's
/// boards when it has no problems. A file that can't be read, isn't valid
/// JSON or has an invalid board is a request error whose diagnostics name
/// each problem. Diagnostics name the file as the request gave it (a
/// request's display paths may rename it, docs/tool-api.md).
pub fn load(file: Option<&str>) -> Result<CustomBoards, ToolError> {
    let Some(given) = file else {
        return Ok(CustomBoards::new());
    };
    let path = Path::new(given);
    if !path.is_absolute() {
        return Err(ToolError::request(format!(
            "custom_boards_file: {given:?} isn't an absolute path."
        )));
    }
    let real = crate::paths::resolve(path)
        .map_err(|e| ToolError::request(format!("custom_boards_file: can't read {given}: {e}.")))?;
    let text = std::fs::read_to_string(&real)
        .map_err(|e| ToolError::request(format!("custom_boards_file: can't read {given}: {e}.")))?;
    let result = parse(&text, given);
    log::info!("{}", log_line(given, &text, &result));
    result
}

/// The log's line about a request's custom boards file: its path, a short
/// SHA-256 of its contents (to tell which version was used) and its board
/// IDs, or how many problems it has.
fn log_line(given: &str, text: &str, result: &Result<CustomBoards, ToolError>) -> String {
    use sha2::{Digest, Sha256};
    let digest = hex::encode(Sha256::digest(text.as_bytes()));
    let head = format!("Custom boards file: {given} (sha256 {})", &digest[..12]);
    match result {
        Ok(boards) => {
            let ids: Vec<&str> = boards.keys().map(String::as_str).collect();
            format!(
                "{head}, boards: {}",
                if ids.is_empty() {
                    "none".into()
                } else {
                    ids.join(", ")
                }
            )
        }
        Err(e) => format!("{head}, problems: {}", e.diagnostics.len().max(1)),
    }
}

/// At most this many problems are listed. Past it, a last note says how
/// many more there are: editing these files by hand is rare, and a few
/// problems at a time are easier to fix than a long list.
pub const MAX_PROBLEMS: usize = 10;

/// The boards in a custom boards file's `text`, or a request error with a
/// diagnostic for each problem, in file `name`, in file order. Invalid JSON
/// is one problem, the parser's. Otherwise each board and each variant is
/// read on its own, so a wrong type or field in one doesn't hide problems
/// in the others, and every part that reads cleanly is checked in full.
pub fn parse(text: &str, name: &str) -> Result<CustomBoards, ToolError> {
    if text.trim().is_empty() {
        return Ok(CustomBoards::new());
    }
    let root: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => return Err(problems(name, vec![from_serde(&e, (1, 1))])),
    };
    if !root.is_object() {
        // Not a map of boards: the parser's message, such as "Invalid type:
        // sequence, expected a map".
        let e = serde_json::from_str::<CustomBoards>(text)
            .err()
            .map(|e| from_serde(&e, (1, 1)))
            .unwrap_or_else(|| {
                Found::at(
                    Some((1, 1)),
                    "Expected a map of board IDs to boards.".into(),
                )
            });
        return Err(problems(name, vec![e]));
    }
    let doc = Doc::new(text, &root);
    let mut found = Vec::new();
    let mut boards = CustomBoards::new();
    for (id, value) in root.as_object().into_iter().flatten() {
        let base = format!("/{}", segment(id));
        // The file's schema, for editors: optional, but only this one.
        if id == catalog::SCHEMA_KEY {
            if let Some(problem) = catalog::schema_problem(value) {
                found.push(Found::at(doc.index.value(&base), problem));
            }
            continue;
        }
        let variants_value = value.get("variants").and_then(|v| v.as_array());
        let board = doc.read_board(&base);
        if let Err(e) = &board {
            found.push(e.clone());
        }
        let mut variants = Vec::new();
        for i in 0..variants_value.map_or(0, Vec::len) {
            match doc.read::<Variant>(&format!("{base}/variants/{i}")) {
                Ok(v) => variants.push((i, v)),
                Err(e) => found.push(e),
            }
        }
        let checks = board_problems(
            id,
            board.as_ref().ok(),
            variants_value.map(Vec::len),
            &variants,
        );
        found.extend(checks.into_iter().map(|p| {
            let pos = match p.at {
                At::Key(path) => doc.index.key(&path),
                At::Value(path) => doc.index.nearest(&path),
            };
            Found::at(pos, p.text)
        }));
        if let Ok(mut b) = board {
            b.variants = variants.into_iter().map(|(_, v)| v).collect();
            boards.insert(id.clone(), b);
        }
    }
    if found.is_empty() {
        return Ok(boards);
    }
    // File order; a problem without a place last.
    found.sort_by_key(|f| f.pos.unwrap_or((u32::MAX, u32::MAX)));
    Err(problems(name, found))
}

/// A problem found, at its place in the file when known.
#[derive(Debug, Clone)]
struct Found {
    pos: Option<Position>,
    text: String,
}

impl Found {
    fn at(pos: Option<Position>, text: String) -> Found {
        Found { pos, text }
    }
}

/// The parser's problem, its place counted from `start`, where the text it
/// read starts in the file. A missing field is at the object it belongs in
/// (`start`): the parser notices it only at the object's end.
fn from_serde(e: &serde_json::Error, start: Position) -> Found {
    if e.to_string().starts_with("missing field") {
        return Found::at(Some(start), serde_message(e));
    }
    let (line, column) = (e.line() as u32, e.column() as u32);
    let pos = match (line, column) {
        (0, _) | (_, 0) => Some(start),
        (1, c) => Some((start.0, start.1 + c - 1)),
        (l, c) => Some((start.0 + l - 1, c)),
    };
    Found::at(pos, serde_message(e))
}

/// The file's text and where its values are, to read its parts one by one.
struct Doc<'a> {
    chars: Vec<char>,
    root: &'a serde_json::Value,
    index: PositionIndex,
}

impl<'a> Doc<'a> {
    fn new(text: &str, root: &'a serde_json::Value) -> Doc<'a> {
        Doc {
            chars: text.chars().collect(),
            root,
            index: PositionIndex::new(text).unwrap_or_default(),
        }
    }

    /// The value at `path` as written in the file, or None when the index
    /// doesn't have it.
    fn text(&self, path: &str) -> Option<Vec<char>> {
        let (a, b) = self.index.span(path)?;
        Some(self.chars[a..b].to_vec())
    }

    /// The value at `path` read as a `T`, with the parser's problem at its
    /// place in the file.
    fn read<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, Found> {
        let start = self.index.value(path).unwrap_or((1, 1));
        match self.text(path) {
            Some(chars) => serde_json::from_str(&chars.iter().collect::<String>())
                .map_err(|e| from_serde(&e, start)),
            None => serde_json::from_value(self.root.pointer(path).cloned().unwrap_or_default())
                .map_err(|e| Found::at(Some(start), serde_message(&e))),
        }
    }

    /// The board at `path` without its variants, which are read one by one:
    /// a list of variants is blanked to `[]` (lines kept, so places stay
    /// right).
    fn read_board(&self, path: &str) -> Result<CustomBoard, Found> {
        let variants = format!("{path}/variants");
        let is_list = self.root.pointer(&variants).is_some_and(|v| v.is_array());
        let (Some((a, _)), Some((va, vb)), Some(mut chars), true) = (
            self.index.span(path),
            self.index.span(&variants),
            self.text(path),
            is_list,
        ) else {
            return self.read(path);
        };
        for (n, c) in chars[va - a..vb - a].iter_mut().enumerate() {
            *c = match (n, *c) {
                (0, _) => '[',
                (1, _) => ']',
                (_, '\n') => '\n',
                _ => ' ',
            };
        }
        let start = self.index.value(path).unwrap_or((1, 1));
        serde_json::from_str(&chars.iter().collect::<String>()).map_err(|e| from_serde(&e, start))
    }
}

/// The request error for a file's problems: the first ones, at most
/// `MAX_PROBLEMS`, then a note with how many more there are.
fn problems(name: &str, found: Vec<Found>) -> ToolError {
    let total = found.len();
    let mut diagnostics: Vec<Diagnostic> = found
        .into_iter()
        .take(MAX_PROBLEMS)
        .map(|f| Diagnostic {
            file: Some(name.to_string()),
            line: f.pos.map(|p| p.0),
            column: f.pos.map(|p| p.1),
            severity: Severity::Error,
            text: f.text,
            tool: None,
            time_ns: None,
        })
        .collect();
    if total > MAX_PROBLEMS {
        let more = total - MAX_PROBLEMS;
        diagnostics.push(Diagnostic {
            file: Some(name.to_string()),
            line: None,
            column: None,
            severity: Severity::Info,
            text: format!(
                "{} not listed. Fix these first, then check again.",
                if more == 1 {
                    "1 more problem".to_string()
                } else {
                    format!("{more} more problems")
                }
            ),
            tool: None,
            time_ns: None,
        });
    }
    let first = &diagnostics[0];
    let place = match (first.line, first.column) {
        (Some(l), Some(c)) => format!("{name}:{l}:{c}"),
        (Some(l), None) => format!("{name}:{l}"),
        _ => name.to_string(),
    };
    let more = match total {
        1 => String::new(),
        2 => " (and 1 more problem)".to_string(),
        n => format!(" (and {} more problems)", n - 1),
    };
    let mut e = ToolError::request(format!(
        "The custom boards file has problems. {place}: {}{more}",
        first.text
    ));
    e.diagnostics = diagnostics;
    e
}

/// serde's message without its own " at line L column C".
/// An unknown field is just named, without serde's list of the expected
/// ones (the user's request: too much to read).
fn serde_message(e: &serde_json::Error) -> String {
    let text = e.to_string();
    let text = match text.rfind(" at line ") {
        Some(i) => text[..i].to_string(),
        None => text,
    };
    if let Some(rest) = text.strip_prefix("unknown field `")
        && let Some(end) = rest.find('`')
    {
        return format!("Unknown field {:?}.", &rest[..end]);
    }
    let mut c = text.chars();
    match c.next() {
        Some(first) => format!("{}{}.", first.to_uppercase(), c.as_str()),
        None => "Invalid JSON.".into(),
    }
}

/// Where a problem is: a member's key, or a value (or, when it's missing,
/// the object it belongs in).
enum At {
    Key(String),
    Value(String),
}

struct Problem {
    at: At,
    text: String,
}

/// Every problem with one board's contents: its ID; its FPGA, when `board`
/// (without its variants) could be read; that it has variants, when
/// `variant_count` says how many are listed; and each of `variants`, those
/// that could be read, with their place in the list.
fn board_problems(
    id: &str,
    board: Option<&CustomBoard>,
    variant_count: Option<usize>,
    variants: &[(usize, Variant)],
) -> Vec<Problem> {
    let base = format!("/{}", segment(id));
    let field = |f: &str| format!("{base}/{f}");
    let mut out = Vec::new();
    let mut add = |at: At, text: String| out.push(Problem { at, text });
    if !catalog::is_catalog_id(id) {
        add(
            At::Key(base.clone()),
            format!("{id:?} isn't a board ID: use lowercase letters, digits and hyphens."),
        );
    }
    if let Some(b) = board {
        let f = &b.fpga_id;
        if !catalog::builtin().fpgas.contains_key(f) {
            add(
                At::Value(field("fpga_id")),
                format!("{f:?} isn't an FPGA in the catalog. Use get_catalog to list them."),
            );
        }
        for (at, text) in catalog::pin_group_problems(&b.pin_groups) {
            add(At::Value(field(&format!("pin_groups/{at}"))), text);
        }
    }
    if variant_count == Some(0) {
        add(
            At::Value(field("variants")),
            "A board needs at least one variant.".into(),
        );
    }
    let mut selectors = BTreeSet::new();
    for (i, v) in variants {
        let vf = |f: &str| At::Value(format!("{base}/variants/{i}/{f}"));
        for (dim, _, values) in STANDARD_DIMENSIONS {
            let value = v.value(dim).unwrap_or_default();
            if !values.iter().any(|(x, _, _)| *x == value) {
                let known: Vec<_> = values.iter().map(|(x, _, _)| *x).collect();
                add(
                    vf(&format!("selector/{dim}")),
                    format!(
                        "{value:?} isn't a {dim} value. Known: {}.",
                        known.join(", ")
                    ),
                );
            }
        }
        if !selectors.insert(v.selector.clone()) {
            add(
                vf("selector"),
                format!("Another variant has the same selector ({}).", v.selector),
            );
        }
        if !PROGRAMMERS.contains(&v.programmer.as_str()) {
            add(
                vf("programmer"),
                format!(
                    "Unknown programmer {:?}. Known: {}.",
                    v.programmer,
                    PROGRAMMERS.join(", ")
                ),
            );
        }
        if let Err(e) = catalog::check_template(&v.args) {
            add(vf("args"), format!("{}.", capitalize(&e)));
        }
        for (name, value) in [
            ("vid", Some(&v.usb.vid)),
            ("pid", Some(&v.usb.pid)),
            ("release", v.usb.release.as_ref()),
        ] {
            if value.is_some_and(|x| !catalog::is_usb_id(x)) {
                add(
                    vf(&format!("usb/{name}")),
                    format!("usb {name} must be four lowercase hex digits, such as 0403."),
                );
            }
        }
        for (name, re) in [
            ("product_regex", &v.usb.product_regex),
            ("serial_regex", &v.usb.serial_regex),
        ] {
            if let Some(Err(e)) = re.as_deref().map(regex::Regex::new) {
                add(
                    vf(&format!("usb/{name}")),
                    format!(
                        "Bad {name}: {}.",
                        e.to_string().lines().last().unwrap_or("invalid")
                    ),
                );
            }
        }
    }
    out
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Writes `boards` to a custom boards file in a new temporary folder, for
/// tests: the folder (kept while it lives) and the file's path.
#[cfg(test)]
pub(crate) fn test_file(boards: &serde_json::Value) -> (tempfile::TempDir, String) {
    let tmp = tempfile::tempdir().expect("a temporary folder");
    let path = tmp.path().join("custom-boards.json");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(boards).unwrap_or_default(),
    )
    .expect("writing the file");
    (tmp, path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"{
  "my-board": {
    "description": "My board",
    "fpga_id": "ice40up5k-sg48",
    "variants": [
      {
        "selector": {"connection": "usb", "memory": "flash"},
        "programmer": "iceprog", "args": "-d d:${BUS}/${DEV} ${BITSTREAM}",
        "usb": {"vid": "0403", "pid": "6014"}, "tested": false
      }
    ]
  }
}
"#;

    #[test]
    fn the_schema_key_is_optional_but_must_be_this_one() {
        let named = GOOD.replacen(
            "{\n",
            &format!("{{\n  \"$schema\": \"{}\",\n", catalog::BOARDS_SCHEMA),
            1,
        );
        let boards = parse(&named, "custom-boards.json").unwrap();
        assert_eq!(boards.keys().collect::<Vec<_>>(), ["my-board"]);
        // Another schema, or a future version, is a problem at its value.
        let other = GOOD.replacen(
            "{\n",
            "{\n  \"$schema\": \"https://www.openlogix.org/schemas/logix-boards-v2.json\",\n",
            1,
        );
        assert_eq!(
            diags(&other),
            [(
                Some(2),
                Some(14),
                format!(
                    "Unknown $schema \"https://www.openlogix.org/schemas/logix-boards-v2.json\": this Logix Engine reads {}.",
                    catalog::BOARDS_SCHEMA
                )
            )]
        );
        let not_text = GOOD.replacen("{\n", "{\n  \"$schema\": 1,\n", 1);
        assert_eq!(
            diags(&not_text)[0].2,
            format!(
                "Unknown $schema 1: this Logix Engine reads {}.",
                catalog::BOARDS_SCHEMA
            )
        );
    }

    #[test]
    fn pin_group_problems_point_at_their_values() {
        let with_groups = GOOD.replacen(
            "    \"variants\": [",
            "    \"pin_groups\": [\n      {\"name\": \"sys_clk\", \"type\": \"clock\", \"pins\": [\"35\"]}\n    ],\n    \"variants\": [",
            1,
        );
        assert_eq!(
            diags(&with_groups),
            [(
                Some(6),
                Some(7),
                "A clock needs its frequency, mhz.".to_string()
            )]
        );
        let ok = with_groups.replace("\"type\": \"clock\",", "\"type\": \"clock\", \"mhz\": 12,");
        let boards = parse(&ok, "custom-boards.json").unwrap();
        assert_eq!(boards["my-board"].pin_groups[0].mhz, Some(12.0));
    }

    #[test]
    fn the_log_names_the_file_its_version_and_boards() {
        let ok = parse(GOOD, "custom-boards.json");
        let line = log_line("/p/custom-boards.json", GOOD, &ok);
        assert!(
            line.starts_with("Custom boards file: /p/custom-boards.json (sha256 "),
            "{line}"
        );
        assert!(line.ends_with("), boards: my-board"), "{line}");
        // The same text gives the same short digest; another text another.
        assert_eq!(line, log_line("/p/custom-boards.json", GOOD, &ok));
        assert_ne!(line, log_line("/p/custom-boards.json", "{}", &ok));
        let bad = parse("{", "custom-boards.json");
        assert!(log_line("/p/custom-boards.json", "{", &bad).ends_with(", problems: 1"));
        let none = parse("{}", "custom-boards.json");
        assert!(log_line("/p/c.json", "{}", &none).ends_with("boards: none"));
    }

    fn diags(text: &str) -> Vec<(Option<u32>, Option<u32>, String)> {
        let e = parse(text, "custom-boards.json").unwrap_err();
        assert_eq!(e.category, crate::errors::Category::Request);
        assert!(
            e.message
                .starts_with("The custom boards file has problems. custom-boards.json:"),
            "{}",
            e.message
        );
        e.diagnostics
            .into_iter()
            .map(|d| {
                assert_eq!(d.file.as_deref(), Some("custom-boards.json"));
                (d.line, d.column, d.text)
            })
            .collect()
    }

    #[test]
    fn a_good_file_and_an_empty_one() {
        assert_eq!(parse(GOOD, "f").unwrap().len(), 1);
        assert!(parse(" \n", "f").unwrap().is_empty());
        assert!(parse("{}", "f").unwrap().is_empty());
    }

    #[test]
    fn syntax_and_type_errors_at_their_line_and_column() {
        let d = diags("{\n  \"x\": {\n    \"description\": \"x\",\n  }\n}\n");
        assert_eq!((d[0].0, d[0].1), (Some(4), Some(3)), "{d:?}");
        assert!(d[0].2.starts_with("Trailing comma"), "{d:?}");
        assert!(!d[0].2.contains(" at line "), "{d:?}");
        // A list instead of a map.
        let d = diags("[\n]\n");
        assert_eq!(d[0].0, Some(1), "{d:?}");
        assert!(d[0].2.contains("expected a map"), "{d:?}");
        // A misspelled field.
        let d = diags(&GOOD.replace("\"tested\"", "\"testd\""));
        assert_eq!(d[0].2, "Unknown field \"testd\".", "{d:?}");
        assert_eq!(d[0].0, Some(9), "{d:?}");
    }

    #[test]
    fn definition_errors_point_at_the_value() {
        let bad = GOOD
            .replace("\"0403\"", "\"04G3\"")
            .replace("ice40up5k-sg48", "nope")
            .replace("\"iceprog\"", "\"tinyprog\"");
        let e = parse(&bad, "custom-boards.json").unwrap_err();
        assert!(
            e.message.ends_with(" (and 2 more problems)"),
            "{}",
            e.message
        );
        let d = diags(&bad);
        assert_eq!(d.len(), 3, "{d:?}");
        // fpga_id's value, line 4.
        assert_eq!((d[0].0, d[0].1), (Some(4), Some(16)), "{d:?}");
        assert!(d[0].2.contains("\"nope\" isn't an FPGA"), "{d:?}");
        // programmer, line 8.
        assert_eq!((d[1].0, d[1].1), (Some(8), Some(23)), "{d:?}");
        assert!(
            d[1].2.starts_with("Unknown programmer \"tinyprog\""),
            "{d:?}"
        );
        // usb vid, line 9.
        assert_eq!((d[2].0, d[2].1), (Some(9), Some(24)), "{d:?}");
        assert!(d[2].2.starts_with("usb vid must be"), "{d:?}");
    }

    #[test]
    fn selector_problems_point_at_the_selector() {
        // An unknown memory value: at the memory value, line 7.
        let d = diags(&GOOD.replace("\"memory\": \"flash\"", "\"memory\": \"eeprom\""));
        assert_eq!(d.len(), 1, "{d:?}");
        assert_eq!(d[0].0, Some(7), "{d:?}");
        assert!(
            d[0].2
                .starts_with("\"eeprom\" isn't a memory value. Known: flash, ram."),
            "{d:?}"
        );
        // The same selector twice: at the second one, line 12.
        let variant = "      {\n        \"selector\": {\"connection\": \"usb\", \"memory\": \"flash\"},\n        \"programmer\": \"iceprog\", \"args\": \"${BITSTREAM}\",\n        \"usb\": {\"vid\": \"0403\", \"pid\": \"6014\"}, \"tested\": false\n      }";
        let twice = GOOD.replace("      }\n    ]", &format!("      }},\n{variant}\n    ]"));
        let d = diags(&twice);
        assert_eq!(d.len(), 1, "{d:?}");
        assert_eq!((d[0].0, d[0].1), (Some(12), Some(21)), "{d:?}");
        assert_eq!(
            d[0].2,
            "Another variant has the same selector (connection usb, memory flash)."
        );
        // The old shape, with an ID: an unknown field.
        let d = diags(&GOOD.replace("\"selector\": {", "\"id\": \"usb-flash\", \"selector\": {"));
        assert_eq!(d[0].2, "Unknown field \"id\".", "{d:?}");
        // A selector needs both dimensions.
        let d = diags(&GOOD.replace("\"connection\": \"usb\", ", ""));
        assert!(d[0].2.starts_with("Missing field `connection`"), "{d:?}");
    }

    /// Two boards, in the file in the opposite order to their IDs, with
    /// problems of every kind: shape and content, in boards and variants.
    const SEVERAL: &str = r#"{
  "z-board": {
    "description": "Z",
    "fpga_id": "nope",
    "variants": [
      {
        "selector": {"connection": "usb", "memory": "flash"},
        "programer": "iceprog", "args": "${BITSTREAM}",
        "usb": {"vid": "0403", "pid": "6014"}, "tested": false
      },
      {
        "selector": {"connection": "usb", "memory": "ram"},
        "programmer": "iceprog", "args": "${BITSTREAM}",
        "usb": {"vid": "04G3", "pid": "6014"}, "tested": false
      },
      {"selector": {"connection": "usb", "memory": "flash"}, "programmer": 7}
    ]
  },
  "a-board": {
    "description": 3,
    "fpga_id": "ice40up5k-sg48",
    "variants": [
      {
        "selector": {"connection": "usb", "memory": "flash"},
        "programmer": "icepro", "args": "${BITSTREAM}",
        "usb": {"vid": "0403", "pid": "6014"}, "tested": false
      }
    ]
  }
}
"#;

    #[test]
    fn every_problem_at_once_in_file_order() {
        let d = diags(SEVERAL);
        let got: Vec<_> = d
            .iter()
            .map(|(l, c, t)| {
                (
                    l.unwrap(),
                    c.unwrap(),
                    t.split(['.', ',']).next().unwrap().to_string(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                // z-board: its FPGA, then each variant on its own.
                (4, 16, "\"nope\" isn't an FPGA in the catalog".to_string()),
                (8, 19, "Unknown field \"programer\"".to_string()),
                (
                    14,
                    24,
                    "usb vid must be four lowercase hex digits".to_string()
                ),
                (16, 76, "Invalid type: integer `7`".to_string()),
                // a-board: a wrong type in the board doesn't hide its variant.
                (20, 20, "Invalid type: integer `3`".to_string()),
                (25, 23, "Unknown programmer \"icepro\"".to_string()),
            ],
            "{d:?}"
        );
        let e = parse(SEVERAL, "custom-boards.json").unwrap_err();
        assert!(
            e.message
                .starts_with("The custom boards file has problems. custom-boards.json:4:16: "),
            "{}",
            e.message
        );
        assert!(
            e.message.ends_with(" (and 5 more problems)"),
            "{}",
            e.message
        );
    }

    #[test]
    fn at_most_ten_problems_then_how_many_more() {
        let boards: Vec<String> = (0..13)
            .map(|i| format!("  \"Board {i}\": {{\"description\": \"x\", \"fpga_id\": \"ice40up5k-sg48\", \"variants\": []}}"))
            .collect();
        let text = format!("{{\n{}\n}}\n", boards.join(",\n"));
        // Each board: a bad ID and no variants.
        let e = parse(&text, "custom-boards.json").unwrap_err();
        assert_eq!(e.diagnostics.len(), MAX_PROBLEMS + 1, "{:?}", e.diagnostics);
        assert!(
            e.diagnostics[..MAX_PROBLEMS]
                .iter()
                .all(|d| d.severity == Severity::Error)
        );
        // In file order: board 0's two problems first.
        assert_eq!(e.diagnostics[0].line, Some(2));
        assert_eq!(e.diagnostics[1].line, Some(2));
        assert_eq!(e.diagnostics[9].line, Some(6));
        let last = &e.diagnostics[MAX_PROBLEMS];
        assert_eq!(last.severity, Severity::Info);
        assert_eq!((last.line, last.column), (None, None));
        assert_eq!(
            last.text,
            "16 more problems not listed. Fix these first, then check again."
        );
        assert!(
            e.message.ends_with(" (and 25 more problems)"),
            "{}",
            e.message
        );
    }

    #[test]
    fn a_board_that_isnt_an_object_and_variants_that_arent_a_list() {
        let d = diags(
            "{\n  \"a\": 5,\n  \"b\": {\"description\": \"b\", \"fpga_id\": \"ice40up5k-sg48\", \"variants\": {}}\n}\n",
        );
        assert_eq!(d.len(), 2, "{d:?}");
        assert_eq!((d[0].0, d[0].1), (Some(2), Some(8)), "{d:?}");
        assert!(d[0].2.starts_with("Invalid type: integer `5`"), "{d:?}");
        assert_eq!((d[1].0, d[1].1), (Some(3), Some(69)), "{d:?}");
        assert!(d[1].2.starts_with("Invalid type: map"), "{d:?}");
    }

    #[test]
    fn a_bad_board_id_points_at_its_key() {
        let d = diags(&GOOD.replace("\"my-board\"", "\"My Board\""));
        assert_eq!((d[0].0, d[0].1), (Some(2), Some(3)), "{d:?}");
        assert!(d[0].2.contains("isn't a board ID"), "{d:?}");
    }

    #[test]
    fn missing_fpga_points_at_the_board() {
        // At the board's object, where the field belongs, not at its end.
        let d = diags(&GOOD.replace("\"fpga_id\": \"ice40up5k-sg48\",", ""));
        assert_eq!((d[0].0, d[0].1), (Some(2), Some(15)), "{d:?}");
        assert_eq!(d[0].2, "Missing field `fpga_id`.");
        // A full FPGA definition isn't a field: catalog FPGAs only.
        let d = diags(&GOOD.replace(
            "\"fpga_id\": \"ice40up5k-sg48\",",
            "\"fpga_id\": \"ice40up5k-sg48\", \"fpga\": {},",
        ));
        assert_eq!(d[0].2, "Unknown field \"fpga\".");
    }

    #[test]
    fn files_are_read_by_absolute_path_and_named_as_given() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("proj");
        std::fs::create_dir(&src).unwrap();
        let file = src.join("custom-boards.json");
        std::fs::write(&file, GOOD.replace("\"0403\"", "\"x\"")).unwrap();
        let given = file.to_string_lossy().into_owned();
        let e = load(Some(&given)).unwrap_err();
        assert_eq!(e.diagnostics[0].file.as_deref(), Some(given.as_str()));
        assert!(
            load(Some("custom-boards.json"))
                .unwrap_err()
                .message
                .contains("absolute")
        );
        let missing = tmp.path().join("none.json").to_string_lossy().into_owned();
        assert!(
            load(Some(&missing))
                .unwrap_err()
                .message
                .contains("can't read")
        );
        assert!(load(None).unwrap().is_empty());
    }
}
