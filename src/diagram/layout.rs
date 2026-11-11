//! Lays out schematics with ELK (the Eclipse Layout Kernel), running
//! logix-core's elkjs in QuickJS, a small JavaScript engine built into the
//! engine. No other program or runtime is needed.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rquickjs::{Context, Runtime};

/// The JavaScript stack, and the thread's native stack, which must be larger
/// (ELK's layered layout recurses deeply on large graphs).
const JS_STACK: usize = 256 << 20;
const THREAD_STACK: usize = 320 << 20;

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
    /// elkjs or QuickJS failed.
    Failed(String),
}

/// ELK, loaded and ready.
pub struct Elk {
    rt: Runtime,
    ctx: Context,
    /// When the current layout must end.
    deadline: Arc<std::sync::Mutex<Option<Instant>>>,
    cancelled: Arc<AtomicBool>,
    timed_out: Arc<AtomicBool>,
}

impl Elk {
    /// Loads elkjs's bundle (`lib/elk.bundled.js`). `cancelled` stops a
    /// layout that's running.
    pub fn load(bundle: &Path, cancelled: Arc<AtomicBool>) -> Result<Elk, Stop> {
        let code = std::fs::read_to_string(bundle)
            .map_err(|e| Stop::Failed(format!("can't read {}: {e}", bundle.display())))?;
        let rt = Runtime::new().map_err(|e| Stop::Failed(e.to_string()))?;
        rt.set_max_stack_size(JS_STACK);
        let deadline: Arc<std::sync::Mutex<Option<Instant>>> = Default::default();
        let timed_out = Arc::new(AtomicBool::new(false));
        {
            let (deadline, cancelled, timed_out) =
                (deadline.clone(), cancelled.clone(), timed_out.clone());
            rt.set_interrupt_handler(Some(Box::new(move || {
                if cancelled.load(Ordering::Relaxed) {
                    return true;
                }
                let late = deadline
                    .lock()
                    .ok()
                    .and_then(|d| *d)
                    .is_some_and(|d| Instant::now() > d);
                if late {
                    timed_out.store(true, Ordering::Relaxed);
                }
                late
            })));
        }
        let ctx = Context::full(&rt).map_err(|e| Stop::Failed(e.to_string()))?;
        ctx.with(|ctx| -> Result<(), Stop> {
            let run = |src: &str| -> Result<(), Stop> {
                ctx.eval::<(), _>(src).map_err(|e| {
                    let thrown = ctx.catch();
                    Stop::Failed(format!("{e}: {thrown:?}"))
                })
            };
            run(SHIMS)?;
            run(&code)?;
            run("globalThis.__elk = new ELK();")
        })?;
        Ok(Elk {
            rt,
            ctx,
            deadline,
            cancelled,
            timed_out,
        })
    }

    /// Lays out an ELK JSON graph, within `limit`, and returns the laid-out
    /// graph.
    pub fn layout(&self, graph: &str, limit: Duration) -> Result<String, Stop> {
        *self.deadline.lock().unwrap() = Some(Instant::now() + limit);
        self.timed_out.store(false, Ordering::Relaxed);
        let stop = |e: String| {
            if self.cancelled.load(Ordering::Relaxed) {
                Stop::Cancelled
            } else if self.timed_out.load(Ordering::Relaxed) {
                Stop::TooLong
            } else {
                Stop::Failed(e)
            }
        };
        self.ctx.with(|ctx| -> Result<(), Stop> {
            ctx.globals()
                .set("__in", graph)
                .map_err(|e| stop(e.to_string()))?;
            ctx.eval::<(), _>(
                "globalThis.__out = null; globalThis.__err = null;\
                 __elk.layout(JSON.parse(__in)).then(\
                   (r) => { globalThis.__out = JSON.stringify(r); },\
                   (e) => { globalThis.__err = String(e && e.message || e); });",
            )
            .map_err(|e| stop(format!("{e}: {:?}", ctx.catch())))
        })?;
        while self.rt.is_job_pending() {
            if let Err(e) = self.rt.execute_pending_job() {
                let text = format!("{e:?}");
                return Err(stop(text));
            }
        }
        *self.deadline.lock().unwrap() = None;
        self.ctx.with(|ctx| {
            let out: Option<String> = ctx
                .eval("globalThis.__out")
                .map_err(|e| stop(e.to_string()))?;
            let err: Option<String> = ctx
                .eval("globalThis.__err")
                .map_err(|e| stop(e.to_string()))?;
            match (out, err) {
                (Some(o), _) => Ok(o),
                (None, Some(e)) => Err(stop(e)),
                (None, None) => Err(stop("the layout didn't finish".into())),
            }
        })
    }
}

/// Runs `work` with a loaded ELK on a thread of its own, with the large
/// native stack QuickJS needs.
pub async fn with_elk<T: Send + 'static>(
    bundle: std::path::PathBuf,
    cancel: tokio_util::sync::CancellationToken,
    work: impl FnOnce(Result<Elk, Stop>) -> T + Send + 'static,
) -> Result<T, String> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let watcher = {
        let (cancelled, cancel) = (cancelled.clone(), cancel.clone());
        tokio::spawn(async move {
            cancel.cancelled().await;
            cancelled.store(true, Ordering::Relaxed);
        })
    };
    let (tx, rx) = tokio::sync::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("diagram-layout".into())
        .stack_size(THREAD_STACK)
        .spawn(move || {
            let elk = Elk::load(&bundle, cancelled);
            let _ = tx.send(work(elk));
        });
    let result = match spawned {
        Ok(_) => rx
            .await
            .map_err(|_| "the layout thread stopped".to_string()),
        Err(e) => Err(format!("can't start the layout thread: {e}")),
    };
    watcher.abort();
    result
}
