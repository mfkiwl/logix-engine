//! The graph tool's work: yosys draws the design's top module as a Graphviz
//! .dot file and Graphviz's dot renders it, with the standard commands
//! (docs/tool-api.md, "graph tool").

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::build::parse::parse_yosys;
use crate::build::{run_logged, tool_failed};
use crate::errors::{Category, Diagnostic, Severity, ToolError};
use crate::progress::Phases;
use crate::sim::toolchain;
use crate::state::EngineState;
use crate::tools::Call;

/// The file format of the graph.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GraphFormat {
    /// SVG, which browsers and web views show and scale.
    #[default]
    Svg,
    /// PDF.
    Pdf,
    /// PNG, a bitmap.
    Png,
}

impl GraphFormat {
    pub fn extension(self) -> &'static str {
        match self {
            GraphFormat::Svg => "svg",
            GraphFormat::Pdf => "pdf",
            GraphFormat::Png => "png",
        }
    }
}

/// A checked graph request.
#[derive(Debug, Clone)]
pub struct GraphRequest {
    /// The real path of the source folder.
    pub source_folder: PathBuf,
    /// The design files, relative to the source folder.
    pub source_files: Vec<String>,
    /// The real path of the build folder.
    pub build_folder: PathBuf,
    pub top_module: String,
    pub defines: BTreeMap<String, String>,
    pub format: GraphFormat,
    pub verbose: bool,
}

/// What graph made.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphResult {
    /// The graph, relative to the build folder, such as graph.svg.
    pub graph_file: String,
    /// Yosys warnings while reading the design.
    pub diagnostics: Vec<Diagnostic>,
}

/// The file names yosys and dot write in the build folder: fixed names.
const STEM: &str = "graph";

/// Quotes a path for a yosys script.
fn yosys_quote(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

/// Yosys's arguments: the standard command, run in the build folder so the .dot
/// file's prefix needs no quoting (yosys keeps quotes in it), with the
/// design files as absolute paths.
pub fn yosys_args(req: &GraphRequest) -> Vec<OsString> {
    let files: Vec<String> = req
        .source_files
        .iter()
        .map(|f| yosys_quote(&req.source_folder.join(f)))
        .collect();
    let mut args: Vec<OsString> = vec![
        "-p".into(),
        format!(
            "read_verilog -sv {}; show -format dot -colors 1 -wireshape plaintext -prefix {STEM} {}",
            files.join(" "),
            req.top_module
        )
        .into(),
        "-DSYNTHESIZE".into(),
    ];
    for (name, value) in &req.defines {
        args.push(format!("-D{name}={value}").into());
    }
    if !req.verbose {
        args.push("-q".into());
    }
    args
}

/// dot's arguments: the standard command.
pub fn dot_args(format: GraphFormat) -> Vec<OsString> {
    let ext = format.extension();
    vec![
        format!("-T{ext}").into(),
        format!("{STEM}.dot").into(),
        "-o".into(),
        format!("{STEM}.{ext}").into(),
    ]
}

/// Yosys's message when the selection names no module.
const NO_MODULE: &str = "did not match any module";

/// The error for a yosys run that failed: a design error with the problems
/// it reported, or a tool error when it reported none.
fn yosys_error(run: &crate::runner::ToolRun, req: &GraphRequest) -> ToolError {
    let src = req.source_folder.to_string_lossy();
    let mut errors = parse_yosys(&run.output, &src).errors;
    if run.output.contains(NO_MODULE) {
        // Yosys says "Nothing there to show", which doesn't say why.
        errors = vec![Diagnostic {
            file: None,
            line: None,
            column: None,
            severity: Severity::Error,
            text: format!(
                "The top module {} isn't in the design files.",
                req.top_module
            ),
            tool: Some("yosys".into()),
            time_ns: None,
        }];
    }
    let failed = tool_failed("yosys", "reading the design", run);
    let Some(first) = errors.first() else {
        return failed;
    };
    let place = match (&first.file, first.line) {
        (Some(f), Some(l)) => format!("{f}:{l}: "),
        _ => String::new(),
    };
    let mut err = ToolError::new(
        Category::Design,
        format!("The design has errors. {place}{}", first.text),
    );
    err.details = failed.details;
    err.diagnostics = errors;
    err
}

/// Draws the graph.
pub async fn graph(
    state: &EngineState,
    req: GraphRequest,
    call: &Call,
) -> Result<GraphResult, ToolError> {
    let needed = state.packages.needing_download(&[crate::packages::CORE])?;
    let phases = Phases::new(call.progress.clone(), needed.len() + 2);
    let (bin, _) = toolchain(state, &needed, &phases, call).await?;
    graph_with(&bin, &req, call, &phases).await
}

/// Draws the graph with the toolchain in `bin`; the rest of graph, for
/// tests.
pub async fn graph_with(
    bin: &Path,
    req: &GraphRequest,
    call: &Call,
    phases: &Phases,
) -> Result<GraphResult, ToolError> {
    let out = &req.build_folder;
    let io = |what: &str, e: std::io::Error| {
        ToolError::engine(format!("Can't {what} in the build folder: {e}."))
    };
    std::fs::create_dir_all(out).map_err(|e| io("create the folder", e))?;
    // Remove the last graph first, so a failure never leaves an old one
    // looking current.
    let ext = req.format.extension();
    for name in [format!("{STEM}.dot"), format!("{STEM}.{ext}")] {
        match std::fs::remove_file(out.join(&name)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(io(&format!("delete the old {name}"), e));
            }
            _ => {}
        }
    }

    phases.start("Reading the design");
    let run = run_logged("graph", "yosys", bin, &yosys_args(req), out, call).await?;
    if !run.status.success() {
        return Err(yosys_error(&run, req));
    }
    let src = req.source_folder.to_string_lossy();
    let diagnostics = parse_yosys(&run.output, &src).warnings;
    // The labels name each process's file and line: yosys was given
    // absolute paths, so name them as the request says (source files
    // relative to the source folder unless display_paths says otherwise).
    let dot = out.join(format!("{STEM}.dot"));
    if let Ok(text) = std::fs::read_to_string(&dot) {
        let named = call.display.text(&text);
        if named != text {
            std::fs::write(&dot, named).map_err(|e| io("write the graph", e))?;
        }
    }

    phases.start("Drawing the graph");
    let run = run_logged("graph", "dot", bin, &dot_args(req.format), out, call).await?;
    let graph_file = format!("{STEM}.{ext}");
    if !run.status.success() || !out.join(&graph_file).is_file() {
        return Err(tool_failed("dot", "drawing the graph", &run));
    }
    Ok(GraphResult {
        graph_file,
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> GraphRequest {
        GraphRequest {
            source_folder: "/p/my proj".into(),
            source_files: vec!["main.v".into(), "rtl/uart.sv".into()],
            build_folder: "/p/my proj/_build".into(),
            top_module: "main".into(),
            defines: BTreeMap::from([("WIDTH".into(), "8".into()), ("FAST".into(), "".into())]),
            format: GraphFormat::Svg,
            verbose: false,
        }
    }

    #[test]
    fn standard_commands() {
        assert_eq!(
            yosys_args(&req()),
            [
                "-p",
                "read_verilog -sv \"/p/my proj/main.v\" \"/p/my proj/rtl/uart.sv\"; \
                 show -format dot -colors 1 -wireshape plaintext -prefix graph main",
                "-DSYNTHESIZE",
                "-DFAST=",
                "-DWIDTH=8",
                "-q",
            ]
        );
        let verbose = GraphRequest {
            verbose: true,
            ..req()
        };
        assert!(!yosys_args(&verbose).contains(&"-q".into()));
        assert_eq!(
            dot_args(GraphFormat::Png),
            ["-Tpng", "graph.dot", "-o", "graph.png"]
        );
        assert_eq!(dot_args(GraphFormat::Svg)[3], "graph.svg");
        assert_eq!(dot_args(GraphFormat::Pdf)[0], "-Tpdf");
    }

    fn run(output: &str) -> crate::runner::ToolRun {
        use std::os::unix::process::ExitStatusExt;
        crate::runner::ToolRun {
            status: std::process::ExitStatus::from_raw(1 << 8),
            output: output.into(),
        }
    }

    #[test]
    fn yosys_failures() {
        // Real yosys output for a syntax error and for a missing top module.
        let e = yosys_error(
            &run("/p/my proj/rtl/uart.sv:2: ERROR: syntax error, unexpected ';'\n"),
            &req(),
        );
        assert_eq!(e.category, Category::Design);
        assert_eq!(
            e.message,
            "The design has errors. rtl/uart.sv:2: syntax error, unexpected ';'"
        );
        assert_eq!(e.diagnostics[0].file.as_deref(), Some("rtl/uart.sv"));
        assert_eq!(e.diagnostics[0].line, Some(2));

        let e = yosys_error(
            &run("Warning: Selection \"main\" did not match any module.\n\
                 ERROR: Nothing there to show.\n"),
            &req(),
        );
        assert_eq!(e.category, Category::Design);
        assert_eq!(
            e.message,
            "The design has errors. The top module main isn't in the design files."
        );
        assert_eq!(e.diagnostics.len(), 1);

        let e = yosys_error(&run("Segmentation fault\n"), &req());
        assert_eq!(e.category, Category::Tool);
    }
}
