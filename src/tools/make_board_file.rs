//! The make_board_file tool (docs/tool-api.md): the engine side of the Board
//! Wizard. Makes a board file from a connected device and the catalog board
//! the user says it is (docs/requirements.md, Board files).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::EngineTool;
use crate::board_file;
use crate::catalog;
use crate::devices::{self, Device};
use crate::errors::ToolError;
use crate::state::EngineState;

pub struct MakeBoardFile;

impl EngineTool for MakeBoardFile {
    const NAME: &'static str = "make_board_file";
    const DESCRIPTION: &'static str = "Makes a board file: the user's own board, from a \
        connected device and the catalog board the user says it is, for a device that \
        list_devices doesn't match to that board, or only weakly (the Board Wizard). The \
        board file is for the frontend to store, typically in the project, and to send back \
        unchanged as the board of build, lint, test and simulate, and in board_files of \
        list_devices and get_catalog: never read or change its contents, which only the \
        engine understands and may change between versions. It matches only devices like \
        this one, and with pin_serial_number only this exact device, to tell identical \
        boards apart. Show the notes to the user. Nothing is written to disk.";
    const READ_ONLY: bool = true;
    type Input = MakeBoardFileInput;
    type Output = MakeBoardFileOutput;
}

/// The device, and the board it is.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MakeBoardFileInput {
    /// The device's ID, from list_devices.
    pub device: String,
    /// The catalog board the user says the device is, such as
    /// arty-a7-100t. get_catalog lists them.
    #[schemars(pattern(r"^[a-z0-9-]+$"))]
    pub board: String,
    /// The board's name for people, such as "My Arty". Omitted means the
    /// catalog board's name with "(my board)".
    #[serde(default)]
    pub name: Option<String>,
    /// Match only this exact device, by its serial number, to tell it apart
    /// from identical boards connected at the same time. The board file then
    /// holds the serial number. Defaults to false.
    #[serde(default)]
    pub pin_serial_number: bool,
}

/// The board file.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MakeBoardFileOutput {
    /// The board file, to store and send back unchanged. Its contents are
    /// the engine's own.
    #[schemars(with = "serde_json::Map<String, serde_json::Value>")]
    pub board_file: serde_json::Value,
    /// The board's ID, as results name it, such as arty-a7-100t-3f9a2c. The
    /// same device and board give the same ID.
    pub board_id: String,
    /// The board's name for people.
    pub name: String,
    /// Things the user should know, such as that the device has no product
    /// name, to show as they are.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

pub fn run_with(
    input: MakeBoardFileInput,
    scan: impl FnOnce() -> Result<Vec<Device>, ToolError>,
) -> Result<MakeBoardFileOutput, ToolError> {
    let board_name = catalog::builtin()
        .boards
        .get(&input.board)
        .map(|b| b.description.clone())
        .unwrap_or_else(|| "board".into());
    let connected = scan()?;
    let device = devices::find(&connected, &input.device, &board_name)?;
    let made = board_file::make(
        &device,
        &connected,
        &input.board,
        input.name.as_deref(),
        input.pin_serial_number,
    )?;
    Ok(MakeBoardFileOutput {
        board_id: made.file.id.clone(),
        name: made.file.name.clone(),
        board_file: made.file.to_value(),
        notes: made.notes,
    })
}

pub fn run(
    _state: &EngineState,
    input: MakeBoardFileInput,
) -> Result<MakeBoardFileOutput, ToolError> {
    run_with(input, devices::scan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::tests::device;
    use serde_json::json;

    fn make(
        input: serde_json::Value,
        devices: Vec<Device>,
    ) -> Result<MakeBoardFileOutput, ToolError> {
        let input: MakeBoardFileInput = serde_json::from_value(input).unwrap();
        run_with(input, move || Ok(devices))
    }

    #[test]
    fn makes_a_board_file_for_a_listed_device() {
        let d = device(
            "0403",
            "6010",
            Some("Digilent USB Device"),
            Some("210319AB51BBA"),
            4,
        );
        let out = make(
            json!({"device": d.id, "board": "arty-a7-100t", "name": "My Arty"}),
            vec![d.clone()],
        )
        .unwrap();
        assert_eq!(out.name, "My Arty");
        assert!(out.board_id.starts_with("arty-a7-100t-"));
        assert_eq!(out.board_file["id"], out.board_id);
        assert!(out.notes.is_empty());

        // A device that isn't connected anymore, and a bad board.
        let gone = make(json!({"device": d.id, "board": "arty-a7-100t"}), vec![]).unwrap_err();
        assert_eq!(gone.category, crate::errors::Category::Setup);
        let bad = make(json!({"device": d.id, "board": "nope"}), vec![d]).unwrap_err();
        assert_eq!(bad.category, crate::errors::Category::Request);
    }
}
