//! Build and clean_build tests with the real toolchain. They share one engine
//! folder, target/tmp/test-engine, so oss-cad-suite (about 526 MB) and
//! openxc7 (about 124 MB) are downloaded once and reused by later runs. CI
//! caches them.

#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use logix_engine::errors::Category;
use logix_engine::packages::PackageManager;
use logix_engine::packages::manifest;
use logix_engine::progress::{NoProgress, Phases, ProgressSink};
use logix_engine::state::{self, EngineState};
use logix_engine::tools::{Call, build, clean_build};
use serde_json::{Value, json};
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;

fn engine_folder() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("test-engine")
}

/// An engine state with the toolchain installed.
async fn engine() -> EngineState {
    static READY: OnceCell<()> = OnceCell::const_new();
    let folder = engine_folder();
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
            let missing = packages
                .needing_install(&["oss-cad-suite", "openxc7"])
                .unwrap();
            let phases = Phases::new(Arc::new(NoProgress), missing.len());
            packages
                .ensure(&missing, true, &phases, &CancellationToken::new())
                .await
                .expect("install oss-cad-suite and openxc7 for the build tests");
        })
        .await;
    EngineState {
        engine_folder: folder.display().to_string(),
        engine_folder_resolved: folder.clone(),
        auto_install: true,
        command_line: vec![],
        log_file: folder.join("logs/test.log"),
        packages,
    }
}

fn call() -> Call {
    Call::new(Arc::new(NoProgress), CancellationToken::new())
}

const BLINK: &str = r#"`include "defs.vh"
module main(input CLK, output LED0, output LED1);
  reg [23:0] cnt = 0;
  always @(posedge CLK) cnt <= cnt + 1;
  assign LED0 = cnt[23];
`ifdef SYNTHESIZE
  assign LED1 = `LED1_VALUE;
`endif
endmodule
"#;

const PCF: &str = "set_io LED0 45\nset_io LED1 44\nset_io CLK 49\n";

/// A project folder with a blink design for the Alhambra II.
fn project() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().canonicalize().unwrap().join("proj");
    std::fs::create_dir_all(src.join("inc")).unwrap();
    std::fs::write(src.join("main.v"), BLINK).unwrap();
    std::fs::write(src.join("defs.vh"), "`define LED1_VALUE 1'b1\n").unwrap();
    std::fs::write(src.join("pinout.pcf"), PCF).unwrap();
    (tmp, src)
}

fn request(src: &Path, extra: Value) -> build::BuildInput {
    let mut v = json!({
        "board": "alhambra-ii",
        "top_module": "main",
        "source_folder": src,
        "source_files": ["main.v"],
        "constraint_file": "pinout.pcf",
        "build_folder": src.join("_build"),
    });
    for (k, val) in extra.as_object().unwrap() {
        v[k] = val.clone();
    }
    serde_json::from_value(v).expect("a valid build input")
}

async fn build_it(
    state: &EngineState,
    src: &Path,
    extra: Value,
) -> Result<build::BuildOutput, logix_engine::errors::ToolError> {
    build::run(state, request(src, extra), &call()).await
}

#[tokio::test]
async fn builds_then_is_up_to_date_until_an_input_changes() {
    let state = engine().await;
    let (_tmp, src) = project();
    let out_dir = src.join("_build");

    let out = build_it(&state, &src, json!({})).await.unwrap();
    assert!(!out.up_to_date);
    let tools: Vec<_> = out.commands.iter().map(|c| c.tool.as_str()).collect();
    assert_eq!(tools, ["yosys", "nextpnr-ice40", "icepack"]);
    assert!(
        out.commands[0]
            .package
            .as_deref()
            .unwrap()
            .starts_with("oss-cad-suite ")
    );
    assert_eq!(out.commands[0].cwd, src.to_string_lossy());
    assert_eq!(out.bitstream, "hardware.bin");
    let bin = std::fs::read(out_dir.join("hardware.bin")).unwrap();
    assert!(bin.len() > 100_000, "an hx8k bitstream is about 135 KB");
    let lc = &out.resources_usage["ICESTORM_LC"];
    assert!(lc.used > 0 && lc.available == 7680, "{lc:?}");
    assert_eq!(out.resources_usage["SB_IO"].used, 3);
    assert_eq!(out.timing.len(), 1);
    assert!(
        out.timing.values().all(|mhz| *mhz > 50.0),
        "{:?}",
        out.timing
    );
    for f in [
        ".logix-build",
        "build-record.json",
        "synth.log",
        "pnr.log",
        "hardware.json",
    ] {
        assert!(out_dir.join(f).exists(), "{f}");
    }

    // Same inputs: up to date, same result.
    let again = build_it(&state, &src, json!({})).await.unwrap();
    assert!(again.up_to_date);
    assert!(
        again.commands.is_empty(),
        "an up-to-date build runs nothing"
    );
    assert_eq!(again.resources_usage, out.resources_usage);

    // An included file changes: rebuilds.
    std::fs::write(src.join("defs.vh"), "`define LED1_VALUE 1'b0\n").unwrap();
    assert!(!build_it(&state, &src, json!({})).await.unwrap().up_to_date);
    // A define changes: rebuilds.
    assert!(
        !build_it(&state, &src, json!({"defines": {"X": "1"}}))
            .await
            .unwrap()
            .up_to_date
    );
    assert!(
        build_it(&state, &src, json!({"defines": {"X": "1"}}))
            .await
            .unwrap()
            .up_to_date
    );
}

/// An ECP5 build with the standard commands, for the iCESugar-Pro.
#[tokio::test]
async fn builds_an_ecp5_design() {
    let state = engine().await;
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().canonicalize().unwrap();
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/designs/blinky-ecp5");
    for f in ["main.v", "main.lpf"] {
        std::fs::copy(from.join(f), src.join(f)).unwrap();
    }
    let out = build_it(
        &state,
        &src,
        json!({"board": "icesugar-pro", "constraint_file": "main.lpf"}),
    )
    .await
    .unwrap();
    let tools: Vec<_> = out.commands.iter().map(|c| c.tool.as_str()).collect();
    assert_eq!(tools, ["yosys", "nextpnr-ecp5", "ecppack"]);
    assert!(
        out.commands[0]
            .args
            .iter()
            .any(|a| a.starts_with("synth_ecp5 -top main"))
    );
    let pnr = &out.commands[1].args[1..];
    assert_eq!(
        &pnr[..6],
        ["--25k", "--package", "CABGA256", "--speed", "6", "--json"]
    );
    assert!(pnr.contains(&"--timing-allow-fail".to_string()));
    assert_eq!(out.bitstream, "hardware.bit");
    let bit = std::fs::read(src.join("_build/hardware.bit")).unwrap();
    assert!(bit.len() > 10_000, "{}", bit.len());
    assert!(
        out.resources_usage["TRELLIS_COMB"].used > 0,
        "{:?}",
        out.resources_usage
    );
    assert_eq!(out.timing.len(), 1, "{:?}", out.timing);

    // Another variant always rebuilds, even one that only changes how the
    // board is programmed; the build record keeps it for program_board.
    let ram = json!({"board": "icesugar-pro", "constraint_file": "main.lpf",
                     "board_variant": "usb-ram"});
    assert!(
        !build_it(&state, &src, ram.clone())
            .await
            .unwrap()
            .up_to_date
    );
    let record: Value =
        serde_json::from_slice(&std::fs::read(src.join("_build/build-record.json")).unwrap())
            .unwrap();
    assert_eq!(record["board"]["variant"]["id"], json!("usb-ram"));
    assert_eq!(record["board"]["name"], "Icesugar Pro (RAM)");
    assert!(build_it(&state, &src, ram).await.unwrap().up_to_date);
    let err = build_it(
        &state,
        &src,
        json!({"board": "icesugar-pro", "constraint_file": "main.lpf",
               "board_variant": "usb-eeprom"}),
    )
    .await
    .unwrap_err();
    assert_eq!(err.category, logix_engine::errors::Category::Request);
    assert!(
        err.message.contains("usb-flash, usb-ram"),
        "{}",
        err.message
    );

    // A pin that doesn't exist is a design error.
    std::fs::write(
        src.join("main.lpf"),
        "LOCATE COMP \"CLK\" SITE \"P6\";\nLOCATE COMP \"LED0\" SITE \"ZZ99\";\n",
    )
    .unwrap();
    let err = build_it(
        &state,
        &src,
        json!({"board": "icesugar-pro", "constraint_file": "main.lpf"}),
    )
    .await
    .unwrap_err();
    assert_eq!(
        err.category,
        logix_engine::errors::Category::Design,
        "{err:?}"
    );
    assert!(!src.join("_build/hardware.bit").exists());
}

/// A Gowin build with the standard commands, for the Tang Nano 9K.
#[tokio::test]
async fn builds_a_gowin_design() {
    let state = engine().await;
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().canonicalize().unwrap();
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/designs/blinky-gowin");
    for f in ["main.v", "main.cst"] {
        std::fs::copy(from.join(f), src.join(f)).unwrap();
    }
    let input = json!({"board": "sipeed-tang-nano-9k", "constraint_file": "main.cst"});
    let out = build_it(&state, &src, input.clone()).await.unwrap();
    let tools: Vec<_> = out.commands.iter().map(|c| c.tool.as_str()).collect();
    assert_eq!(tools, ["yosys", "nextpnr-himbaechel", "gowin_pack"]);
    assert!(
        out.commands[0]
            .args
            .iter()
            .any(|a| a.starts_with("synth_gowin -top main; write_json "))
    );
    let pnr = &out.commands[1].args[1..];
    assert_eq!(
        &pnr[..10],
        [
            "--device",
            "GW1NR-LV9QN88PC6/I5",
            "--json",
            "hardware.json",
            "--write",
            "hardware.pnr.json",
            "--report",
            "hardware.pnr",
            "--vopt",
            "family=GW1N-9C"
        ]
    );
    assert_eq!(pnr[10], "--vopt");
    assert!(pnr[11].starts_with("cst=") && pnr[11].ends_with("main.cst"));
    assert_eq!(
        out.commands[2].args[1..],
        ["-d", "GW1N-9C", "-o", "hardware.fs", "hardware.pnr.json"]
    );
    assert_eq!(out.bitstream, "hardware.fs");
    let fs = std::fs::read(src.join("_build/hardware.fs")).unwrap();
    assert!(fs.len() > 10_000, "{}", fs.len());
    assert!(
        out.resources_usage["LUT4"].used > 0 && out.resources_usage["DFF"].used > 0,
        "{:?}",
        out.resources_usage
    );
    assert_eq!(out.timing.len(), 1, "{:?}", out.timing);

    // A pin that doesn't exist is a design error.
    std::fs::write(
        src.join("main.cst"),
        "IO_LOC \"CLK\" 52;\nIO_LOC \"LED0\" 999;\n",
    )
    .unwrap();
    let err = build_it(&state, &src, input).await.unwrap_err();
    assert_eq!(
        err.category,
        logix_engine::errors::Category::Design,
        "{err:?}"
    );
    assert!(!src.join("_build/hardware.fs").exists());
}

/// A Xilinx build with the standard commands, for the Arty A7-35T: yosys from
/// oss-cad-suite, the rest from openxc7.
#[tokio::test]
async fn builds_a_xilinx_design() {
    let state = engine().await;
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().canonicalize().unwrap();
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/designs/blinky-xilinx");
    for f in ["main.v", "main.xdc"] {
        std::fs::copy(from.join(f), src.join(f)).unwrap();
    }
    let input = json!({"board": "arty-a7-35t", "constraint_file": "main.xdc"});
    let out = build_it(&state, &src, input.clone()).await.unwrap();
    let tools: Vec<_> = out.commands.iter().map(|c| c.tool.as_str()).collect();
    assert_eq!(
        tools,
        ["yosys", "nextpnr-xilinx", "fasm2frames", "xc7frames2bit"]
    );
    let packages: Vec<_> = out
        .commands
        .iter()
        .map(|c| c.package.clone().unwrap_or_default())
        .collect();
    assert!(packages[0].starts_with("oss-cad-suite "), "{packages:?}");
    assert!(
        packages[1..].iter().all(|p| p.starts_with("openxc7 ")),
        "{packages:?}"
    );
    assert!(
        out.commands[0]
            .args
            .iter()
            .any(|a| a.starts_with("synth_xilinx -arch xc7 -top main; write_json "))
    );
    let pnr = &out.commands[1].args[1..];
    assert_eq!(&pnr[..3], ["--device", "xc7a35tcsg324-1", "-o"]);
    assert!(pnr[3].starts_with("xdc=") && pnr[3].ends_with("main.xdc"));
    assert_eq!(
        &pnr[4..10],
        [
            "-o",
            "fasm=hardware.fasm",
            "--json",
            "hardware.json",
            "--report",
            "hardware.pnr"
        ]
    );
    let frames = &out.commands[2].args[1..];
    assert!(frames[3].ends_with("share/nextpnr/external/prjxray-db/artix7"));
    assert_eq!(&frames[4..], ["hardware.fasm", "hardware.frames"]);
    assert!(out.commands[3].args[2].ends_with("prjxray-db/artix7/xc7a35tcsg324-1/part.yaml"));
    assert_eq!(out.bitstream, "hardware.bit");
    let bit = std::fs::read(src.join("_build/hardware.bit")).unwrap();
    assert!(bit.len() > 100_000, "{}", bit.len());
    assert!(!out.resources_usage.is_empty(), "{:?}", out.resources_usage);
    assert_eq!(out.timing.len(), 1, "{:?}", out.timing);

    // A pin that doesn't exist is a design error.
    std::fs::write(
        src.join("main.xdc"),
        "set_property PACKAGE_PIN E3 [get_ports CLK]\nset_property PACKAGE_PIN ZZ99 [get_ports LED0]\n",
    )
    .unwrap();
    let err = build_it(&state, &src, input).await.unwrap_err();
    assert_eq!(
        err.category,
        logix_engine::errors::Category::Design,
        "{err:?}"
    );
    assert!(!src.join("_build/hardware.bit").exists());
}

#[tokio::test]
async fn a_syntax_error_is_a_design_error_and_removes_the_old_bitstream() {
    let state = engine().await;
    let (_tmp, src) = project();
    let out_dir = src.join("_build");
    build_it(&state, &src, json!({})).await.unwrap();
    std::fs::write(out_dir.join("my-notes.txt"), "keep").unwrap();

    std::fs::write(
        src.join("main.v"),
        "module main(input CLK, output LED0, output LED1);\n  reg x\n  assign LED0 = 1;\nendmodule\n",
    )
    .unwrap();
    let err = build_it(&state, &src, json!({})).await.unwrap_err();
    assert_eq!(err.category, Category::Design, "{err:?}");
    assert_eq!(err.diagnostics[0].file.as_deref(), Some("main.v"));
    assert_eq!(err.diagnostics[0].line, Some(3));
    assert!(err.message.contains("main.v:3"), "{}", err.message);
    assert!(!out_dir.join("hardware.bin").exists(), "no stale bitstream");
    assert!(
        !out_dir.join("build-record.json").exists(),
        "no stale record"
    );
    assert!(
        out_dir.join("my-notes.txt").exists(),
        "other files are kept"
    );
}

#[tokio::test]
async fn design_errors_from_each_tool() {
    let state = engine().await;

    // Unknown top module: no location.
    let (_t1, src) = project();
    let err = build_it(&state, &src, json!({"top_module": "nosuch"}))
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Design);
    assert!(err.message.contains("nosuch"), "{}", err.message);

    // A pin the package doesn't have: in the constraint file.
    let (_t2, src) = project();
    std::fs::write(
        src.join("pinout.pcf"),
        "set_io LED0 45\nset_io LED1 999\nset_io CLK 49\n",
    )
    .unwrap();
    let err = build_it(&state, &src, json!({})).await.unwrap_err();
    assert_eq!(err.category, Category::Design);
    assert_eq!(err.diagnostics[0].file.as_deref(), Some("pinout.pcf"));
    assert_eq!(err.diagnostics[0].line, Some(2));
    assert_eq!(err.diagnostics[0].tool.as_deref(), Some("nextpnr"));

    // An unconstrained port.
    let (_t3, src) = project();
    std::fs::write(src.join("pinout.pcf"), "set_io LED0 45\nset_io CLK 49\n").unwrap();
    let err = build_it(&state, &src, json!({})).await.unwrap_err();
    assert_eq!(err.category, Category::Design);
    assert!(err.message.contains("LED1"), "{}", err.message);
}

#[tokio::test]
async fn warnings_are_reported() {
    let state = engine().await;
    let (_tmp, src) = project();
    std::fs::write(
        src.join("main.v"),
        "module main(input CLK, output LED0, output LED1);\n  assign LED0 = undeclared;\n  assign LED1 = 0;\nendmodule\n",
    )
    .unwrap();
    let out = build_it(&state, &src, json!({})).await.unwrap();
    let w = out
        .warnings
        .iter()
        .find(|w| w.text.contains("undeclared"))
        .expect("the implicit declaration warning");
    assert_eq!(w.file.as_deref(), Some("main.v"));
    assert_eq!(w.line, Some(2));
}

#[tokio::test]
async fn request_errors() {
    let state = engine().await;
    let (tmp, src) = project();
    let cases = [
        json!({"build_folder": src.join("out")}),
        json!({"build_folder": src}),
        json!({"source_files": ["../proj/main.v"]}),
        json!({"source_files": ["missing.v"]}),
        json!({"source_files": []}),
        json!({"constraint_file": "/abs.pcf"}),
        json!({"board": "no-such-board"}),
        json!({"defines": {"1BAD": "x"}}),
        json!({"top_module": "two words"}),
        json!({"source_folder": "relative/path"}),
    ];
    for extra in cases {
        let err = build_it(&state, &src, extra.clone()).await.unwrap_err();
        assert_eq!(err.category, Category::Request, "{extra}: {err:?}");
    }
    // A PCF file named .lpf, for an iCE40 board: nextpnr would read it as PCF
    // anyway, but the name is the wrong family's format.
    std::fs::copy(src.join("pinout.pcf"), src.join("pinout.lpf")).unwrap();
    let err = build_it(&state, &src, json!({"constraint_file": "pinout.lpf"}))
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Request, "{err:?}");
    assert!(err.message.contains("isn't a .pcf file"), "{}", err.message);
    std::fs::remove_file(src.join("pinout.lpf")).unwrap();
    // An include from outside the source folder.
    std::fs::write(tmp.path().join("outside.vh"), "").unwrap();
    std::fs::write(
        src.join("main.v"),
        format!("`include \"../outside.vh\"\n{BLINK}"),
    )
    .unwrap();
    let err = build_it(&state, &src, json!({})).await.unwrap_err();
    assert_eq!(err.category, Category::Request);
    assert!(
        !src.join("_build").exists(),
        "nothing written on a request error"
    );
}

#[tokio::test]
async fn a_full_board_definition_works() {
    let state = engine().await;
    let (_tmp, src) = project();
    let board = json!({
        "description": "My Alhambra II copy",
        "fpga": {
            "architecture": "ice40", "part_number": "ICE40HX4K-TQ144", "size": "8k",
            "nextpnr_device": "hx8k", "nextpnr_package": "tq144:4k"
        },
        "variants": [{
            "id": "usb-flash", "connection": "usb", "memory": "flash",
            "programmer": "iceprog", "args": "-d d:${BUS}/${DEV} ${BITSTREAM}",
            "usb": {"vid": "0403", "pid": "6010"}, "tested": false
        }]
    });
    let out = build_it(&state, &src, json!({"board": board}))
        .await
        .unwrap();
    assert!(src.join("_build").join(out.bitstream).exists());
}

#[tokio::test]
async fn progress_and_cancel() {
    #[derive(Default)]
    struct Messages(std::sync::Mutex<Vec<String>>);
    impl ProgressSink for Messages {
        fn report(&self, _p: f64, _t: f64, m: Option<String>) {
            if let Some(m) = m {
                self.0.lock().unwrap().push(m);
            }
        }
    }
    let state = engine().await;
    let (_tmp, src) = project();
    let rec = Arc::new(Messages::default());
    let c = Call::new(rec.clone(), CancellationToken::new());
    build::run(&state, request(&src, json!({})), &c)
        .await
        .unwrap();
    assert_eq!(
        *rec.0.lock().unwrap(),
        vec!["Synthesizing", "Placing and routing", "Packing bitstream"]
    );

    let (_tmp2, src) = project();
    let c = Call::new(Arc::new(NoProgress), CancellationToken::new());
    c.cancel.cancel();
    let err = build::run(&state, request(&src, json!({})), &c)
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Cancelled);
}

#[tokio::test]
async fn clean_build_removes_a_folder_of_build_folders() {
    let state = engine().await;
    let (_tmp, src) = project();
    let root = src.join("_build");
    build_it(
        &state,
        &src,
        json!({"build_folder": root.join("alhambra-ii")}),
    )
    .await
    .unwrap();
    build_it(&state, &src, json!({"build_folder": root.join("icezum")}))
        .await
        .unwrap();

    let out = clean_build::run(
        &state,
        serde_json::from_value(json!({"folder": root})).unwrap(),
    )
    .unwrap();
    assert_eq!(
        out.deleted,
        vec![
            root.join("alhambra-ii").display().to_string(),
            root.join("icezum").display().to_string(),
            root.display().to_string(),
        ]
    );
    assert!(!root.exists());
    assert!(src.join("main.v").exists());

    // The project folder itself fails the path rules.
    let err = clean_build::run(
        &state,
        serde_json::from_value(json!({"folder": src})).unwrap(),
    )
    .unwrap_err();
    assert_eq!(err.category, Category::Request);
}

// ----- list_devices and program_board, with fake device lists. Programming
// real boards is checked by hand on hardware.

use logix_engine::devices::{CONFIDENCE_HIGH, Device};
use logix_engine::errors::ActionCode;
use logix_engine::tools::{list_devices, program_board};

fn usb(product: &str, serial: Option<&str>, dev: u8) -> Device {
    Device::usb(
        "0403".into(),
        "6010".into(),
        None,
        Some(product.into()),
        serial.map(Into::into),
        250,
        dev,
        format!("250-{dev}"),
        true,
    )
}

fn program_input(build_folder: &Path, extra: Value) -> program_board::ProgramBoardInput {
    let mut v = json!({ "build_folder": build_folder });
    for (k, val) in extra.as_object().unwrap() {
        v[k] = val.clone();
    }
    serde_json::from_value(v).unwrap()
}

#[test]
fn list_devices_names_the_boards_each_device_may_be() {
    let devices = vec![
        usb("Alhambra II", Some("A1"), 3),
        usb("Dual RS232-HS", None, 4),
    ];
    let out = list_devices::run_with(Default::default(), || Ok(devices.clone())).unwrap();
    let best = &out.devices[0].boards[0];
    assert_eq!(
        (best.board.as_str(), best.variants[0].confidence),
        ("alhambra-ii", CONFIDENCE_HIGH)
    );
    assert!(
        out.devices[1]
            .boards
            .iter()
            .all(|m| m.board != "alhambra-ii"),
        "another product name"
    );
    assert_eq!(out.devices[0].id, devices[0].id);
}

#[tokio::test]
async fn list_devices_for_a_build_lists_only_the_devices_that_fit() {
    let state = engine().await;
    let (_tmp, src) = project();
    let folder = src.join("_build");
    let input = |folder: &Path| -> list_devices::ListDevicesInput {
        serde_json::from_value(json!({"build_folder": folder})).unwrap()
    };
    // No build yet: a request error.
    let err = list_devices::run_with(input(&folder), || Ok(vec![])).unwrap_err();
    assert_eq!(err.category, Category::Request);

    build_it(&state, &src, json!({})).await.unwrap();
    let mut locked = usb("", None, 7);
    locked.product = None;
    locked.reachable = false;
    let devices = vec![
        usb("Dual RS232-HS", None, 4),
        usb("Alhambra II v1.0A", Some("A1"), 3),
        locked,
    ];
    let out = list_devices::run_with(input(&folder), || Ok(devices)).unwrap();
    let fit: Vec<_> = out
        .devices
        .iter()
        .map(|d| (d.product.as_deref().unwrap(), d.build_confidence))
        .collect();
    assert_eq!(fit, [("Alhambra II v1.0A", Some(CONFIDENCE_HIGH))]);
    assert!(!out.devices[0].boards.is_empty(), "still all its boards");
    assert_eq!(out.unreachable.len(), 1);
    assert_eq!(out.unreachable[0].build_confidence, Some(1));
}

#[tokio::test]
async fn program_board_needs_a_successful_build() {
    let state = engine().await;
    let (_tmp, src) = project();
    let folder = src.join("_build");
    let input = program_input(&folder, json!({"device": "usb:250-3:0403:6010:"}));
    let err = program_board::run_with(&state, input, &call(), || Ok(vec![]))
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Request);
    assert!(err.message.contains("Run build first"), "{}", err.message);
}

#[tokio::test]
async fn program_board_checks_the_chosen_device() {
    let state = engine().await;
    let (_tmp, src) = project();
    let folder = src.join("_build");
    build_it(&state, &src, json!({})).await.unwrap();
    let alhambra = || usb("Alhambra II v2.0", Some("A1"), 3);
    let second = || usb("Alhambra II", Some("B2"), 5);
    let other = || usb("Dual RS232-HS", None, 4);

    let run = |devices: Vec<Device>, extra: Value| {
        let state = state.clone();
        let input = program_input(&folder, extra);
        async move { program_board::run_with(&state, input, &call(), move || Ok(devices)).await }
    };

    // Not connected anymore.
    let err = run(vec![other()], json!({"device": alhambra().id}))
        .await
        .unwrap_err();
    assert_eq!(err.action.unwrap().code, ActionCode::ConnectBoard);

    // Not an ID, and a device that isn't the board.
    let err = run(vec![alhambra()], json!({"device": "alhambra"}))
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Request);
    let err = run(vec![alhambra(), other()], json!({"device": other().id}))
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Request);
    assert!(err.message.contains("Dual RS232-HS"), "{}", err.message);

    // A command that doesn't name the device runs only when no other device
    // has its USB IDs.
    let err = run(
        vec![alhambra(), other()],
        json!({"device": alhambra().id, "programmer_command": "openFPGALoader ${BITSTREAM}"}),
    )
    .await
    .unwrap_err();
    assert_eq!(err.action.unwrap().code, ActionCode::DisconnectDevices);

    // The real openFPGALoader, pointed at a device that isn't there: a
    // setup error with connect_board and the programmer's output.
    let err = run(vec![alhambra(), second()], json!({"device": second().id}))
        .await
        .unwrap_err();
    assert_eq!(err.category, Category::Setup, "{err:?}");
    assert_eq!(err.action.as_ref().unwrap().code, ActionCode::ConnectBoard);
    assert!(err.details.contains("device not found"), "{err:?}");
}

#[tokio::test]
async fn program_board_runs_the_command_with_the_device_and_bitstream() {
    #[derive(Default)]
    struct Messages(std::sync::Mutex<Vec<String>>);
    impl ProgressSink for Messages {
        fn report(&self, _p: f64, _t: f64, m: Option<String>) {
            if let Some(m) = m {
                self.0.lock().unwrap().push(m);
            }
        }
    }
    let state = engine().await;
    let (tmp, src) = project();
    let folder = src.join("_build");
    build_it(&state, &src, json!({})).await.unwrap();

    // A fake programmer that records its arguments.
    let log = tmp.path().join("args.txt");
    let script = tmp.path().join("fake-prog.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho \"$@\" > {}\necho 'Writing: 50.00%'\n",
            log.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let cmd = format!(
        "{} --busdev ${{BUS}}:${{DEV}} ${{BITSTREAM}}",
        script.display()
    );
    let rec = Arc::new(Messages::default());
    let c = Call::new(rec.clone(), CancellationToken::new());
    let out = program_board::run_with(
        &state,
        program_input(
            &folder,
            json!({"programmer_command": cmd, "device": usb("Alhambra II", None, 9).id}),
        ),
        &c,
        || Ok(vec![usb("Alhambra II", None, 9)]),
    )
    .await
    .unwrap();
    assert_eq!(out.programmer, "custom");
    assert_eq!(out.device.device, 9);
    assert_eq!(out.commands.len(), 1);
    assert_eq!(out.commands[0].args[0], script.to_string_lossy());
    assert_eq!(out.commands[0].package, None, "a custom programmer");
    let args = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        args.trim(),
        format!("--busdev 250:9 {}", folder.join("hardware.bin").display())
    );
    assert_eq!(
        *rec.0.lock().unwrap(),
        vec!["Finding device", "Programming"]
    );

    // A failing programmer is an error with its output.
    std::fs::write(&script, "#!/bin/sh\necho 'segfault here' >&2\nexit 3\n").unwrap();
    let err = program_board::run_with(
        &state,
        program_input(
            &folder,
            json!({"programmer_command": cmd, "device": usb("Alhambra II", None, 9).id}),
        ),
        &call(),
        || Ok(vec![usb("Alhambra II", None, 9)]),
    )
    .await
    .unwrap_err();
    assert_eq!(err.category, Category::Tool);
    assert!(err.details.contains("segfault here"));
}

#[tokio::test]
async fn verbose_prints_more_but_gives_the_same_result() {
    use logix_engine::progress::{OutputLevel, OutputSink};
    #[derive(Default)]
    struct Lines(std::sync::Mutex<Vec<String>>);
    impl OutputSink for Lines {
        fn line(&self, _tool: &str, _level: OutputLevel, text: &str) {
            self.0.lock().unwrap().push(text.to_string());
        }
    }
    let state = engine().await;
    let (_tmp, src) = project();
    std::fs::write(
        src.join("main.v"),
        "module main(input CLK, output LED0, output LED1);\n  assign LED0 = undeclared;\n  assign LED1 = 0;\nendmodule\n",
    )
    .unwrap();
    let mut results = Vec::new();
    let mut counts = Vec::new();
    for (folder, verbose) in [("_build/quiet", false), ("_build/verbose", true)] {
        let lines = Arc::new(Lines::default());
        let mut c = Call::new(Arc::new(NoProgress), CancellationToken::new());
        c.output = lines.clone();
        let input = request(
            &src,
            json!({"build_folder": src.join(folder), "verbose": verbose}),
        );
        results.push(build::run(&state, input, &c).await.unwrap());
        counts.push(lines.0.lock().unwrap().len());
    }
    assert!(
        counts[1] > counts[0] * 5 + 50,
        "verbose printed more: {counts:?}"
    );
    assert_eq!(
        results[0].warnings, results[1].warnings,
        "same warnings, no duplicates"
    );
    assert_eq!(results[0].resources_usage, results[1].resources_usage);
}
