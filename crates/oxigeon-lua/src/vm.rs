//! One worker's Lua VM, and how a single job runs inside it.
//!
//! This VM shares nothing with the game VM. It has no efuns — not one — and
//! that is the whole safety story. The temptation to add "just a read-only
//! one" is strong and each candidate fails for a concrete reason:
//!
//! - `get_current_session` is a **thread-local**. On a worker it is
//!   permanently `None`, so session-scoped efuns would not error, they would
//!   quietly return nothing. Silent wrongness is the worst failure available.
//! - `set_object_state` and `get_persistent` are **Lua globals**. A second VM
//!   gets its own empty copies: writes vanish, reads lie, and it looks like it
//!   works right up until it matters.
//! - `session_handler` and the Diesel stores genuinely *are* `Send + Sync`, so
//!   `send()` would compile and run. That is the trap, not the reassurance —
//!   it would interleave a worker's output with the game thread's and let a
//!   job watch the world change underneath it.
//!
//! Arguments are the only channel in, and the return value the only channel
//! out. That is a feature: it forces a job to state what it depends on, which
//! makes it reproducible and testable as plain Lua with no driver at all.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use mlua::prelude::*;
use mlua::{HookTriggers, VmState};

use crate::marshal::{self, Limits, LuaData};
use crate::settings::ComputeSettings;

/// How a job ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ending {
    Ok,
    /// The job raised.
    Error,
    /// The module or function could not be loaded.
    LoadError,
    /// The deadline passed while the job was still running.
    Timeout,
    /// Cancelled before or during the run.
    Cancelled,
    /// The instruction budget ran out.
    Budget,
    /// The queue was full, or the bridge refused it for another reason.
    Refused,
}

impl Ending {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::LoadError => "load_error",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::Budget => "budget",
            Self::Refused => "refused",
        }
    }

    pub fn is_ok(self) -> bool {
        self == Self::Ok
    }
}

/// Markers the hook raises with, so `run` can tell why a job stopped. They are
/// prefixed with a sentinel no ordinary Lua error will contain.
const MARK_CANCELLED: &str = "\u{1}compute-cancelled";
const MARK_TIMEOUT: &str = "\u{1}compute-timeout";
const MARK_BUDGET: &str = "\u{1}compute-budget";

/// At most this many log lines per job, and this many bytes each. A job runs
/// on a thread nobody is rate-limiting, so an unbounded `compute_log` would be
/// a way to flood the journal from inside game code.
const MAX_LOG_LINES: usize = 32;
const MAX_LOG_BYTES: usize = 512;

/// Per-job state shared with the hook and the intrinsics.
///
/// `Lua::set_hook` takes an `Fn`, so this lives behind `Rc<RefCell<..>>` — the
/// same shape the debugger's `HookLocal` uses, and for the same reason.
///
/// Cancellation is the exception and is an `Arc<AtomicBool>`: it arrives on the
/// worker's stdin, read by a thread that is not the Lua thread and cannot touch
/// an `Rc` or a `RefCell`. See [`ComputeVm::cancel_flag`].
#[derive(Default)]
pub struct JobCtl {
    pub deadline: Option<Instant>,
    instr_used: u64,
    instr_limit: u64,
    instr_step: u64,
    logs: Vec<(String, String)>,
    log_dropped: usize,
}

impl JobCtl {
    fn begin(&mut self, deadline: Option<Instant>) {
        self.deadline = deadline;
        self.instr_used = 0;
        self.logs.clear();
        self.log_dropped = 0;
    }

    fn take_logs(&mut self) -> Vec<(String, String)> {
        if self.log_dropped > 0 {
            let dropped = self.log_dropped;
            self.logs.push((
                "warn".to_string(),
                format!("[{dropped} more log line(s) dropped]"),
            ));
        }
        std::mem::take(&mut self.logs)
    }
}

/// A worker's VM. Built on the worker thread and never moved off it — `Lua` is
/// `!Send`, which is also why this cannot simply be handed around.
pub struct ComputeVm {
    lua: Lua,
    ctl: Rc<RefCell<JobCtl>>,
    /// Set from off-thread to ask the running job to stop.
    cancel: Arc<AtomicBool>,
    limits: Limits,
}

/// Build a compute VM.
///
/// The order mirrors `ScriptEngine::start` deliberately: turn the compiler off
/// first if a budget is wanted, install the intrinsics, *then* close the
/// sandbox, then set the path. Anything registered after `apply_sandbox` would
/// not be subject to it.
pub fn build(
    cfg: &ComputeSettings,
    mudlib: &Path,
    game: &Path,
    salt: u64,
) -> LuaResult<ComputeVm> {
    let lua = Lua::new();

    // Same bargain as the game VM: a budget needs the interpreter, because
    // LuaJIT dispatches no hooks from inside a compiled trace. Unlike the game
    // VM, the default here is to keep the compiler — running expensive code
    // fast is the entire reason this facility exists.
    // PUC Lua has no compiler to disable and always dispatches hooks, so this
    // is a LuaJIT-only concern.
    #[cfg(feature = "luajit")]
    if cfg.instruction_limit > 0 {
        lua.load("jit.off()").set_name("=<oxigeon>/compute_jit_off").exec()?;
    }

    if cfg.memory_mb > 0 {
        if let Err(e) = lua.set_memory_limit(cfg.memory_mb * 1024 * 1024) {
            tracing::warn!("compute: memory limit not enforceable on this build: {e}");
        }
    }

    let ctl = Rc::new(RefCell::new(JobCtl {
        instr_limit: cfg.instruction_limit,
        instr_step: instruction_step(cfg.instruction_limit) as u64,
        ..Default::default()
    }));

    let cancel = Arc::new(AtomicBool::new(false));

    register_intrinsics(&lua, &ctl, &cancel)?;
    crate::sandbox::apply_sandbox(&lua)?;

    // Each worker gets its own sequence. LuaJIT seeds from a constant, so
    // without this every worker VM — and every rebuild of one after a reload —
    // would replay the same numbers, which is precisely wrong for the facility
    // meant to run simulations. `salt` is the worker index, so two workers
    // built in the same nanosecond still diverge.
    crate::sandbox::seed_prng(&lua, salt)?;

    set_package_path(&lua, mudlib, game)?;

    if cfg.instruction_limit > 0 {
        // A hook that failed to install is a budget that never fires, and a
        // runaway job would then burn a worker for the life of the process.
        install_hook(&lua, &ctl, &cancel)?;
    }

    Ok(ComputeVm {
        lua,
        ctl,
        cancel,
        limits: Limits { depth: cfg.max_arg_depth, nodes: cfg.max_arg_nodes },
    })
}

fn instruction_step(limit: u64) -> u32 {
    (limit / 10).clamp(1_000, 1_000_000) as u32
}

/// The three things a job can call that the game VM does not provide.
///
/// They are not efuns: none of them touches game state. `compute_log` matters
/// more than it looks — a debug adapter cannot attach to a compute VM, so
/// without it there is no way at all to see inside a job.
fn register_intrinsics(
    lua: &Lua,
    ctl: &Rc<RefCell<JobCtl>>,
    cancel: &Arc<AtomicBool>,
) -> LuaResult<()> {
    let globals = lua.globals();

    let c = ctl.clone();
    globals.set(
        "compute_log",
        lua.create_function(move |_, (level, message): (String, String)| {
            let mut ctl = c.borrow_mut();
            if ctl.logs.len() >= MAX_LOG_LINES {
                ctl.log_dropped += 1;
                return Ok(());
            }
            let mut message = message;
            message.truncate(MAX_LOG_BYTES);
            ctl.logs.push((level, message));
            Ok(())
        })?,
    )?;

    let c = ctl.clone();
    globals.set(
        "compute_deadline_ms",
        lua.create_function(move |_, ()| {
            let ctl = c.borrow();
            Ok(match ctl.deadline {
                Some(d) => d.saturating_duration_since(Instant::now()).as_millis() as f64,
                None => f64::INFINITY,
            })
        })?,
    )?;

    let c = cancel.clone();
    globals.set(
        "compute_cancelled",
        lua.create_function(move |_, ()| Ok(c.load(Ordering::Relaxed)))?,
    )?;

    Ok(())
}

/// Charge instructions, and stop the job if its budget, deadline or cancel
/// flag says so. Only installed when a budget is configured — with the
/// compiler on the hook would never fire anyway.
fn install_hook(lua: &Lua, ctl: &Rc<RefCell<JobCtl>>, cancel: &Arc<AtomicBool>) -> LuaResult<()> {
    let step = ctl.borrow().instr_step;
    let ctl = ctl.clone();
    let cancel = cancel.clone();
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(step as u32),
        move |_, _| {
            if cancel.load(Ordering::Relaxed) {
                return Err(LuaError::RuntimeError(MARK_CANCELLED.into()));
            }
            let Ok(mut c) = ctl.try_borrow_mut() else {
                return Ok(VmState::Continue);
            };
            if c.deadline.is_some_and(|d| Instant::now() >= d) {
                return Err(LuaError::RuntimeError(MARK_TIMEOUT.into()));
            }
            c.instr_used = c.instr_used.saturating_add(c.instr_step);
            if c.instr_limit > 0 && c.instr_used > c.instr_limit {
                return Err(LuaError::RuntimeError(MARK_BUDGET.into()));
            }
            Ok(VmState::Continue)
        },
    )
}

/// The same `package.path` the game VM uses, so a compute module can `require`
/// the same shared libraries and static data.
fn set_package_path(lua: &Lua, mudlib: &Path, game: &Path) -> LuaResult<()> {
    use crate::lua_path::abs_lua_path;
    let new_path = format!(
        "{game}/?.lua;{game}/?/init.lua;{mudlib}/?.lua;{mudlib}/?/init.lua",
        game = abs_lua_path(game),
        mudlib = abs_lua_path(mudlib),
    );
    lua.load(format!(
        "package.path = \"{};\" .. package.path",
        new_path.replace('"', "\\\"")
    ))
    .set_name("=<oxigeon>/compute_path")
    .exec()
}

/// What one job produced.
pub struct Outcome {
    pub ending: Ending,
    pub value: LuaData,
    pub error: Option<String>,
    pub logs: Vec<(String, String)>,
}

impl ComputeVm {
    /// The flag a `Cancel` frame sets.
    ///
    /// Handed to the worker's stdin reader, which is the only other thread that
    /// touches this VM's job — and touches nothing else about it, because
    /// everything else here belongs to the Lua thread.
    ///
    /// Setting it only stops a job if a budget is armed: without the hook there
    /// is nothing inside the VM that checks, except the job itself via
    /// `compute_cancelled()`. The server's fallback is to kill the process.
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    /// Run `module.func(args)`. Never panics; every failure is an [`Ending`].
    pub fn run(
        &self,
        module: &str,
        func: &str,
        args: &LuaData,
        deadline: Option<Instant>,
    ) -> Outcome {
        self.cancel.store(false, Ordering::Relaxed);
        self.ctl.borrow_mut().begin(deadline);

        let outcome = self.run_inner(module, func, args);
        let logs = self.ctl.borrow_mut().take_logs();

        match outcome {
            Ok(value) => Outcome { ending: Ending::Ok, value, error: None, logs },
            Err((ending, error)) => {
                Outcome { ending, value: LuaData::Nil, error: Some(error), logs }
            }
        }
    }

    fn run_inner(
        &self,
        module: &str,
        func: &str,
        args: &LuaData,
    ) -> Result<LuaData, (Ending, String)> {
        // `require` caches, so this is only expensive the first time a worker
        // sees a module. A reload recycles the whole VM rather than picking at
        // `package.loaded` — see `ComputeBridge::recycle`.
        let table: LuaTable = self
            .lua
            .globals()
            .get::<LuaFunction>("require")
            .and_then(|require| require.call(module))
            .map_err(|e| (Ending::LoadError, format!("could not load '{module}': {e}")))?;

        let f: LuaFunction = table.get(func).map_err(|e| {
            (
                Ending::LoadError,
                format!("'{module}' has no function '{func}': {e}"),
            )
        })?;

        let lua_args = marshal::to_lua(&self.lua, args)
            .map_err(|e| (Ending::Error, format!("could not rebuild arguments: {e}")))?;

        let result: LuaValue = f.call(lua_args).map_err(|e| classify(&e))?;

        marshal::from_lua(&result, &self.limits).map_err(|e| {
            (
                Ending::Error,
                format!("the job's return value cannot cross back: {e}"),
            )
        })
    }
}

/// Map a Lua error onto an [`Ending`], recognising the hook's markers.
fn classify(e: &LuaError) -> (Ending, String) {
    let text = e.to_string();
    if text.contains(MARK_CANCELLED) {
        (Ending::Cancelled, "cancelled".to_string())
    } else if text.contains(MARK_TIMEOUT) {
        (Ending::Timeout, "deadline passed while the job was running".to_string())
    } else if text.contains(MARK_BUDGET) {
        (Ending::Budget, "instruction budget exhausted".to_string())
    } else {
        (Ending::Error, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A compute root with one module in it.
    fn workspace(body: &str) -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().unwrap();
        let compute = dir.path().join("compute");
        std::fs::create_dir_all(&compute).unwrap();
        let mut f = std::fs::File::create(compute.join("probe.lua")).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        dir
    }

    const PROBE: &str = r#"
local M = {}
function M.echo(args) return args end
function M.boom() error("kaboom") end
function M.spin() while true do end end
function M.reach()
    return { send = tostring(send), io = tostring(io), state = tostring(set_object_state) }
end
function M.logs() compute_log("info", "hello from the worker") return true end
return M
"#;

    fn vm_with(cfg: ComputeSettings, dir: &tempfile::TempDir) -> ComputeVm {
        build(&cfg, dir.path(), dir.path(), 0).unwrap()
    }

    #[test]
    fn a_job_runs_and_its_value_comes_back() {
        let dir = workspace(PROBE);
        let vm = vm_with(ComputeSettings::default(), &dir);
        let args = marshal::from_lua(
            &Lua::new().load("return {n = 7, list = {1,2}}").eval::<LuaValue>().unwrap(),
            &Limits::default(),
        )
        .unwrap();

        let out = vm.run("compute.probe", "echo", &args, None);
        assert_eq!(out.ending, Ending::Ok, "{:?}", out.error);
        assert_eq!(out.value, args, "the value should survive both trips unchanged");
    }

    /// The compute analogue of `sandbox_reality_check`: what can a job see?
    #[test]
    fn a_job_cannot_reach_the_game_or_the_host() {
        let dir = workspace(PROBE);
        let vm = vm_with(ComputeSettings::default(), &dir);
        let out = vm.run("compute.probe", "reach", &LuaData::Nil, None);
        assert_eq!(out.ending, Ending::Ok, "{:?}", out.error);

        let LuaData::Table(t) = out.value else { panic!("expected a table") };
        for key in ["send", "io", "state"] {
            let got = t.map.get(&marshal::Key::Str(key.as_bytes().to_vec()));
            assert_eq!(
                got,
                Some(&LuaData::Str(b"nil".to_vec())),
                "`{key}` must not be reachable from a compute job"
            );
        }
    }

    #[test]
    fn a_job_that_raises_is_reported_not_swallowed() {
        let dir = workspace(PROBE);
        let vm = vm_with(ComputeSettings::default(), &dir);
        let out = vm.run("compute.probe", "boom", &LuaData::Nil, None);
        assert_eq!(out.ending, Ending::Error);
        assert!(out.error.unwrap().contains("kaboom"));
    }

    #[test]
    fn a_missing_module_or_function_is_a_load_error() {
        let dir = workspace(PROBE);
        let vm = vm_with(ComputeSettings::default(), &dir);
        assert_eq!(
            vm.run("compute.nope", "echo", &LuaData::Nil, None).ending,
            Ending::LoadError
        );
        assert_eq!(
            vm.run("compute.probe", "nope", &LuaData::Nil, None).ending,
            Ending::LoadError
        );
    }

    /// A worker whose module failed must still be usable — the VM is not
    /// poisoned by a bad job.
    #[test]
    fn a_worker_survives_a_failed_job() {
        let dir = workspace(PROBE);
        let vm = vm_with(ComputeSettings::default(), &dir);
        vm.run("compute.probe", "boom", &LuaData::Nil, None);
        assert_eq!(
            vm.run("compute.probe", "echo", &LuaData::Int(1), None).ending,
            Ending::Ok
        );
    }

    /// With a budget armed the compiler is off and the hook can stop a runaway
    /// job, so the worker comes back. This is what the budget buys.
    #[test]
    fn a_runaway_job_is_stopped_when_a_budget_is_armed() {
        let dir = workspace(PROBE);
        let cfg = ComputeSettings { instruction_limit: 500_000, ..Default::default() };
        let vm = vm_with(cfg, &dir);

        let out = vm.run("compute.probe", "spin", &LuaData::Nil, None);
        assert_eq!(out.ending, Ending::Budget, "{:?}", out.error);

        // And the worker is still good for the next job.
        assert_eq!(
            vm.run("compute.probe", "echo", &LuaData::Int(1), None).ending,
            Ending::Ok
        );
    }

    #[test]
    fn a_deadline_stops_a_running_job_when_a_budget_is_armed() {
        let dir = workspace(PROBE);
        let cfg = ComputeSettings { instruction_limit: 100_000_000, ..Default::default() };
        let vm = vm_with(cfg, &dir);

        let deadline = Instant::now() + std::time::Duration::from_millis(100);
        let started = Instant::now();
        let out = vm.run("compute.probe", "spin", &LuaData::Nil, Some(deadline));
        assert_eq!(out.ending, Ending::Timeout, "{:?}", out.error);
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    #[test]
    fn compute_log_lines_come_back_with_the_result() {
        let dir = workspace(PROBE);
        let vm = vm_with(ComputeSettings::default(), &dir);
        let out = vm.run("compute.probe", "logs", &LuaData::Nil, None);
        assert_eq!(out.ending, Ending::Ok);
        assert_eq!(out.logs.len(), 1);
        assert_eq!(out.logs[0].1, "hello from the worker");
    }
}
