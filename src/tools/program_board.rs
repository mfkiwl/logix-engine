//! The program_board tool (docs/tool-api.md, Tables 9 and 10).

use std::sync::atomic::{AtomicBool, Ordering};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Call, EngineTool};
use crate::build::{self, TOOLCHAIN};
use crate::devices::{self, Device};
use crate::errors::{ActionCode, Category, ToolError};
use crate::paths;
use crate::program::{self, ProgressLine};
use crate::progress::{OutputLevel, Phases};
use crate::runner::{RunError, command_line, run_tool_with};
use crate::state::EngineState;

pub struct ProgramBoard;

impl EngineTool for ProgramBoard {
    const NAME: &'static str = "program_board";
    const DESCRIPTION: &'static str = "Programs the bitstream of a successful build into the \
        board it was built for. It doesn't build: call build first, and program_board only if \
        the build succeeded. The board comes from the build, so it takes the build folder and \
        the device to program: the id of a device from list_devices whose boards include the \
        build's board (ask the user when several do). A device that isn't connected anymore is a \
        setup error with connect_board; list the devices again. Most boards are programmed to \
        flash, so the design stays after power-off.";
    const READ_ONLY: bool = false;
    const OPEN_WORLD: bool = true;
    type Input = ProgramBoardInput;
    type Output = ProgramBoardOutput;
}

/// Which build to program, and into which device.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProgramBoardInput {
    /// The build folder of a successful build, as an absolute path.
    pub build_folder: String,
    /// The device to program: the id of a device from list_devices, such as
    /// one whose boards include the build's board. It must still be
    /// connected and match the board's USB filter.
    pub device: String,
    /// A custom programmer command that replaces the board's default one,
    /// like Apio's programmer_cmd. Its first word is a program from the
    /// toolchain package, such as openFPGALoader, or an absolute path. It may
    /// use ${VID}, ${PID}, ${BUS}, ${DEV}, ${SERIAL} and must use
    /// ${BITSTREAM}.
    #[serde(default)]
    pub programmer_command: Option<String>,
    /// Let the programmer print more detail, shown in the live output and
    /// the logs. Not applied to a programmer_command. Defaults to false.
    #[serde(default)]
    pub verbose: bool,
}

/// What was programmed.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProgramBoardOutput {
    /// The device that was programmed.
    pub device: Device,
    /// The programmer that was used, such as openfpgaloader, or custom for a
    /// programmer_command.
    pub programmer: String,
    /// The programs this call ran, in order, each with the exact command and
    /// a shell line that runs it again without the engine, such as for a bug
    /// report to the tool's developers.
    pub commands: Vec<crate::commands::CommandRecord>,
}

pub async fn run_with(
    state: &EngineState,
    input: ProgramBoardInput,
    call: &Call,
    scan: impl FnOnce() -> Result<Vec<Device>, ToolError>,
) -> Result<ProgramBoardOutput, ToolError> {
    let folder =
        paths::check_build_folder(&input.build_folder, paths::home_folder().as_deref(), None)
            .map_err(|e| ToolError::request(format!("build_folder: {e}.")))?;
    let record = build::read_record(&folder).ok_or_else(|| {
        ToolError::request(format!(
            "{} has no build record from a successful build. Run build first.",
            input.build_folder
        ))
    })?;
    if record.board.checksum() != record.board_checksum {
        return Err(ToolError::request(
            "The build record is damaged. Run build again.",
        ));
    }
    let bitstream = folder.join(&record.result.bitstream);
    if !bitstream.is_file() {
        return Err(ToolError::request(format!(
            "The bitstream {} is missing. Run build again.",
            bitstream.display()
        )));
    }
    let board = &record.board;
    let variant = &board.configuration;
    let custom = input.programmer_command.as_deref();

    let needed = state.packages.needing_install(&[TOOLCHAIN])?;
    let steps = if custom.is_some() { 2 } else { 3 };
    let phases = Phases::new(call.progress.clone(), needed.len() + steps);
    state
        .packages
        .ensure(&needed, state.auto_install, &phases, &call.cancel)
        .await?;
    let spec = state
        .packages
        .manifest()
        .get(TOOLCHAIN)
        .cloned()
        .ok_or_else(|| ToolError::engine(format!("{TOOLCHAIN} isn't in the manifest.")))?;
    let bin = state.packages.package_dir(&spec).join("bin");

    // Say how the board will be programmed (not for a custom command, which
    // may do anything), and any note for users, such as holding a button.
    if custom.is_none() {
        if let Some(plan) = program::plan(&board.description, variant) {
            call.output.message(&plan);
        }
        if let Some(notes) = &variant.user_notes {
            call.output.message(&format!("Note: {notes}"));
        }
    }
    phases.start("Finding device");
    let connected = scan()?;
    let device = devices::find(&connected, &input.device, &board.name)?;
    if !devices::matches(&device, &variant.usb) {
        return Err(ToolError::request(format!(
            "device: {} isn't a {}: it doesn't match the board's USB filter. Choose a device whose boards in list_devices include the build's board.",
            device_name(&device),
            board.name
        )));
    }
    program::check_alone(
        custom.unwrap_or(&variant.args),
        &connected,
        &device,
        &board.name,
    )?;
    let (program, args) =
        program::command(variant, custom, &device, &bitstream, &bin, input.verbose)?;
    let programmer = match custom {
        None => variant.programmer.clone(),
        Some(_) => "custom".to_string(),
    };

    // Progress: write, then verify. A custom command's output isn't read.
    let verifying = AtomicBool::new(false);
    let tool_name = program
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let on_line = |line: &str| {
        call.output.line(&tool_name, OutputLevel::of(line), line);
        if custom.is_some() {
            return;
        }
        match program::progress_line(line) {
            ProgressLine::Percent(p) => phases.percent(p),
            ProgressLine::Verifying(p) => {
                if !verifying.swap(true, Ordering::SeqCst) {
                    phases.start("Verifying");
                }
                if let Some(p) = p {
                    phases.percent(p);
                }
            }
            ProgressLine::Other => {}
        }
    };
    phases.start(if custom.is_some() {
        "Programming"
    } else {
        program::writing_phase(variant)
    });
    call.output
        .command(&tool_name, &command_line(&program, &args));
    call.commands
        .push(crate::commands::record(&program, &args, &folder, &bin));
    let run = run_tool_with(&program, &args, &folder, &bin, &call.cancel, &on_line).await;
    match run {
        Ok(r) if r.status.success() => {
            if custom.is_none()
                && let Some(done) = program::done(&board.description, variant)
            {
                call.output.message(&done);
            }
            Ok(ProgramBoardOutput {
                device,
                programmer,
                commands: call.commands.take(),
            })
        }
        Ok(r) => Err(program::failure(
            &program.file_name().unwrap_or_default().to_string_lossy(),
            &board.name,
            &r.output,
            &r.status.to_string(),
        )),
        Err(RunError::Cancelled) => Err(ToolError::cancelled("Programming was cancelled.")),
        Err(RunError::Spawn(e)) => Err(ToolError::new(
            Category::Tool,
            format!("Can't start {}: {e}", program.display()),
        )
        .with_action(
            ActionCode::InstallPackages,
            format!("Reinstall the {TOOLCHAIN} package, which may be damaged."),
        )),
    }
}

pub async fn run(
    state: &EngineState,
    input: ProgramBoardInput,
    call: &Call,
) -> Result<ProgramBoardOutput, ToolError> {
    run_with(state, input, call, devices::scan).await
}

/// A device for messages, such as "Dual RS232-HS (0403:6010)".
fn device_name(d: &Device) -> String {
    let name = d.product.as_deref().unwrap_or("The device");
    format!("{name} ({}:{})", d.vid, d.pid)
}
