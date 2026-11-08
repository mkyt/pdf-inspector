//! Running a macro on QuickJS: the `__host` functions over a [`Session`],
//! the prelude, the macro's module, and stopping it.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use pdfi_api::{
    ApiError, DocId, ErrorCode, GroupId, JobState, Request, Response, ResponseBody, Session,
};
use rquickjs::context::EvalOptions;
use rquickjs::function::Opt;
use rquickjs::promise::PromiseState;
use rquickjs::{
    Array, CaughtError, Context, Ctx, Function, Module, Object, Promise, Runtime, TypedArray, Value,
};

use crate::meta::Meta;
use crate::{MacroError, PRELUDE};

/// The level of a log line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

/// A file a macro wrote with `ctx.emit`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Emitted {
    pub name: String,
    pub mime: Option<String>,
    pub data: Vec<u8>,
}

/// Receives the macro's log lines.
pub type LogFn = Box<dyn FnMut(Level, &str)>;
/// Receives progress: done, total and a label.
pub type ProgressFn = Box<dyn FnMut(u64, u64, Option<&str>)>;

/// What a run gave back.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    /// The default export's return value, as JSON (`null` for none).
    pub value: serde_json::Value,
    /// For a value that is a list of objects: their keys in the order the
    /// macro wrote them (for a table).
    pub columns: Option<Vec<String>>,
    pub emitted: Vec<Emitted>,
}

/// Where a run goes and how it reports.
pub struct RunOptions {
    /// Set to stop the macro (checked between host calls and while
    /// JavaScript runs).
    pub cancel: Arc<AtomicBool>,
    pub timeout: Option<Duration>,
    /// `ctx.host` (`"cli"`, `"server"`, `"macos"`).
    pub host: String,
    /// `ctx.input`: what the macro may know about its input (no paths).
    pub input: serde_json::Value,
    pub log: LogFn,
    pub progress: ProgressFn,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            cancel: Arc::default(),
            timeout: None,
            host: "cli".into(),
            input: serde_json::json!({}),
            log: Box::new(|_, _| {}),
            progress: Box::new(|_, _, _| {}),
        }
    }
}

struct HostState {
    session: Option<Arc<Session>>,
    group: Option<GroupId>,
    cancel: Arc<AtomicBool>,
    deadline: Option<Instant>,
    log: RefCell<LogFn>,
    progress: RefCell<ProgressFn>,
    emitted: RefCell<Vec<Emitted>>,
}

impl HostState {
    fn stopped(&self) -> bool {
        self.cancel.load(Ordering::Relaxed) || self.deadline.is_some_and(|d| Instant::now() >= d)
    }

    fn dispatch(&self, req: Request) -> pdfi_api::Reply {
        let Some(s) = &self.session else {
            return pdfi_api::Reply {
                response: Response::Err(ApiError::new(
                    ErrorCode::Unsupported,
                    "there is no document while the macro's meta is read",
                )),
                blobs: Vec::new(),
            };
        };
        let cancel = || self.stopped();
        match self.group {
            Some(g) => s.dispatch_in_group(req, g, &cancel),
            None => s.dispatch_cancellable(req, &cancel),
        }
    }

    fn wait_job(&self, job: u32) -> String {
        let status = || self.dispatch(Request::JobStatus { job });
        loop {
            let r = status();
            let running = matches!(
                &r.response,
                Response::Ok(ResponseBody::Job(j)) if j.state == JobState::Running
            );
            if !running || self.stopped() {
                if running {
                    self.dispatch(Request::CancelJob { job });
                }
                return match r.response {
                    Response::Ok(ResponseBody::Job(j)) => serde_json::to_string(&j).unwrap(),
                    Response::Ok(_) => "{}".into(),
                    Response::Err(e) => serde_json::json!({
                        "state": "failed",
                        "error": e.message,
                    })
                    .to_string(),
                };
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

fn install<'js>(ctx: &Ctx<'js>, state: &Rc<HostState>) -> rquickjs::Result<()> {
    let h = Object::new(ctx.clone())?;
    let s = state.clone();
    h.set(
        "dispatch",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, json: String| -> rquickjs::Result<Object<'js>> {
                let reply = match serde_json::from_str::<Request>(&json) {
                    Ok(req) => s.dispatch(req),
                    Err(e) => pdfi_api::Reply {
                        response: Response::Err(ApiError::new(
                            ErrorCode::BadRequest,
                            format!("malformed request: {e}"),
                        )),
                        blobs: Vec::new(),
                    },
                };
                let o = Object::new(ctx.clone())?;
                o.set("body", serde_json::to_string(&reply.response).unwrap())?;
                let parts = Array::new(ctx.clone())?;
                for (i, b) in reply.blobs.into_iter().enumerate() {
                    parts.set(i, TypedArray::<u8>::new(ctx.clone(), b)?)?;
                }
                o.set("parts", parts)?;
                Ok(o)
            },
        )?,
    )?;
    let s = state.clone();
    h.set(
        "waitJob",
        Function::new(ctx.clone(), move |job: u32| s.wait_job(job))?,
    )?;
    let s = state.clone();
    h.set(
        "log",
        Function::new(ctx.clone(), move |level: String, text: String| {
            let level = match level.as_str() {
                "warn" => Level::Warn,
                "error" => Level::Error,
                _ => Level::Info,
            };
            (s.log.borrow_mut())(level, &text);
        })?,
    )?;
    let s = state.clone();
    h.set(
        "progress",
        Function::new(
            ctx.clone(),
            move |done: f64, total: f64, label: Opt<Option<String>>| {
                let label = label.0.flatten();
                (s.progress.borrow_mut())(
                    done.max(0.0) as u64,
                    total.max(0.0) as u64,
                    label.as_deref(),
                );
            },
        )?,
    )?;
    let s = state.clone();
    h.set(
        "emit",
        Function::new(
            ctx.clone(),
            move |name: String, binary: String, mime: Opt<Option<String>>| {
                // One character per byte (the prelude's `latin1`).
                let data = binary.chars().map(|c| c as u32 as u8).collect();
                s.emitted.borrow_mut().push(Emitted {
                    name,
                    mime: mime.0.flatten(),
                    data,
                });
            },
        )?,
    )?;
    let s = state.clone();
    h.set(
        "cancelled",
        Function::new(ctx.clone(), move || s.stopped())?,
    )?;
    ctx.globals().set("__host", h)?;
    let mut o = EvalOptions::default();
    o.global = true;
    o.strict = true;
    o.filename = Some("prelude.js".into());
    ctx.eval_with_options::<(), _>(PRELUDE, o)
}

/// The message of what was thrown (the pending exception after `e`).
fn caught(ctx: &Ctx<'_>, e: rquickjs::Error) -> String {
    match CaughtError::from_error(ctx, e) {
        CaughtError::Exception(x) => {
            let msg = x.message().unwrap_or_default();
            match x.stack() {
                Some(s) if !s.trim().is_empty() => format!("{msg}\n{}", s.trim_end()),
                _ => msg,
            }
        }
        CaughtError::Value(v) => format!("{v:?}"),
        CaughtError::Error(e) => e.to_string(),
    }
}

/// A runtime with the host functions and the prelude, running `f` in it.
fn with_engine<R>(
    state: Rc<HostState>,
    f: impl for<'js> FnOnce(&Ctx<'js>) -> Result<R, MacroError>,
) -> Result<R, MacroError> {
    let host = |e: rquickjs::Error| MacroError::Host(e.to_string());
    let rt = Runtime::new().map_err(host)?;
    rt.set_max_stack_size(1 << 20);
    let s = state.clone();
    rt.set_interrupt_handler(Some(Box::new(move || s.stopped())));
    let cx = Context::full(&rt).map_err(host)?;
    cx.with(|ctx| {
        install(&ctx, &state)
            .map_err(|e| MacroError::Host(format!("prelude: {}", caught(&ctx, e))))?;
        f(&ctx)
    })
}

/// Run the job queue until `p` settles.
fn settle<'js>(
    ctx: &Ctx<'js>,
    p: &Promise<'js>,
    state: &HostState,
) -> Result<Value<'js>, MacroError> {
    loop {
        match p.state() {
            PromiseState::Resolved => {
                return p
                    .result::<Value>()
                    .unwrap()
                    .map_err(|e| script(caught(ctx, e)));
            }
            PromiseState::Rejected => {
                let e = p.result::<Value>().unwrap().unwrap_err();
                return Err(stopped(state).unwrap_or_else(|| script(caught(ctx, e))));
            }
            PromiseState::Pending => {}
        }
        // A job that throws (an interrupt: nothing else escapes the
        // prelude) leaves its promise pending; the queue then runs dry.
        if !ctx.execute_pending_job() {
            return Err(stopped(state).unwrap_or_else(|| MacroError::Script {
                code: "Error".into(),
                message: "the macro waits for something that never comes".into(),
                stack: None,
            }));
        }
    }
}

fn stopped(state: &HostState) -> Option<MacroError> {
    if state.cancel.load(Ordering::Relaxed) {
        Some(MacroError::Cancelled)
    } else if state.deadline.is_some_and(|d| Instant::now() >= d) {
        Some(MacroError::Timeout)
    } else {
        None
    }
}

fn script(message: String) -> MacroError {
    MacroError::Script {
        code: "Error".into(),
        message,
        stack: None,
    }
}

/// Evaluate a macro's module; returns its namespace.
fn module<'js>(
    ctx: &Ctx<'js>,
    file: &str,
    code: &str,
    state: &HostState,
) -> Result<Object<'js>, MacroError> {
    let m = Module::declare(ctx.clone(), file, code).map_err(|e| script(caught(ctx, e)))?;
    let (m, p) = m.eval().map_err(|e| script(caught(ctx, e)))?;
    settle(ctx, &p, state)?;
    m.namespace().map_err(|e| script(caught(ctx, e)))
}

fn state(session: Option<Arc<Session>>, group: Option<GroupId>, opts: RunOptions) -> Rc<HostState> {
    Rc::new(HostState {
        session,
        group,
        cancel: opts.cancel,
        deadline: opts.timeout.map(|t| Instant::now() + t),
        log: RefCell::new(opts.log),
        progress: RefCell::new(opts.progress),
        emitted: RefCell::default(),
    })
}

/// Evaluate a macro without a document and read its `meta`. `fallback`
/// names it when `meta.name` does not.
pub fn read_meta(file: &str, code: &str, fallback: &str) -> Result<Meta, MacroError> {
    let opts = RunOptions {
        timeout: Some(Duration::from_secs(10)),
        ..RunOptions::default()
    };
    let st = state(None, None, opts);
    with_engine(st.clone(), |ctx| {
        let ns = module(ctx, file, code, &st)?;
        let pdfi: Object = ctx
            .globals()
            .get("__pdfi")
            .map_err(|e| MacroError::Host(e.to_string()))?;
        let f: Function = pdfi
            .get("meta")
            .map_err(|e| MacroError::Host(e.to_string()))?;
        let json: String = f
            .call((ns, fallback))
            .map_err(|e| invalid(caught(ctx, e)))?;
        serde_json::from_str(&json).map_err(|e| MacroError::Host(format!("meta: {e}")))
    })
}

fn invalid(message: String) -> MacroError {
    MacroError::Invalid(message.lines().next().unwrap_or_default().to_string())
}

/// Check parameters (as `run` takes them) against `meta` without a
/// document: types, names, missing ones. Page ranges are checked against
/// the document when it runs.
pub fn check_params(
    meta: &Meta,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), MacroError> {
    let st = state(None, None, RunOptions::default());
    with_engine(st, |ctx| {
        let pdfi: Object = ctx
            .globals()
            .get("__pdfi")
            .map_err(|e| MacroError::Host(e.to_string()))?;
        let f: Function = pdfi
            .get("checkParams")
            .map_err(|e| MacroError::Host(e.to_string()))?;
        f.call::<_, ()>((
            serde_json::to_string(meta).unwrap(),
            serde_json::to_string(params).unwrap(),
        ))
        .map_err(|e| invalid(caught(ctx, e)))
    })
}

/// Run a macro on document `doc` of `session` as one undoable step: its
/// edits are kept if it succeeds and reverted otherwise.
///
/// `params` are the raw parameters: JSON values, or strings as a command
/// line gives them (the prelude converts and checks them); `bytes` as
/// `{"$bytes": base64}`, `pdf` as `{"$doc": id}` of a document open in the
/// same session.
pub fn run(
    session: &Arc<Session>,
    doc: DocId,
    file: &str,
    code: &str,
    meta: &Meta,
    params: &serde_json::Map<String, serde_json::Value>,
    opts: RunOptions,
) -> Result<Outcome, MacroError> {
    let group = session
        .begin_group(doc, meta.title.clone())
        .map_err(MacroError::Api)?;
    let input = serde_json::json!({ "host": opts.host, "input": opts.input }).to_string();
    let st = state(Some(session.clone()), Some(group), opts);
    let result = with_engine(st.clone(), |ctx| {
        let ns = module(ctx, file, code, &st)?;
        let pdfi: Object = ctx
            .globals()
            .get("__pdfi")
            .map_err(|e| MacroError::Host(e.to_string()))?;
        let f: Function = pdfi
            .get("run")
            .map_err(|e| MacroError::Host(e.to_string()))?;
        let p: Promise = f
            .call((
                ns,
                serde_json::to_string(meta).unwrap(),
                doc,
                serde_json::to_string(params).unwrap(),
                input,
            ))
            .map_err(|e| script(caught(ctx, e)))?;
        let v = settle(ctx, &p, &st)?;
        let json: String = v.get().map_err(|e| MacroError::Host(e.to_string()))?;
        Ok(json)
    })
    .and_then(|json| {
        #[derive(serde::Deserialize)]
        struct Res {
            ok: bool,
            #[serde(default)]
            value: serde_json::Value,
            #[serde(default)]
            columns: Option<Vec<String>>,
            error: Option<Thrown>,
        }
        #[derive(serde::Deserialize)]
        struct Thrown {
            code: String,
            message: String,
            stack: Option<String>,
        }
        let r: Res = serde_json::from_str(&json).map_err(|e| MacroError::Host(e.to_string()))?;
        match (r.ok, r.error) {
            (true, _) => Ok((r.value, r.columns)),
            (false, Some(t)) => Err(stopped(&st).unwrap_or(MacroError::Script {
                code: t.code,
                message: t.message,
                stack: t.stack.filter(|s| !s.trim().is_empty()),
            })),
            (false, None) => Err(MacroError::Host("no error given".into())),
        }
    });
    let end = session.end_group(doc, group, result.is_ok());
    let (value, columns) = result?;
    end.map_err(MacroError::Api)?;
    let emitted = st.emitted.take();
    Ok(Outcome {
        value,
        columns,
        emitted,
    })
}
