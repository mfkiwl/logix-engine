//! Lays out schematics with ELK (the Eclipse Layout Kernel), running
//! logix-core's elkjs in QuickJS, a small JavaScript engine built into the
//! engine. No other program or runtime is needed.
//!
//! The layout runs in a worker: the engine's own binary started with
//! `--diagram-layout-worker`, one graph per line on its input, one result
//! per line on its output. A layout that crashes or runs too long stops only
//! the worker (it's killed, never interrupted, since interrupting QuickJS in
//! the middle of elkjs damaged its memory, 2026-11-11), and the engine goes
//! on with the next module in a new worker.

use std::io::{BufRead, Write as _};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use rquickjs::{Context, Runtime};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;

/// The JavaScript stack, and the thread's native stack, which must be larger
/// (ELK's layered layout recurses deeply on large graphs).
const JS_STACK: usize = 256 << 20;
const THREAD_STACK: usize = 320 << 20;

/// The command-line option that starts a worker (hidden; see
/// docs/command-line.md).
pub const WORKER_OPTION: &str = "--diagram-layout-worker";

/// What elkjs expects of a browser or Node.js: its bundle runs the layout in
/// a "worker" it drives with setTimeout, and may log.
const SHIMS: &str = "globalThis.self = globalThis;\
    globalThis.setTimeout = (f) => { Promise.resolve().then(() => f()); return 0; };\
    globalThis.clearTimeout = () => {};\
    globalThis.console = { log() {}, warn() {}, error() {}, info() {}, debug() {} };";

/// Why a layout didn't finish.
#[derive(Debug, PartialEq)]
pub enum Stop {
    /// The call was cancelled.
    Cancelled,
    /// It took longer than its time limit.
    TooLong,
    /// elkjs, QuickJS or the worker failed.
    Failed(String),
}

/// ELK, loaded in this process.
struct Elk {
    rt: Runtime,
    ctx: Context,
}

impl Elk {
    fn load(bundle: &Path) -> Result<Elk, String> {
        let code = std::fs::read_to_string(bundle)
            .map_err(|e| format!("can't read {}: {e}", bundle.display()))?;
        let rt = Runtime::new().map_err(|e| e.to_string())?;
        rt.set_max_stack_size(JS_STACK);
        let ctx = Context::full(&rt).map_err(|e| e.to_string())?;
        ctx.with(|ctx| -> Result<(), String> {
            for src in [SHIMS, code.as_str(), "globalThis.__elk = new ELK();"] {
                ctx.eval::<(), _>(src)
                    .map_err(|e| format!("{e}: {:?}", ctx.catch()))?;
            }
            Ok(())
        })?;
        Ok(Elk { rt, ctx })
    }

    /// Lays out an ELK JSON graph and returns the laid-out graph.
    fn layout(&self, graph: &str) -> Result<String, String> {
        self.ctx.with(|ctx| -> Result<(), String> {
            ctx.globals()
                .set("__in", graph)
                .map_err(|e| e.to_string())?;
            ctx.eval::<(), _>(
                "globalThis.__out = null; globalThis.__err = null;\
                 __elk.layout(JSON.parse(__in)).then(\
                   (r) => { globalThis.__out = JSON.stringify(r); },\
                   (e) => { globalThis.__err = String(e && e.message || e); });",
            )
            .map_err(|e| format!("{e}: {:?}", ctx.catch()))
        })?;
        while self.rt.is_job_pending() {
            self.rt
                .execute_pending_job()
                .map_err(|e| format!("{e:?}"))?;
        }
        self.ctx.with(|ctx| {
            let out: Option<String> = ctx.eval("globalThis.__out").map_err(|e| e.to_string())?;
            let err: Option<String> = ctx.eval("globalThis.__err").map_err(|e| e.to_string())?;
            match (out, err) {
                (Some(o), _) => Ok(o),
                (None, Some(e)) => Err(e),
                (None, None) => Err("the layout didn't finish".into()),
            }
        })
    }
}

/// The worker's main: loads ELK, then lays out each graph on its input (one
/// JSON graph per line) and writes each result as a line: the laid-out
/// graph, or {"error": "..."}.
pub fn worker_main(bundle: &Path) -> std::process::ExitCode {
    let bundle = bundle.to_path_buf();
    let run = std::thread::Builder::new()
        .name("diagram-layout".into())
        .stack_size(THREAD_STACK)
        .spawn(move || -> Result<(), String> {
            let elk = Elk::load(&bundle)?;
            let stdin = std::io::stdin();
            let mut out = std::io::stdout().lock();
            for line in stdin.lock().lines() {
                let line = line.map_err(|e| e.to_string())?;
                let result = match elk.layout(&line) {
                    Ok(r) => r,
                    Err(e) => serde_json::json!({ "error": e }).to_string(),
                };
                writeln!(out, "{result}").map_err(|e| e.to_string())?;
                out.flush().map_err(|e| e.to_string())?;
            }
            Ok(())
        });
    match run.map(|t| t.join()) {
        Ok(Ok(Ok(()))) => std::process::ExitCode::SUCCESS,
        Ok(Ok(Err(e))) => {
            eprintln!("diagram layout worker: {e}");
            std::process::ExitCode::FAILURE
        }
        _ => std::process::ExitCode::FAILURE,
    }
}

/// The program workers run: the engine itself, or the one tests set.
static PROGRAM: OnceLock<PathBuf> = OnceLock::new();

/// For tests, whose own binary isn't the engine: the engine binary to start
/// workers with.
#[doc(hidden)]
pub fn set_worker_program(program: PathBuf) {
    let _ = PROGRAM.set(program);
}

fn worker_program() -> Result<PathBuf, String> {
    match PROGRAM.get() {
        Some(p) => Ok(p.clone()),
        None => std::env::current_exe().map_err(|e| format!("can't find the engine's binary: {e}")),
    }
}

/// A running worker.
struct Worker {
    child: tokio::process::Child,
    input: tokio::process::ChildStdin,
    output: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
}

impl Worker {
    fn start(bundle: &Path) -> Result<Worker, String> {
        let mut child = tokio::process::Command::new(worker_program()?)
            .arg(WORKER_OPTION)
            .arg(bundle)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("can't start the layout worker: {e}"))?;
        let input = child.stdin.take().ok_or("no input to the worker")?;
        let output =
            BufReader::new(child.stdout.take().ok_or("no output from the worker")?).lines();
        Ok(Worker {
            child,
            input,
            output,
        })
    }
}

/// Lays out graphs in workers, one after the other.
pub struct Layouts {
    bundle: PathBuf,
    worker: Option<Worker>,
    cancel: CancellationToken,
}

impl Layouts {
    pub fn new(bundle: &Path, cancel: CancellationToken) -> Layouts {
        Layouts {
            bundle: bundle.to_path_buf(),
            worker: None,
            cancel,
        }
    }

    /// Lays out an ELK JSON graph within `limit`. A worker that fails or
    /// takes too long is stopped; the next layout starts a new one.
    pub async fn layout(&mut self, graph: &str, limit: Duration) -> Result<String, Stop> {
        if self.cancel.is_cancelled() {
            return Err(Stop::Cancelled);
        }
        if self.worker.is_none() {
            self.worker = Some(Worker::start(&self.bundle).map_err(Stop::Failed)?);
        }
        let worker = self.worker.as_mut().unwrap();
        let exchange = async {
            worker
                .input
                .write_all(format!("{graph}\n").as_bytes())
                .await
                .map_err(|e| format!("can't send the graph to the layout worker: {e}"))?;
            worker.input.flush().await.map_err(|e| e.to_string())?;
            match worker.output.next_line().await {
                Ok(Some(line)) => Ok(line),
                Ok(None) => Err("the layout worker stopped".to_string()),
                Err(e) => Err(format!("can't read the layout worker's result: {e}")),
            }
        };
        let result = tokio::select! {
            r = tokio::time::timeout(limit, exchange) => match r {
                Ok(Ok(line)) => Ok(line),
                Ok(Err(e)) => Err(Stop::Failed(e)),
                Err(_) => Err(Stop::TooLong),
            },
            _ = self.cancel.cancelled() => Err(Stop::Cancelled),
        };
        match result {
            Ok(line) => match serde_json::from_str::<serde_json::Value>(&line) {
                Ok(v) if v.get("children").is_some() => Ok(line),
                Ok(v) if v.get("error").is_some() => {
                    Err(Stop::Failed(v["error"].as_str().unwrap_or("").to_string()))
                }
                _ => {
                    self.stop_worker().await;
                    Err(Stop::Failed(
                        "the layout worker's result isn't a layout".into(),
                    ))
                }
            },
            Err(e) => {
                self.stop_worker().await;
                Err(e)
            }
        }
    }

    async fn stop_worker(&mut self) {
        if let Some(mut w) = self.worker.take() {
            let _ = w.child.start_kill();
            let _ = w.child.wait().await;
        }
    }
}
