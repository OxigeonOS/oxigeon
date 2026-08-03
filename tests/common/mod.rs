//! A handle on the VM the engine actually builds.
//!
//! Items 1 and 2 of the hardening review both survived a green suite because
//! the tests drove a helper in isolation — `create_sandboxed_env()`, a bare
//! `Lua::new()` — while production took a different path. Both stayed green
//! forever regardless of what the server did.
//!
//! This starts a real [`ScriptEngine`] against a temp mudlib and runs probe
//! source *inside it*, so a test asks the same question a player would: what
//! can Lua running in this server actually do? Anything that stops calling
//! `apply_sandbox`, or stops arming the instruction budget, breaks these.

// Each integration-test binary compiles this whole module, so helpers only one
// of them uses would otherwise warn in all the others.
#![allow(dead_code)]

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use tempfile::TempDir;
use tokio::sync::mpsc;

use oxigeon::config::{
    AccountsConfig, DatabaseBackend, DatabaseConfig, DebugServerConfig, GameConfig, LimitsConfig,
    MultisessionMode, PermissionConfig, ServerConfig, SessionsConfig,
};
use oxigeon::config::server_config::ComputeConfig;
use oxigeon::core::auth::AuthWorker;
use oxigeon::core::compute::ComputeBridge;
use oxigeon::core::logging::GameLogger;
use oxigeon::core::scripting::debugger::DebugState;
use oxigeon::core::scripting::efuns::EfunContext;
use oxigeon::core::scripting::{LuaCommand, ScriptEngine};
use oxigeon::core::session::{Session, SessionHandler, SessionOutput};
use oxigeon::domain::db::connection::AnyPool;
use oxigeon::domain::models::role::DieselRoleStore;
use oxigeon::domain::models::{DieselAccountStore, DieselCharacterStore};

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

/// How long to wait for the Lua thread to answer one probe.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// The result of running a probe chunk in the live VM.
#[derive(Debug, PartialEq, Eq)]
pub enum Probe {
    /// The chunk ran and returned this, rendered with `tostring`.
    Ok(String),
    /// The chunk raised, or failed to compile. Holds the message.
    Err(String),
}

impl Probe {
    /// The returned value, or a panic naming the error — for probes whose
    /// failure is never the thing under test.
    pub fn unwrap(self) -> String {
        match self {
            Probe::Ok(v) => v,
            Probe::Err(e) => panic!("probe failed: {e}"),
        }
    }

    pub fn is_err(&self) -> bool {
        matches!(self, Probe::Err(_))
    }

    /// The error message, or a panic — for probes that were meant to fail.
    pub fn err(self) -> String {
        match self {
            Probe::Err(e) => e,
            Probe::Ok(v) => panic!("probe was expected to fail, but returned {v:?}"),
        }
    }
}

/// What a finished compute job came back with.
#[derive(Debug, Clone)]
pub struct ComputeReply {
    pub id: String,
    /// `"ok"`, `"error"`, `"timeout"`, `"refused"`, ...
    pub kind: String,
    pub error: Option<String>,
    /// `value.marker` if the job returned a table, else the value `tostring`ed.
    pub value: String,
    pub tag: Option<String>,
}

/// What an asynchronous `authenticate` / `create_account` came back with.
#[derive(Debug, Clone)]
pub struct AuthReply {
    /// `"authenticate"` or `"create_account"`.
    pub kind: String,
    /// The account's username on success.
    pub username: Option<String>,
    /// The player-facing message on failure.
    pub error: Option<String>,
}

/// A running engine with one connected session to run probes through.
pub struct RealVm {
    engine: Option<ScriptEngine>,
    session_id: String,
    output: mpsc::Receiver<SessionOutput>,
    /// Auth results that arrived while a probe reply was being waited for.
    /// They are asynchronous, so they can land at any point.
    pending_auth: std::collections::VecDeque<AuthReply>,
    /// Probe replies that arrived while an auth result was being waited for.
    pending_probe: std::collections::VecDeque<Probe>,
    pending_compute: std::collections::VecDeque<ComputeReply>,
    /// The same database the VM writes to, for a test that needs to ask what
    /// actually reached disk.
    pool: AnyPool,
    // Held only so the temp directories outlive the engine. `_mudlib` is
    // `None` when running the repository's real mudlib, which is not a temp
    // directory and must not be cleaned up.
    _mudlib: Option<TempDir>,
    _game: Option<TempDir>,
    _logs: Option<TempDir>,
    _db: TempDir,
}

/// A game layer that loads the real one and then replaces the command
/// dispatcher with a probe.
///
/// `game/init.lua` is loaded *after* `mudlib/init.lua` and both define globals,
/// so a game layer can override `on_input`. That is the whole trick behind
/// [`RealVm::boot_real_mudlib_with_probe`]. `{game}` is the repository's real
/// game directory, put on `package.path` so its content — traits, effects,
/// areas, mobs — registers exactly as it does in production before the
/// override happens.
///
/// `on_shutdown` is *wrapped* rather than replaced, so the mudlib's real one
/// still runs and the probe only observes that it did.
const PROBE_GAME_LAYER: &str = r#"
package.path = "{game}/?.lua;{game}/?/init.lua;" .. package.path

local loaded, err = pcall(require, 'init')
if not loaded then
    log("error", "probe: the real game layer failed to load: " .. tostring(err))
end

local mudlib_shutdown = on_shutdown

function on_input(session_id, text)
    local chunk, err2 = load(text, "=probe")
    if not chunk then
        send(session_id, "COMPILE	" .. tostring(err2))
        return
    end
    local ok, res = pcall(chunk)
    send(session_id, (ok and "OK	" or "ERR	") .. tostring(res))
end

_shutdown_session = nil
function on_shutdown()
    if mudlib_shutdown then
        local ok, serr = pcall(mudlib_shutdown)
        if not ok then log("error", "probe: the mudlib's on_shutdown failed: " .. tostring(serr)) end
    end
    if _shutdown_session then send(_shutdown_session, "OK	ran") end
end
"#;

impl RealVm {
    /// Boot the way `config/server.toml` ships: no instruction limit, so the
    /// LuaJIT compiler stays on.
    pub fn boot() -> Self {
        Self::boot_with_instruction_limit(0)
    }

    /// Boot with a permission config in force, so gated efuns can be tested
    /// the way production reaches them.
    pub fn boot_with_permissions(permissions: PermissionConfig) -> Self {
        Self::boot_inner(0, permissions)
    }

    /// Boot with the compute pool running. `setup` is handed the mudlib root
    /// so a test can write the modules its jobs will `require`.
    pub fn boot_with_compute(compute: ComputeConfig, setup: impl FnOnce(&std::path::Path)) -> Self {
        let mudlib = TempDir::new().unwrap();
        write_probe_mudlib(mudlib.path());
        setup(mudlib.path());
        let path = mudlib.path().to_path_buf();
        let mut vm = Self::boot_inner_at(
            Some(mudlib),
            path,
            TestCtx {
                compute,
                max_connections: 8,
                max_characters_per_account: 1,
                ..Default::default()
            },
        );
        assert_eq!(vm.eval("return 'ready'").unwrap(), "ready");
        vm
    }

    /// The engine handle, for a caller that needs to send a command directly.
    pub fn engine(&self) -> &ScriptEngine {
        self.engine.as_ref().unwrap()
    }

    /// This VM's session id.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// The database the VM writes to — for asking what actually reached disk.
    pub fn pool(&self) -> AnyPool {
        self.pool.clone()
    }

    /// Shut down the way the driver does on Ctrl+C: dispatch `on_shutdown` and
    /// wait for the Lua thread, bounded. Returns whether it finished in time.
    ///
    /// The VM is unusable afterwards — this is the end of a test, not a step
    /// in one.
    pub fn shutdown_within(&mut self, timeout: Duration) -> bool {
        self.engine.as_ref().unwrap().shutdown_within(timeout)
    }

    /// The next reply already sitting in the session's output queue.
    ///
    /// Unlike [`RealVm::eval`] this never waits on the Lua thread, so it can
    /// read what `on_shutdown` reported after the thread has stopped. Messages
    /// buffered before the sender dropped are still delivered.
    pub fn next_buffered_reply(&mut self) -> Option<Probe> {
        while let Ok(msg) = self.output.try_recv() {
            if let SessionOutput::Text(t) = msg {
                match parse_line(&t) {
                    Line::Probe(p) => return Some(p),
                    Line::Auth(a) => self.pending_auth.push_back(a),
                    Line::Compute(c) => self.pending_compute.push_back(c),
                }
            }
        }
        None
    }

    /// Boot with an explicit `limits.lua_instruction_limit`. Zero disables it
    /// and leaves the JIT enabled; any other value disables the JIT so the
    /// budget can actually be enforced.
    pub fn boot_with_instruction_limit(instruction_limit: u64) -> Self {
        Self::boot_inner(instruction_limit, PermissionConfig::default())
    }

    /// Boot against the repository's real `mudlib/` and `game/`.
    ///
    /// Every other constructor writes a throwaway mudlib that answers probes
    /// directly. This one runs the actual game — daemon load, command
    /// dispatch, rooms, the prompt — which is what a benchmark has to measure
    /// and what no synthetic workload can stand in for. The session comes back
    /// logged in and playing, via the real login flow.
    ///
    /// The database and the log directory are still temporary; only the Lua is
    /// real.
    pub fn boot_real_mudlib(instruction_limit: u64) -> Self {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let logs = TempDir::new().unwrap();
        let mut vm = Self::boot_inner_at(
            None,
            root.join("mudlib"),
            TestCtx {
                instruction_limit,
                game_path: Some(root.join("game")),
                start_room: Some("wizard_workshop.entrance".to_string()),
                log_dir: Some(logs.path().to_path_buf()),
                ..Default::default()
            },
        );
        vm._logs = Some(logs);
        vm.login_as("benchuser", "a good long benchmark password");
        vm
    }

    /// Boot the repository's real `mudlib/` behind a probe `on_input`.
    ///
    /// [`RealVm::boot_real_mudlib`] can only send commands, because the real
    /// mudlib's `on_input` *is* the command dispatcher. That makes anything
    /// without a player-facing verb untestable — every daemon's internals, the
    /// tickers, what `on_shutdown` writes — and the usual workaround is to ship
    /// a wizard command for each thing you wanted to test.
    ///
    /// This boots the same real mudlib with a throwaway game layer whose
    /// `init.lua` overrides `on_input` with the probe dispatcher, so `eval`
    /// works against the fully-wired `DAEMON` table, the real `journal_d`, the
    /// real config and the real document store.
    ///
    /// Two things it deliberately does not do: the real `game/` content is not
    /// loaded (no rooms, no start room, nobody logs in), and the real command
    /// dispatcher is not exercised — `boot_real_mudlib` already covers that.
    /// Use this to ask what mudlib code does; use that one to ask what a player
    /// experiences.
    pub fn boot_real_mudlib_with_probe() -> Self {
        Self::boot_real_mudlib_with_probe_opts(TestCtx::default())
    }

    /// As above, with control over the config — for a test that needs one of
    /// the periodic subsystems actually registered. `game_path` and `log_dir`
    /// are overwritten; everything else is honoured.
    pub fn boot_real_mudlib_with_probe_opts(mut opts: TestCtx) -> Self {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let logs = TempDir::new().unwrap();
        let game = TempDir::new().unwrap();

        let real_game = root
            .join("game")
            .to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches("//?/")
            .to_string();
        std::fs::write(
            game.path().join("init.lua"),
            PROBE_GAME_LAYER.replace("{game}", &real_game),
        )
        .unwrap();

        opts.game_path = Some(game.path().to_path_buf());
        opts.log_dir = Some(logs.path().to_path_buf());

        let mut vm = Self::boot_inner_at(None, root.join("mudlib"), opts);
        vm._logs = Some(logs);
        vm._game = Some(game);
        assert_eq!(vm.eval("return 'ready'").unwrap(), "ready");
        vm
    }

    fn boot_inner(instruction_limit: u64, permissions: PermissionConfig) -> Self {
        let mudlib = TempDir::new().unwrap();
        write_probe_mudlib(mudlib.path());
        let path = mudlib.path().to_path_buf();
        let mut vm = Self::boot_inner_at(
            Some(mudlib),
            path,
            TestCtx {
                instruction_limit,
                permissions,
                max_connections: 8,
                max_characters_per_account: 1,
                ..Default::default()
            },
        );
        // Do not return until the VM has actually started.
        //
        // `ScriptEngine::start` returns as soon as the thread is spawned, so
        // without this the caller gets a handle to a VM that has not yet built
        // its `Lua`, let alone read its configuration. `benches/dispatch.rs`
        // sets `OXIGEON_JIT` around a boot and cleared it on return — and
        // because of this gap the Lua thread read the variable *after* it was
        // cleared, so the benchmark's "JIT off" configuration silently ran
        // with the JIT on and reported that the compiler was worth nothing.
        assert_eq!(vm.eval("return 'ready'").unwrap(), "ready");
        vm
    }

    fn boot_inner_at(
        owned_mudlib: Option<TempDir>,
        mudlib_path: std::path::PathBuf,
        mut opts: TestCtx,
    ) -> Self {
        let (pool, db) = test_pool();
        let session_handler = Arc::new(RwLock::new(SessionHandler::new(
            MultisessionMode::Single,
            opts.max_connections,
        )));

        let (output_tx, output) = mpsc::channel::<SessionOutput>(1024);
        let session = Session::new("test".to_string(), "127.0.0.1:0".parse().unwrap(), output_tx);
        let session_id = session.id.to_string();
        session_handler.write().unwrap().connect(session).unwrap();

        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<LuaCommand>();
        opts.cmd_tx = Some(cmd_tx.clone());
        let ctx = efun_context(session_handler, mudlib_path.clone(), pool.clone(), opts);
        let engine = ScriptEngine::start(mudlib_path, ctx, cmd_tx, cmd_rx).unwrap();

        Self {
            engine: Some(engine),
            session_id,
            output,
            pending_auth: Default::default(),
            pending_probe: Default::default(),
            pending_compute: Default::default(),
            pool,
            _mudlib: owned_mudlib,
            _game: None,
            _logs: None,
            _db: db,
        }
    }

    /// Drive the real mudlib's login flow until the session is playing.
    ///
    /// Only meaningful against the real mudlib — the probe mudlibs have no
    /// login. Creates the account on first use, which costs one Argon2 hash.
    fn login_as(&mut self, user: &str, password: &str) {
        let sid = self.session_id.clone();
        self.engine().send(LuaCommand::OnConnect { session_id: sid.clone() });
        self.drain_for(Duration::from_millis(500));

        for line in ["new", user] {
            self.engine().send(LuaCommand::OnInput {
                session_id: sid.clone(),
                text: line.to_string(),
            });
            self.drain_for(Duration::from_millis(500));
        }

        self.engine().send(LuaCommand::OnInput {
            session_id: sid.clone(),
            text: password.to_string(),
        });

        // The password step hands off to the Argon2 worker and the session
        // goes silent for a few hundred milliseconds, so this cannot wait on a
        // quiet period the way the others do — it has to wait for the result
        // itself. `Welcome` means `enter_game` ran; `Username:` means the
        // login flow bounced us back to the start.
        let out = self.drain_until(
            |t| t.contains("Welcome") || t.contains("Username:"),
            Duration::from_secs(20),
        );

        assert!(
            out.contains("Welcome"),
            "login did not reach the game; output was: {out:?}"
        );

        // Let the rest of `enter_game` land — the room description and the
        // first prompt arrive after the "Welcome" line — then throw it all
        // away, so the first `command` starts against an empty channel rather
        // than reading login's prompt as its own completion.
        //
        // Waiting for a *quiet period* here was a race. Under load the session
        // is briefly quiet before that output is produced, so `drain_for`
        // returned early, `discard_pending` found nothing, and the first
        // `command` collected the tail of the login and stopped at login's
        // prompt — reporting the login banner as that command's output. It
        // showed up as a different real-mudlib test failing on each run, all of
        // which passed in isolation.
        //
        // `send_prompt` is the only thing that produces a `Raw`, so the prompt
        // is an exact marker rather than a guess. Fall back to the old
        // heuristic if one never comes, so a mudlib that does not prompt after
        // login still boots.
        if !self.drain_to_prompt(Duration::from_secs(10)) {
            self.drain_for(Duration::from_secs(2));
        }
        self.discard_pending();
    }

    /// Consume output up to and including the prompt that ends a dispatch.
    /// @return whether one arrived before the deadline
    fn drain_to_prompt(&mut self, timeout: Duration) -> bool {
        let started = Instant::now();
        let deadline = started + timeout;
        loop {
            match self.output.try_recv() {
                Ok(SessionOutput::Raw(_)) => return true,
                Ok(_) => continue,
                Err(mpsc::error::TryRecvError::Empty) => {
                    if Instant::now() >= deadline {
                        return false;
                    }
                    Self::wait_step(started);
                }
                Err(mpsc::error::TryRecvError::Disconnected) => return false,
            }
        }
    }

    /// Send one line as a playing character and wait for the game to finish
    /// with it, returning everything it sent back.
    ///
    /// Completion is the prompt: `mudlib/lib/commands.lua` renders one at the
    /// end of every dispatch, success or failure, and `send_prompt` is the
    /// only thing that produces a `Raw` message. Waiting on that rather than
    /// on a fixed sleep is what makes this usable as a benchmark body.
    pub fn command(&mut self, line: &str) -> String {
        // Anything still queued belongs to whatever ran before. Leaving it
        // would make this call return the *previous* command's output and stop
        // at the previous command's prompt — an off-by-one that silently makes
        // every assertion test the wrong thing.
        self.discard_pending();
        self.engine.as_ref().unwrap().send(LuaCommand::OnInput {
            session_id: self.session_id.clone(),
            text: line.to_string(),
        });
        self.collect_to_prompt(Duration::from_secs(10), line)
    }

    /// Throw away anything already in the channel.
    fn discard_pending(&mut self) {
        while self.output.try_recv().is_ok() {}
    }

    /// Read output until a prompt arrives or the deadline passes.
    fn collect_to_prompt(&mut self, timeout: Duration, what: &str) -> String {
        let started = Instant::now();
        let deadline = started + timeout;
        let mut text = String::new();
        loop {
            match self.output.try_recv() {
                Ok(SessionOutput::Text(t)) => text.push_str(&t),
                // The prompt. End of dispatch.
                Ok(SessionOutput::Raw(_)) => return text,
                Ok(_) => continue,
                Err(mpsc::error::TryRecvError::Empty) => {
                    if Instant::now() >= deadline {
                        panic!("no prompt within {timeout:?} after {what:?}; got: {text:?}");
                    }
                    Self::wait_step(started);
                }
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    panic!("the session output channel closed while running {what:?}")
                }
            }
        }
    }

    /// Drain whatever arrives, returning the text, until the session goes
    /// quiet or the window expires.
    ///
    /// Used only during login, where there is no single reliable end-of-step
    /// marker — the prompt trick `command` relies on does not apply, because
    /// the login flow is not the command dispatcher.
    fn drain_for(&mut self, window: Duration) -> String {
        let deadline = Instant::now() + window;
        let mut text = String::new();
        let mut quiet_since: Option<Instant> = None;
        while Instant::now() < deadline {
            match self.output.try_recv() {
                Ok(SessionOutput::Text(t)) => {
                    text.push_str(&t);
                    quiet_since = None;
                }
                Ok(_) => quiet_since = None,
                Err(mpsc::error::TryRecvError::Empty) => {
                    // Quiet for a moment means the step is done. Waiting out
                    // the whole window on every step would put tens of seconds
                    // of dead time into every benchmark run.
                    match quiet_since {
                        Some(t) if t.elapsed() > Duration::from_millis(20) => break,
                        Some(t) => Self::wait_step(t),
                        None => quiet_since = Some(Instant::now()),
                    }
                }
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
        text
    }

    /// Accumulate output until `done` is satisfied, ignoring quiet periods.
    ///
    /// Needed wherever a step is answered asynchronously — the session goes
    /// silent while a worker thread is busy, so "quiet" does not mean
    /// "finished".
    fn drain_until(&mut self, done: impl Fn(&str) -> bool, timeout: Duration) -> String {
        let started = Instant::now();
        let deadline = started + timeout;
        let mut text = String::new();
        loop {
            match self.output.try_recv() {
                Ok(SessionOutput::Text(t)) => {
                    text.push_str(&t);
                    if done(&text) {
                        return text;
                    }
                }
                Ok(_) => continue,
                Err(mpsc::error::TryRecvError::Empty) => {
                    if Instant::now() >= deadline {
                        return text;
                    }
                    Self::wait_step(started);
                }
                Err(mpsc::error::TryRecvError::Disconnected) => return text,
            }
        }
    }

    /// Wait for more output without burning a core.
    ///
    /// The Lua thread usually answers in tens of microseconds, so a fixed
    /// sleep is the wrong tool: a 5 ms quantum is two orders of magnitude
    /// larger than a command dispatch and made every `eval`-based benchmark
    /// read 5.4 ms regardless of what it was measuring. This yields while an
    /// answer is plausibly imminent and only falls back to sleeping once the
    /// wait is clearly not about scheduling latency.
    fn wait_step(waiting_since: Instant) {
        if waiting_since.elapsed() < Duration::from_millis(20) {
            std::thread::yield_now();
        } else {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Run `src` inside the live VM, exactly as if a player had typed a command
    /// that reached it. Returns whatever the chunk returned, `tostring`ed.
    pub fn eval(&mut self, src: &str) -> Probe {
        // Probe source travels as one input line; the driver splits on newlines
        // before this point, so keep chunks to a single line.
        assert!(
            !src.contains('\n'),
            "probe source must be one line — the input path is line-oriented"
        );
        self.engine.as_ref().unwrap().send(LuaCommand::OnInput {
            session_id: self.session_id.clone(),
            text: src.to_string(),
        });

        if let Some(queued) = self.pending_probe.pop_front() {
            return queued;
        }
        self.pump(src, |vm| vm.pending_probe.pop_front())
    }

    /// Wait for the next asynchronous `on_auth_result`.
    pub fn next_auth_result(&mut self) -> AuthReply {
        if let Some(queued) = self.pending_auth.pop_front() {
            return queued;
        }
        self.pump("on_auth_result", |vm| vm.pending_auth.pop_front())
    }

    /// Wait for the next `on_compute_result`.
    pub fn next_compute_result(&mut self) -> ComputeReply {
        if let Some(queued) = self.pending_compute.pop_front() {
            return queued;
        }
        self.pump("on_compute_result", |vm| vm.pending_compute.pop_front())
    }

    /// Whether a compute result has already arrived, without waiting.
    pub fn compute_result_ready(&mut self) -> bool {
        while let Ok(msg) = self.output.try_recv() {
            if let SessionOutput::Text(t) = msg {
                match parse_line(&t) {
                    Line::Probe(p) => self.pending_probe.push_back(p),
                    Line::Auth(a) => self.pending_auth.push_back(a),
                    Line::Compute(c) => self.pending_compute.push_back(c),
                }
            }
        }
        !self.pending_compute.is_empty()
    }

    /// Read session output until `take` yields something, sorting each line
    /// into the queue it belongs to. The two reply kinds are independent —
    /// an auth result can land in the middle of a probe round trip — so
    /// whichever is not being waited for is kept rather than dropped.
    fn pump<T>(&mut self, what: &str, take: fn(&mut Self) -> Option<T>) -> T {
        let started = Instant::now();
        let deadline = started + PROBE_TIMEOUT;
        loop {
            match self.output.try_recv() {
                Ok(SessionOutput::Text(t)) => {
                    match parse_line(&t) {
                        Line::Probe(p) => self.pending_probe.push_back(p),
                        Line::Auth(a) => self.pending_auth.push_back(a),
                        Line::Compute(c) => self.pending_compute.push_back(c),
                    }
                    if let Some(v) = take(self) {
                        return v;
                    }
                }
                Ok(_) => continue,
                Err(mpsc::error::TryRecvError::Empty) => {
                    assert!(
                        Instant::now() < deadline,
                        "the Lua thread did not answer within {PROBE_TIMEOUT:?} — waiting on: {what}"
                    );
                    // This used to be a flat 5 ms sleep, which is fine for a
                    // pass/fail test and useless for a benchmark: every
                    // `eval`-based measurement came back as 5.4 ms whatever it
                    // was measuring, including the "control" that was supposed
                    // to prove the JIT toggle worked.
                    Self::wait_step(started);
                }
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    panic!("the session output channel closed — waiting on: {what}")
                }
            }
        }
    }

    /// Run `src` from inside a timer dispatch and return what it produced.
    ///
    /// The point is the *dispatch*, not the delay: a tick reaches Lua through
    /// `LuaCommand::TimerFired`, which has no session behind it, and that is
    /// what decides whether a gated efun is allowed.
    pub fn eval_on_timer(&mut self, src: &str) -> Probe {
        assert!(!src.contains('\n'), "timer probe source must be one line");
        self.eval(&format!("_timer_result = nil _timer_source = [[{src}]]"))
            .unwrap();

        self.engine
            .as_ref()
            .unwrap()
            .send(LuaCommand::TimerFired { id: "probe".to_string() });

        // The engine processes commands in order, so by the time this probe is
        // answered the tick above has already run.
        match self.eval("return _timer_result").unwrap().as_str() {
            "nil" => panic!("the timer dispatch did not run"),
            reply => match parse_line(reply) {
                Line::Probe(p) => p,
                other => unreachable!("a timer result is always a probe reply, got {other:?}"),
            },
        }
    }

    /// Whether a global is reachable from unprivileged mudlib code.
    pub fn reaches(&mut self, expr: &str) -> bool {
        self.eval(&format!("return tostring(({expr}) ~= nil)")).unwrap() == "true"
    }
}

impl Drop for RealVm {
    fn drop(&mut self) {
        // Deliberately *not* `ScriptEngine::shutdown`, which joins the Lua
        // thread. A test that fails because the VM is wedged would then hang
        // here instead of reporting, turning a clear failure into a stuck
        // suite. Dropping sends the shutdown command and moves on.
        drop(self.engine.take());
    }
}

#[derive(Debug)]
enum Line {
    Probe(Probe),
    Auth(AuthReply),
    Compute(ComputeReply),
}

/// Decode one tab-separated reply line from the probe mudlib.
fn parse_line(text: &str) -> Line {
    let mut fields = text.split('\t');
    match fields.next() {
        Some("COMPUTE") => {
            let mut next = || fields.next().unwrap_or_default().to_string();
            let id = next();
            let kind = next();
            let error = Some(next()).filter(|s| !s.is_empty());
            let value = next();
            let tag = Some(next()).filter(|s| !s.is_empty());
            Line::Compute(ComputeReply { id, kind, error, value, tag })
        }
        Some("AUTH") => {
            let kind = fields.next().unwrap_or_default().to_string();
            let username = fields.next().filter(|s| !s.is_empty()).map(str::to_string);
            let error = fields.next().filter(|s| !s.is_empty()).map(str::to_string);
            Line::Auth(AuthReply { kind, username, error })
        }
        Some("OK") => Line::Probe(Probe::Ok(fields.collect::<Vec<_>>().join("\t"))),
        Some(_) => Line::Probe(Probe::Err(fields.collect::<Vec<_>>().join("\t"))),
        None => Line::Probe(Probe::Err(format!("malformed probe reply: {text:?}"))),
    }
}

/// A mudlib whose only job is to run whatever it is sent and report back.
///
/// It goes through `load` + `pcall` deliberately: that is the same path a
/// command file takes, so a probe sees the sandbox exactly as game content
/// does.
fn write_probe_mudlib(root: &std::path::Path) {
    let init = r#"
function on_connect(session_id) end
function on_disconnect(session_id) end
function on_gmcp(session_id, package, data) end
function on_unload(m) end
function on_load(m) end

-- Timer ticks have no session behind them. `_timer_source` is set by a probe
-- and run here so a test can ask what a daemon tick can actually reach; the
-- answer lands in `_timer_result` for the next probe to read.
_timer_result = nil
function on_timer(id)
    if not _timer_source then return end
    local chunk, err = load(_timer_source, "=timer")
    if not chunk then
        _timer_result = "COMPILE\t" .. tostring(err)
        return
    end
    local ok, res = pcall(chunk)
    _timer_result = (ok and "OK\t" or "ERR\t") .. tostring(res)
end

-- A clean shutdown dispatches this before the VM stops. Like `on_timer` it has
-- no session behind it, so a probe sets `_shutdown_session` to say where the
-- answer should go and `_shutdown_source` to say what should run inside the
-- dispatch — which is the only way to ask what the shutdown hook may do.
_shutdown_session = nil
_shutdown_source = nil
function on_shutdown()
    local reply = "OK\tran"
    if _shutdown_source then
        local chunk, err = load(_shutdown_source, "=shutdown")
        if not chunk then
            reply = "COMPILE\t" .. tostring(err)
        else
            local ok, res = pcall(chunk)
            reply = (ok and "OK\t" or "ERR\t") .. tostring(res)
        end
    end
    if _shutdown_session then send(_shutdown_session, reply) end
end

function on_input(session_id, text)
    local chunk, err = load(text, "=probe")
    if not chunk then
        send(session_id, "COMPILE\t" .. tostring(err))
        return
    end
    local ok, res = pcall(chunk)
    send(session_id, (ok and "OK\t" or "ERR\t") .. tostring(res))
end

-- Asynchronous auth results arrive here, on their own reply tag so they can
-- never be mistaken for the answer to a probe.
function on_auth_result(session_id, kind, account, err)
    send(session_id, "AUTH\t" .. tostring(kind)
        .. "\t" .. tostring(account and account.username or "")
        .. "\t" .. tostring(err or ""))
end

-- Compute results have no session behind them, so they are reported to the
-- one session this harness has. `_compute_session` is set by the first probe.
_compute_session = nil
function on_compute_result(id, ok, value, err, meta)
    -- Kept so a test can deep-compare it against what it sent, in Lua, which
    -- is the only way to check that the pair of conversions is an identity.
    _last_compute_value = value
    if not _compute_session then return end
    send(_compute_session, "COMPUTE\t" .. tostring(id)
        .. "\t" .. tostring(meta.kind)
        .. "\t" .. tostring(err or "")
        .. "\t" .. tostring(type(value) == "table" and (value.marker or "table") or value)
        .. "\t" .. tostring(meta.tag or ""))
end
"#;
    std::fs::write(root.join("init.lua"), init).unwrap();
}

/// A temp SQLite database with the migrations applied.
///
/// `pool_size: 1` deliberately — it is the setting most likely to expose a
/// design that holds a connection across a callback, which would deadlock.
pub fn test_pool() -> (AnyPool, TempDir) {
    let dir = TempDir::new().unwrap();
    let config = DatabaseConfig {
        backend: DatabaseBackend::Sqlite,
        url: dir.path().join("test.db").to_string_lossy().to_string(),
        pool_size: 1,
    };
    let pool = AnyPool::new(&config).unwrap();
    pool.get_sqlite()
        .unwrap()
        .run_pending_migrations(MIGRATIONS)
        .unwrap();
    (pool, dir)
}

/// What a test wants to vary about its [`EfunContext`].
///
/// This exists so there is exactly one place in the test tree that names every
/// `EfunContext` field. There used to be four near-identical copies, so adding
/// one field to the context meant four mechanical edits before anything would
/// compile — and the next two features each add one.
pub struct TestCtx {
    /// `limits.lua_instruction_limit`. Non-zero also turns the JIT off.
    pub instruction_limit: u64,
    pub permissions: PermissionConfig,
    /// The engine command channel. `Some` also starts an auth worker, since
    /// that is the only way it has to answer.
    pub cmd_tx: Option<mpsc::UnboundedSender<LuaCommand>>,
    /// Where the audit and journal files go. Defaults to `<mudlib>/logs`.
    pub log_dir: Option<std::path::PathBuf>,
    /// `game.game_path`. Defaults to `<mudlib>/game`, which is where the
    /// throwaway probe mudlibs put theirs; the real game layer is a sibling of
    /// `mudlib/`, not a child, so `boot_real_mudlib` sets this.
    pub game_path: Option<std::path::PathBuf>,
    /// `game.start_room`. Only the real mudlib needs one.
    pub start_room: Option<String>,
    /// The `[compute]` block. Disabled unless a test asks for it.
    pub compute: ComputeConfig,
    /// The periodic subsystems, all off by default for the same reason
    /// `autosave_seconds` is: a ticker that fires mid-test injects work
    /// nothing asked for. Set one when the registration itself is what is
    /// under test.
    pub cache_flush_seconds: Option<u64>,
    pub effect_sweep_seconds: Option<u64>,
    pub effect_heartbeat_seconds: Option<u64>,
    pub combat_round_seconds: Option<u64>,
    /// Ceilings on the document store. Default unless a test wants to drive a
    /// limit — `max_bytes` low enough to reject a write is the only honest way
    /// to test the cache's oversize path.
    pub documents: oxigeon::domain::models::document::DocumentLimits,
    pub max_connections: usize,
    pub max_characters_per_account: usize,
    /// Extra `[game]` keys, flattened into `GameConfig::extra`.
    pub game_extra: std::collections::HashMap<String, toml::Value>,
}

impl Default for TestCtx {
    fn default() -> Self {
        Self {
            instruction_limit: 0,
            permissions: PermissionConfig::default(),
            cmd_tx: None,
            log_dir: None,
            game_path: None,
            start_room: None,
            compute: ComputeConfig::default(),
            cache_flush_seconds: Some(0),
            effect_sweep_seconds: Some(0),
            effect_heartbeat_seconds: Some(0),
            combat_round_seconds: Some(0),
            documents: Default::default(),
            max_connections: 256,
            max_characters_per_account: 5,
            game_extra: Default::default(),
        }
    }
}

/// Build an [`EfunContext`] the way the driver does.
///
/// Read the logger back off `ctx.game_logger` if a test needs it.
pub fn efun_context(
    session_handler: Arc<RwLock<SessionHandler>>,
    mudlib_path: std::path::PathBuf,
    pool: AnyPool,
    opts: TestCtx,
) -> EfunContext {
    let server_config = ServerConfig {
        game: GameConfig {
            name: "TestMUD".to_string(),
            mudlib_path: mudlib_path.to_string_lossy().to_string(),
            game_path: Some(
                opts.game_path
                    .clone()
                    .unwrap_or_else(|| mudlib_path.join("game"))
                    .to_string_lossy()
                    .to_string(),
            ),
            command_paths: None,
            start_room: opts.start_room.clone(),
            // Zero, so a ticker task never fires mid-test and injects work
            // nothing asked for.
            area_reset_seconds: Some(0),
            autosave_seconds: Some(0),
            shutdown_timeout_seconds: None,
            // Every periodic subsystem is off by default for the same reason
            // the two above are: a ticker that fires mid-test injects work
            // nothing asked for. A test that wants one drives it explicitly
            // with `LuaCommand::TimerFired`, which is also more honest — it
            // proves the timer id the daemon registered is the one the engine
            // dispatches.
            cache_flush_seconds: opts.cache_flush_seconds,
            cache_flush_budget: None,
            cache_evict_seconds: Some(0),
            cooldown_durable_seconds: None,
            effect_sweep_seconds: opts.effect_sweep_seconds,
            effect_heartbeat_seconds: opts.effect_heartbeat_seconds,
            combat_round_seconds: opts.combat_round_seconds,
            // Game-layer settings the driver has no opinion about, reachable
            // from Lua as `config("game.<key>")`. A test that drives one — the
            // respawn room, a restock interval — sets it here.
            extra: opts.game_extra.clone(),
        },
        sessions: SessionsConfig {
            multisession_mode: MultisessionMode::Single,
            max_connections: opts.max_connections,
        },
        accounts: AccountsConfig {
            allow_creation: true,
            min_password_length: 6,
            max_characters_per_account: opts.max_characters_per_account,
        },
        limits: LimitsConfig {
            lua_memory_mb: 64,
            lua_instruction_limit: opts.instruction_limit,
            input_buffer_bytes: 4096,
        },
        compute: opts.compute.clone(),
        documents: opts.documents.clone(),
    };

    let account_store = Arc::new(DieselAccountStore::new(pool.clone(), 6));
    let document_store = Arc::new(
        oxigeon::domain::models::DieselDocumentStore::new(pool.clone(), opts.documents.clone())
            .expect("document store"),
    );
    // One worker: the tests care that hashing happens off the Lua thread and
    // comes back, not that it happens in parallel. No channel means no way to
    // answer, so no worker.
    let auth_worker = opts
        .cmd_tx
        .as_ref()
        .map(|tx| AuthWorker::start(account_store.clone(), tx.clone(), 1));

    // Same story: no channel means nothing to answer on, so no pool.
    let compute = opts.cmd_tx.as_ref().and_then(|tx| {
        ComputeBridge::start(
            opts.compute.clone(),
            mudlib_path.clone(),
            opts.game_path.clone().unwrap_or_else(|| mudlib_path.join("game")),
            tx.clone(),
        )
    });

    let log_dir = opts.log_dir.unwrap_or_else(|| mudlib_path.join("logs"));

    EfunContext {
        session_handler,
        account_store,
        character_store: Arc::new(DieselCharacterStore::new(
            pool.clone(),
            opts.max_characters_per_account,
        )),
        role_store: Arc::new(DieselRoleStore::new(pool)),
        server_config: Arc::new(server_config),
        mudlib_path,
        cmd_tx: opts.cmd_tx,
        permission_config: Arc::new(opts.permissions),
        game_logger: Arc::new(GameLogger::new(&log_dir)),
        started_at: Instant::now(),
        started_at_utc: "2026-01-01T00:00:00Z".to_string(),
        // The same call the driver makes, so the instruction budget is armed
        // the same way it is in production.
        debug_state: DebugState::from_config(
            &DebugServerConfig::default(),
            opts.instruction_limit,
        ),
        auth_worker,
        compute,
        document_store,
    }
}
