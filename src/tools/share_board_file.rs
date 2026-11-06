//! The share_board_file tool (docs/tool-api.md): a board file's report, as
//! text for the user to paste into a GitHub issue (docs/requirements.md,
//! Board files). The engine never goes online for it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::EngineTool;
use crate::board_file::BoardFile;
use crate::errors::ToolError;
use crate::state::EngineState;

/// Where reports go: the engine's issues, which maintainers turn into
/// catalog entries.
pub const NEW_ISSUE_URL: &str = "https://github.com/openlogix/logix-engine/issues/new";

pub struct ShareBoardFile;

impl EngineTool for ShareBoardFile {
    const NAME: &'static str = "share_board_file";
    const DESCRIPTION: &'static str = "Returns a board file's report, for the user to share \
        with the Logix Engine's maintainers so the catalog can learn the board: a title and a \
        body to paste into a new GitHub issue at new_issue_url (a GitHub account is needed). \
        Show the text, with a way to copy it and open the link, before anything is shared: \
        it holds what the board's device reports (its serial number only as a shape), the \
        catalog board it was made from, and the ways of programming it that worked. The \
        engine sends nothing anywhere.";
    const READ_ONLY: bool = true;
    type Input = ShareBoardFileInput;
    type Output = ShareBoardFileOutput;
}

/// The board file to report.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShareBoardFileInput {
    /// The board file, as make_board_file or program_board returned it,
    /// sent unchanged.
    #[schemars(with = "serde_json::Map<String, serde_json::Value>")]
    pub board_file: serde_json::Value,
}

/// The report.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShareBoardFileOutput {
    /// The issue's title.
    pub title: String,
    /// The issue's body, in Markdown.
    pub body: String,
    /// The page that opens a new issue, to paste the title and body into.
    pub new_issue_url: String,
}

pub fn report(file: &BoardFile) -> ShareBoardFileOutput {
    let made = &file.made_from;
    let dev = &made.device;
    let base = crate::catalog::builtin()
        .boards
        .get(&made.board)
        .map(|b| b.description.clone())
        .unwrap_or_else(|| made.board.clone());
    let product = dev.product.as_deref().unwrap_or("(none)");
    let title = format!(
        "Board report: {base}, device {}:{} \"{product}\"",
        dev.vid, dev.pid
    );
    let cell = |v: Option<&str>| v.unwrap_or("(none)").replace('|', "\\|");
    let pinned = file.variants.iter().any(|v| v.usb.serial_regex.is_some());
    let mut body = format!(
        "A board made with the Board Wizard. The device was programmed as this catalog board.\n\n\
         | | |\n|---|---|\n\
         | Catalog board | {base} (`{}`) |\n\
         | Name given | {} |\n\
         | USB IDs | {}:{} |\n\
         | Release (bcdDevice) | {} |\n\
         | Manufacturer | {} |\n\
         | Product | {} |\n\
         | Serial number shape | {} |\n\
         | Pinned by serial number | {} |\n\
         | Made with engine | {} |\n\n",
        made.board,
        file.name.replace('|', "\\|"),
        dev.vid,
        dev.pid,
        cell(dev.release.as_deref()),
        cell(dev.manufacturer.as_deref()),
        cell(dev.product.as_deref()),
        cell(dev.serial_shape.as_deref()),
        if pinned { "yes (not included)" } else { "no" },
        made.engine,
    );
    if file.verified.is_empty() {
        body.push_str("Verified uploads: none yet (not programmed with this board file).\n");
    } else {
        body.push_str("Verified uploads:\n\n");
        for v in &file.verified {
            let fpga = v
                .fpga_idcode
                .as_deref()
                .map(|c| format!(", FPGA ID code {c} confirmed"))
                .unwrap_or_default();
            body.push_str(&format!("- `{}`{fpga} (engine {})\n", v.variant, v.engine));
        }
    }
    ShareBoardFileOutput {
        title,
        body,
        new_issue_url: NEW_ISSUE_URL.into(),
    }
}

pub fn run(
    _state: &EngineState,
    input: ShareBoardFileInput,
) -> Result<ShareBoardFileOutput, ToolError> {
    let file = BoardFile::from_value(&input.board_file).map_err(|mut e| {
        e.message = format!("board_file: {}", e.message);
        e
    })?;
    Ok(report(&file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::tests::device;

    #[test]
    fn a_report_without_the_serial_number() {
        let mut d = device(
            "0403",
            "6010",
            Some("Digilent USB Device"),
            Some("210319AB51BBA"),
            4,
        );
        d.release = Some("0700".into());
        d.manufacturer = Some("Digilent".into());
        let mut file = crate::board_file::make(&d, &[], "arty-a7-100t", Some("My | Arty"), true)
            .unwrap()
            .file;
        let r = report(&file);
        assert_eq!(
            r.title,
            "Board report: Arty A7-100T (Xilinx, Artix7), device 0403:6010 \"Digilent USB Device\""
        );
        assert!(
            r.body.contains("| Release (bcdDevice) | 0700 |"),
            "{}",
            r.body
        );
        assert!(
            r.body.contains("| Serial number shape | 210319####### |"),
            "{}",
            r.body
        );
        assert!(
            r.body
                .contains("| Pinned by serial number | yes (not included) |")
        );
        assert!(r.body.contains("My \\| Arty"), "table-safe");
        assert!(r.body.contains("none yet"));
        assert!(!r.body.contains("210319AB51BBA"), "{}", r.body);
        assert!(!r.title.contains("210319AB51BBA"));
        assert_eq!(r.new_issue_url, NEW_ISSUE_URL);

        file.record_upload("usb-ram", Some("0x03631093"));
        let r = report(&file);
        assert!(
            r.body
                .contains("- `usb-ram`, FPGA ID code 0x03631093 confirmed (engine "),
            "{}",
            r.body
        );
    }
}
