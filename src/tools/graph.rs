//! The graph tool (docs/tool-api.md, "graph tool").

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::build::{
    check_build_folder, check_defines, check_source_file, check_source_folder, check_top_module,
};
use super::{Call, EngineTool};
use crate::errors::{Diagnostic, ToolError};
use crate::graph::{self, GraphFormat, GraphRequest};
use crate::state::EngineState;

pub struct Graph;

impl EngineTool for Graph {
    const NAME: &'static str = "graph";
    const DESCRIPTION: &'static str = "Draws a diagram of a design module, its cells, ports \
        and the wires between them: yosys reads the design \
        without synthesizing it and Graphviz draws the module. Writes graph.svg (or .pdf or \
        .png) and Graphviz's graph.dot in the build folder, replacing the last ones; \
        graph_file gives its path relative to the build folder. List the design files; the \
        engine doesn't search for them. Errors in the design, such as a syntax error or a \
        top module that isn't in the files, are design errors with diagnostics at file and \
        line. The first call may download the toolchain package first, which takes a few \
        minutes.";
    const READ_ONLY: bool = false;
    const OPEN_WORLD: bool = true;
    type Input = GraphInput;
    type Output = GraphOutput;
}

/// What to draw.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GraphInput {
    /// The module to draw, such as main, the design's top module, or any
    /// module in the design files, such as uart.
    pub top_module: String,
    /// The folder with the design files, as an absolute path.
    pub source_folder: String,
    /// The design files (.v or .sv), relative to the source folder, with /
    /// as the separator, such as ["main.v", "rtl/uart.v"]. Don't list
    /// included files or testbenches.
    #[schemars(length(min = 1))]
    pub source_files: Vec<String>,
    /// The build folder, as an absolute path, where the graph is written.
    /// It may or may not be inside the source folder, and is created if
    /// needed.
    pub build_folder: String,
    /// Verilog macro definitions, from name to value, such as
    /// {"WIDTH": "8"}. An empty value defines the macro with no text, for
    /// `ifdef. SYNTHESIZE is always defined, as in build.
    #[serde(default)]
    pub defines: BTreeMap<String, String>,
    /// The graph's file format: svg, pdf or png. Defaults to svg.
    #[serde(default)]
    pub format: GraphFormat,
    /// Let yosys print more detail, shown in the live output and the logs.
    /// It doesn't change the result. Defaults to false.
    #[serde(default)]
    pub verbose: bool,
    /// How to name paths in what people read (diagnostics, messages,
    /// testbench output, live tool output, diagram labels): files named as
    /// a whole, then folders whose files keep their path below the folder
    /// under the folder's new name ("" gives names relative to it); the
    /// first match in each list wins. Paths are compared by real location.
    /// Never applied to command lines or fields that are paths. Omitted
    /// means paths read as the request gave them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_paths: Option<crate::display_paths::DisplayPaths>,
}

/// What graph made.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GraphOutput {
    /// The graph, relative to the build folder, such as graph.svg.
    pub graph_file: String,
    /// Yosys warnings while reading the design, at file and line where
    /// known. Often empty.
    pub diagnostics: Vec<Diagnostic>,
    /// The programs this call ran, in order, each with the exact command and
    /// a shell line that runs it again without the engine, such as for a bug
    /// report to the tool's developers.
    pub commands: Vec<crate::commands::CommandRecord>,
}

/// Checks the request and turns it into a GraphRequest.
pub fn check(input: GraphInput) -> Result<GraphRequest, ToolError> {
    let req = |m: String| ToolError::request(m);
    check_top_module(&input.top_module)?;
    check_defines(&input.defines)?;
    let src = check_source_folder(&input.source_folder)?;
    // Yosys reads the files from a script, where a double quote would end
    // the file name.
    if src.to_string_lossy().contains('"') {
        return Err(req(
            "source_folder: the path has a double quote (\"), which yosys can't read.".into(),
        ));
    }
    if input.source_files.is_empty() {
        return Err(req("source_files is empty.".into()));
    }
    let mut seen = BTreeSet::new();
    for f in &input.source_files {
        check_source_file(&src, "source_files", f)?;
        if f.contains('"') {
            return Err(req(format!(
                "source_files: {f:?} has a double quote (\"), which yosys can't read."
            )));
        }
        if !seen.insert(f) {
            return Err(req(format!(
                "source_files: {f:?} is listed more than once."
            )));
        }
    }
    let build_folder = check_build_folder(&input.build_folder, &src)?;
    Ok(GraphRequest {
        source_folder: src,
        source_files: input.source_files,
        build_folder,
        top_module: input.top_module,
        defines: input.defines,
        format: input.format,
        verbose: input.verbose,
    })
}

pub async fn run(
    state: &EngineState,
    input: GraphInput,
    call: &Call,
) -> Result<GraphOutput, ToolError> {
    let r = graph::graph(state, check(input)?, call).await?;
    Ok(GraphOutput {
        graph_file: r.graph_file,
        diagnostics: r.diagnostics,
        commands: call.commands.take(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::Category;
    use serde_json::json;
    use std::path::Path;

    fn input(src: &Path, extra: serde_json::Value) -> GraphInput {
        let mut v = json!({
            "top_module": "main",
            "source_folder": src,
            "source_files": ["main.v"],
            "build_folder": src.join("_build"),
        });
        for (k, val) in extra.as_object().unwrap() {
            v[k] = val.clone();
        }
        serde_json::from_value(v).unwrap()
    }

    fn project() -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().canonicalize().unwrap();
        std::fs::create_dir_all(src.join("rtl")).unwrap();
        for f in ["main.v", "rtl/uart.sv", "q\"uote.v"] {
            std::fs::write(src.join(f), "").unwrap();
        }
        (tmp, src)
    }

    #[test]
    fn a_good_request() {
        let (_tmp, src) = project();
        let req = check(input(
            &src,
            json!({
                "source_files": ["main.v", "rtl/uart.sv"],
                "defines": {"W": "8"},
                "format": "png",
            }),
        ))
        .unwrap();
        assert_eq!(req.source_folder, src);
        assert_eq!(req.build_folder, src.join("_build"));
        assert_eq!(req.format, GraphFormat::Png);
        assert_eq!(req.source_files.len(), 2);

        let req = check(input(&src, json!({}))).unwrap();
        assert_eq!(req.format, GraphFormat::Svg);
    }

    #[test]
    fn request_errors() {
        let (_tmp, src) = project();
        for (extra, needle) in [
            (json!({"source_files": []}), "source_files is empty"),
            (json!({"source_files": ["nope.v"]}), "can't be found"),
            (json!({"source_files": ["../x.v"]}), "source_files"),
            (json!({"source_files": ["q\"uote.v"]}), "double quote"),
            (
                json!({"source_files": ["main.v", "main.v"]}),
                "listed more than once",
            ),
            (json!({"top_module": "a b"}), "top_module"),
            (json!({"defines": {"1x": ""}}), "define name"),
            (json!({"build_folder": "rel"}), "build_folder"),
            (json!({"source_folder": "rel"}), "source_folder"),
        ] {
            let err = check(input(&src, extra.clone())).unwrap_err();
            assert_eq!(err.category, Category::Request, "{extra}");
            assert!(err.message.contains(needle), "{extra}: {}", err.message);
        }
        // An unknown format is refused by the schema.
        let v = json!({"top_module": "main", "source_folder": src, "source_files": ["main.v"],
            "build_folder": src.join("_build"), "format": "jpg"});
        assert!(serde_json::from_value::<GraphInput>(v).is_err());
    }
}
