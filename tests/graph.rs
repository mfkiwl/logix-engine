//! graph with the real toolchain, on the designs in tests/designs. It uses
//! the same engine folder as tests/build.rs, target/tmp/test-engine, so the
//! package is downloaded once and reused.

#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use logix_engine::errors::Category;
use logix_engine::packages::PackageManager;
use logix_engine::packages::manifest;
use logix_engine::progress::{NoProgress, Phases};
use logix_engine::state::{self, EngineState};
use logix_engine::tools::{Call, graph, test};
use serde_json::{Value, json};
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;

/// An engine state with the toolchain downloaded.
async fn engine() -> EngineState {
    static READY: OnceCell<()> = OnceCell::const_new();
    let folder = Path::new(env!("CARGO_TARGET_TMPDIR")).join("test-engine");
    std::fs::create_dir_all(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    let packages = PackageManager::new(
        &folder,
        Arc::new(manifest::builtin().clone()),
        &state::platform(),
        manifest::GITHUB,
    );
    READY
        .get_or_init(|| async {
            let missing = packages.needing_download(&["oss-cad-suite"]).unwrap();
            let phases = Phases::new(Arc::new(NoProgress), missing.len());
            packages
                .ensure(&missing, true, &phases, &CancellationToken::new())
                .await
                .expect("download oss-cad-suite for the graph tests");
        })
        .await;
    EngineState {
        engine_folder: folder.display().to_string(),
        engine_folder_resolved: folder.clone(),
        auto_download: true,
        command_line: vec![],
        log_file: folder.join("logs/test.log"),
        packages,
    }
}

fn call() -> Call {
    Call::new(Arc::new(NoProgress), CancellationToken::new())
}

/// A copy of a design from tests/designs in a folder named `name`, so the
/// originals are never changed.
fn design(design: &str, name: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().canonicalize().unwrap().join(name);
    std::fs::create_dir_all(&src).unwrap();
    let from = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/designs")
        .join(design);
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), src.join(entry.file_name())).unwrap();
    }
    (tmp, src)
}

fn input(src: &Path, extra: Value) -> graph::GraphInput {
    let mut v = json!({
        "top_module": "main",
        "source_folder": src,
        "source_files": ["main.v"],
        "build_folder": src.join("_build"),
    });
    for (k, val) in extra.as_object().unwrap() {
        v[k] = val.clone();
    }
    serde_json::from_value(v).expect("a valid graph input")
}

#[tokio::test]
async fn graph_draws_the_design_as_svg() {
    let state = engine().await;
    // A space in the path, which yosys's script must quote.
    let (_tmp, src) = design("blinky", "my proj");
    let c = call();
    let out = graph::run(&state, input(&src, json!({})), &c)
        .await
        .unwrap();
    assert_eq!(out.graph_file, "graph.svg");
    let svg = std::fs::read_to_string(src.join("_build/graph.svg")).unwrap();
    assert!(svg.contains("<svg"), "{svg}");
    // The module's ports are in the drawing.
    assert!(svg.contains("CLK") && svg.contains("LED0"), "{svg}");
    assert!(src.join("_build/graph.dot").is_file());

    // The commands are ready to run again.
    let tools: Vec<_> = out.commands.iter().map(|c| c.tool.as_str()).collect();
    assert_eq!(tools, ["yosys", "dot"]);
    assert!(out.commands[0].args[2].starts_with("read_verilog -sv \""));
    assert!(
        out.commands[0].args[2]
            .ends_with("; show -format dot -colors 1 -wireshape plaintext -prefix graph main")
    );
    assert_eq!(
        &out.commands[1].args[1..],
        ["-Tsvg", "graph.dot", "-o", "graph.svg"]
    );
    std::fs::remove_file(src.join("_build/graph.svg")).unwrap();
    let rerun = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(format!(
            "{} && {}",
            out.commands[0].shell, out.commands[1].shell
        ))
        .status()
        .unwrap();
    assert!(rerun.success());
    assert!(src.join("_build/graph.svg").is_file());
}

/// The diagram's text: the SVG through the engine's tool dispatcher, which
/// applies the request's display paths.
async fn labels(state: &EngineState, args: Value, src: &Path) -> String {
    let result = logix_engine::tools::call(state, "graph", args.as_object().cloned(), &call())
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    std::fs::read_to_string(src.join("_build/graph.svg")).unwrap()
}

#[tokio::test]
async fn graph_labels_name_files_as_the_request_says() {
    let state = engine().await;
    let (_tmp, src) = design("blinky", "my proj");
    let mut args = json!({
        "top_module": "main",
        "source_folder": src,
        "source_files": ["main.v"],
        "build_folder": src.join("_build"),
    });
    let full = src.join("main.v").display().to_string();
    // As the request gave it: relative to the source folder.
    let svg = labels(&state, args.clone(), &src).await;
    assert!(svg.contains("main.v:"), "{svg}");
    assert!(!svg.contains(&full), "{svg}");
    // As display_paths names it.
    args["display_paths"] = json!({"folders": [{"path": src, "as": "PROJECT"}]});
    let svg = labels(&state, args, &src).await;
    assert!(svg.contains("PROJECT/main.v:"), "{svg}");
}

#[tokio::test]
async fn graph_writes_png_and_pdf_and_draws_any_module() {
    let state = engine().await;
    let (_tmp, src) = design("counter", "proj");
    for (format, magic) in [("png", &b"\x89PNG"[..]), ("pdf", &b"%PDF"[..])] {
        let out = graph::run(
            &state,
            input(
                &src,
                json!({"top_module": "counter", "source_files": ["counter.v"], "format": format}),
            ),
            &call(),
        )
        .await
        .unwrap();
        assert_eq!(out.graph_file, format!("graph.{format}"));
        let bytes = std::fs::read(src.join("_build").join(&out.graph_file)).unwrap();
        assert!(bytes.starts_with(magic), "{format}");
    }
}

#[tokio::test]
async fn graph_draws_designs_with_fpga_primitives_and_defines() {
    let state = engine().await;
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().canonicalize().unwrap();
    std::fs::write(
        src.join("main.v"),
        "module main(input CLK, output LED0);\n\
         `ifdef SYNTHESIZE\n  SB_LUT4 #(.LUT_INIT(16'h8000)) lut (.O(LED0), .I0(CLK), .I1(1'b1), \
         .I2(1'b1), .I3(1'b1));\n`endif\n\
         `ifdef MARK\n  wire marked_wire = CLK;\n`endif\nendmodule\n",
    )
    .unwrap();
    let out = graph::run(
        &state,
        input(&src, json!({"defines": {"MARK": ""}})),
        &call(),
    )
    .await
    .unwrap();
    let svg = std::fs::read_to_string(src.join("_build").join(out.graph_file)).unwrap();
    assert!(svg.contains("SB_LUT4"), "{svg}");
    assert!(svg.contains("marked_wire"), "{svg}");
}

#[tokio::test]
async fn graph_design_errors_and_no_stale_graph() {
    let state = engine().await;
    let (_tmp, src) = design("syntax-error", "proj");
    // An old graph is removed, so a failure never leaves it looking current.
    std::fs::create_dir_all(src.join("_build")).unwrap();
    std::fs::write(src.join("_build/graph.svg"), "old").unwrap();
    let c = call();
    let err = graph::run(&state, input(&src, json!({})), &c)
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Design, "{err:?}");
    assert_eq!(err.diagnostics[0].file.as_deref(), Some("main.v"));
    assert_eq!(err.diagnostics[0].line, Some(3));
    assert!(!src.join("_build/graph.svg").exists());

    let (_tmp, src) = design("blinky", "proj");
    let err = graph::run(&state, input(&src, json!({"top_module": "nope"})), &call())
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Design);
    assert_eq!(
        err.message,
        "The design has errors. The top module nope isn't in the design files."
    );
}

/// The counter design's testbenches pass and fail as their names say.
#[tokio::test]
async fn the_counter_test_design_has_a_passing_and_a_failing_testbench() {
    let state = engine().await;
    let (_tmp, src) = design("counter", "proj");
    let v = json!({
        "board": "alhambra-ii",
        "source_folder": src,
        "source_files": ["counter.v"],
        "testbench_files": ["counter_tb.v", "counter_fail_tb.v"],
        "build_folder": src.join("_build"),
    });
    let out = test::run(&state, serde_json::from_value(v).unwrap(), &call())
        .await
        .unwrap();
    let passed: Vec<_> = out.testbenches.iter().map(|t| t.passed).collect();
    assert_eq!(passed, [true, false]);
    assert!(
        out.testbenches[1].diagnostics[0]
            .text
            .contains("q is 9, expected 8"),
        "{:?}",
        out.testbenches[1].diagnostics
    );
}
