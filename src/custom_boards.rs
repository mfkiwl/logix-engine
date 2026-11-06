//! The custom boards file (docs/tool-api.md, Board specification): every
//! tool that deals with boards takes `custom_boards_file`, the absolute path
//! of a JSON file that maps board IDs to definitions in the catalog's
//! format. The engine reads only the file the request names, and reports
//! every problem in it as a diagnostic at file, line and column, as it
//! reports problems in source files.

use std::collections::BTreeSet;
use std::path::Path;

use crate::board::{CustomBoard, CustomBoards};
use crate::catalog::{self, PROGRAMMERS, STANDARD_DIMENSIONS};
use crate::errors::{Diagnostic, Severity, ToolError};
use crate::json_pos::{PositionIndex, segment};

/// The custom boards of a request: none without a file, else the file's
/// boards when it has no problems. A file that can't be read, isn't valid
/// JSON or has an invalid board is a request error whose diagnostics name
/// each problem. Diagnostics name the file relative to `source_folder` when
/// it's inside it, else as the request gave it.
pub fn load(file: Option<&str>, source_folder: Option<&Path>) -> Result<CustomBoards, ToolError> {
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
    let name = source_folder
        .and_then(|src| {
            let src = crate::paths::resolve(src).ok()?;
            real.strip_prefix(&src)
                .ok()
                .map(|r| r.to_string_lossy().replace('\\', "/"))
        })
        .unwrap_or_else(|| given.to_string());
    parse(&text, &name)
}

/// The boards in a custom boards file's `text`, or a request error with a
/// diagnostic for each problem, in file `name`.
pub fn parse(text: &str, name: &str) -> Result<CustomBoards, ToolError> {
    let at = |line: Option<u32>, column: Option<u32>, text: String| Diagnostic {
        file: Some(name.to_string()),
        line,
        column,
        severity: Severity::Error,
        text,
        tool: None,
        time_ns: None,
    };
    if text.trim().is_empty() {
        return Ok(CustomBoards::new());
    }
    let boards: CustomBoards = match serde_json::from_str(text) {
        Ok(b) => b,
        Err(e) => {
            let line = u32::try_from(e.line()).ok().filter(|l| *l > 0);
            let column = u32::try_from(e.column()).ok().filter(|c| *c > 0);
            return Err(problems(name, vec![at(line, column, serde_message(&e))]));
        }
    };
    let index = PositionIndex::new(text).unwrap_or_default();
    let found: Vec<Diagnostic> = boards
        .iter()
        .flat_map(|(id, b)| board_problems(id, b))
        .map(|p| {
            let pos = match p.at {
                At::Key(path) => index.key(&path),
                At::Value(path) => index.nearest(&path),
            };
            at(pos.map(|p| p.0), pos.map(|p| p.1), p.text)
        })
        .collect();
    if found.is_empty() {
        Ok(boards)
    } else {
        Err(problems(name, found))
    }
}

/// The request error for a file's problems.
fn problems(name: &str, diagnostics: Vec<Diagnostic>) -> ToolError {
    let first = &diagnostics[0];
    let place = match (first.line, first.column) {
        (Some(l), Some(c)) => format!("{name}:{l}:{c}"),
        (Some(l), None) => format!("{name}:{l}"),
        _ => name.to_string(),
    };
    let more = match diagnostics.len() {
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
fn serde_message(e: &serde_json::Error) -> String {
    let text = e.to_string();
    let text = match text.rfind(" at line ") {
        Some(i) => text[..i].to_string(),
        None => text,
    };
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

/// Every problem with one board: its ID, its FPGA and its variants.
fn board_problems(id: &str, b: &CustomBoard) -> Vec<Problem> {
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
    match (&b.fpga_id, &b.fpga) {
        (Some(f), None) => {
            if !catalog::builtin().fpgas.contains_key(f) {
                add(
                    At::Value(field("fpga_id")),
                    format!(
                        "{f:?} isn't an FPGA in the catalog. Use get_catalog to list them, or give a full fpga definition."
                    ),
                );
            }
        }
        (None, Some(f)) => {
            if let Err(e) = f.validate() {
                add(
                    At::Value(field("fpga")),
                    format!("Invalid FPGA definition: {e}."),
                );
            }
        }
        _ => add(
            At::Key(base.clone()),
            format!("Board {id:?}: give exactly one of fpga_id and fpga."),
        ),
    }
    if b.variants.is_empty() {
        add(
            At::Value(field("variants")),
            "A board needs at least one variant.".into(),
        );
    }
    let mut ids = BTreeSet::new();
    let mut pairs = BTreeSet::new();
    for (i, v) in b.variants.iter().enumerate() {
        let vf = |f: &str| At::Value(format!("{base}/variants/{i}/{f}"));
        if !catalog::is_catalog_id(&v.id) {
            add(
                vf("id"),
                format!(
                    "Variant ID {:?}: use lowercase letters, digits and hyphens.",
                    v.id
                ),
            );
        } else if !ids.insert(v.id.clone()) {
            add(vf("id"), format!("Another variant has the ID {:?}.", v.id));
        }
        for (dim, _, values) in STANDARD_DIMENSIONS {
            let value = v.value(dim).unwrap_or_default();
            if !values.iter().any(|(x, _, _)| *x == value) {
                let known: Vec<_> = values.iter().map(|(x, _, _)| *x).collect();
                add(
                    vf(dim),
                    format!(
                        "{value:?} isn't a {dim} value. Known: {}.",
                        known.join(", ")
                    ),
                );
            }
        }
        if !pairs.insert((v.connection.clone(), v.memory.clone())) {
            add(
                vf("connection"),
                format!(
                    "Another variant has the same connection {} and memory {}.",
                    v.connection, v.memory
                ),
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
        "id": "usb-flash", "connection": "usb", "memory": "flash",
        "programmer": "iceprog", "args": "-d d:${BUS}/${DEV} ${BITSTREAM}",
        "usb": {"vid": "0403", "pid": "6014"}, "tested": false
      }
    ]
  }
}
"#;

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
        assert!(d[0].2.starts_with("Unknown field `testd`"), "{d:?}");
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
    fn a_bad_board_id_points_at_its_key() {
        let d = diags(&GOOD.replace("\"my-board\"", "\"My Board\""));
        assert_eq!((d[0].0, d[0].1), (Some(2), Some(3)), "{d:?}");
        assert!(d[0].2.contains("isn't a board ID"), "{d:?}");
    }

    #[test]
    fn missing_fpga_points_at_the_board() {
        let d = diags(&GOOD.replace("\"fpga_id\": \"ice40up5k-sg48\",", ""));
        assert_eq!((d[0].0, d[0].1), (Some(2), Some(3)), "{d:?}");
        assert!(d[0].2.contains("exactly one of fpga_id and fpga"), "{d:?}");
    }

    #[test]
    fn files_are_read_by_absolute_path_and_named_from_the_source_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("proj");
        std::fs::create_dir(&src).unwrap();
        let file = src.join("custom-boards.json");
        std::fs::write(&file, GOOD.replace("\"0403\"", "\"x\"")).unwrap();
        let given = file.to_string_lossy().into_owned();
        let e = load(Some(&given), Some(&src)).unwrap_err();
        assert_eq!(e.diagnostics[0].file.as_deref(), Some("custom-boards.json"));
        let e = load(Some(&given), None).unwrap_err();
        assert_eq!(e.diagnostics[0].file.as_deref(), Some(given.as_str()));
        assert!(
            load(Some("custom-boards.json"), None)
                .unwrap_err()
                .message
                .contains("absolute")
        );
        let missing = tmp.path().join("none.json").to_string_lossy().into_owned();
        assert!(
            load(Some(&missing), None)
                .unwrap_err()
                .message
                .contains("can't read")
        );
        assert!(load(None, None).unwrap().is_empty());
    }
}
