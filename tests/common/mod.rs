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
use oxigeon::core::scripting::debugger::{DebugState, SharedDebugState};
use oxigeon::core::scripting::efuns::EfunContext;
use oxigeon::core::scripting::{LuaCommand, ScriptEngine};
use oxigeon::core::session::{Session, SessionHandler, SessionOutput};
use oxigeon::domain::db::connection::AnyPool;
use oxigeon::domain::models::role::DieselRoleStore;
use oxigeon::domain::models::{DieselAccountStore, DieselCharacterStore};

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

/// How long to wait for the Lua thread to answer one probe.
///
/// This is a **wedged-thread backstop, not a latency assertion**. No test reads
/// it, and the one place latency is actually claimed says so itself — see
/// `a_deadline_is_enforced` in `tests/driver/compute_wedge.rs`, which asserts a
/// bound with its own `Instant`.
///
/// It was 10 seconds, which was comfortable while the suite was ~60 small test
/// binaries: cargo runs binaries in parallel but each held few threads. Merged
/// into three, the in-process parallelism went up sharply, and a probe that
/// waits on something spawning an OS process — the compute pool respawning its
/// only worker after a kill — could genuinely take longer than that on a loaded
/// machine. Raising it costs only how long a real hang takes to be reported.
const PROBE_TIMEOUT: Duration = Duration::from_secs(45);

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

/// Build the `oxigeon-compute` binary once per test binary, and say where it is.
///
/// A compute worker is a separate *crate* — it links LuaJIT unconditionally, and
/// cargo unifies features across one invocation, so it cannot be a default
/// workspace member without breaking every `lua55` build. That means `cargo test`
/// does not build it, and a test that needs one has to.
///
/// Built into its own target directory on purpose. Reusing `target/` would have
/// this cargo contending for the build lock the outer `cargo test` may still
/// hold, which shows up as a test run that hangs with no output — much worse
/// than the disk cost of a second cached LuaJIT.
pub fn compute_worker_binary() -> &'static std::path::Path {
    static BUILT: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    BUILT.get_or_init(|| {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let target = root.join("target").join("compute-worker");
        let out = std::process::Command::new(env!("CARGO"))
            .args(["build", "-p", "oxigeon-compute"])
            .current_dir(root)
            .env("CARGO_TARGET_DIR", &target)
            .output()
            .expect("could not run cargo to build the compute worker");
        assert!(
            out.status.success(),
            "building oxigeon-compute failed:
{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let bin = target
            .join("debug")
            .join(format!("oxigeon-compute{}", std::env::consts::EXE_SUFFIX));
        assert!(bin.is_file(), "cargo reported success but {} is missing", bin.display());
        bin
    })
    .as_path()
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
    /// GMCP pushed to this session, kept rather than discarded. See
    /// `discard_pending`.
    gmcp_seen: Vec<(String, serde_json::Value)>,
    /// The same handler the engine holds, so a test can set a capability the
    /// driver's telnet loop would have set.
    session_handler: Arc<RwLock<SessionHandler>>,
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
/// Where the fixture world starts. Not a room in `game.example/`.
pub const FIXTURE_START_ROOM: &str = "fixture.hall";

/// Where the example world starts.
///
/// A constant rather than a read of `config/server.toml`, because that config
/// points at the game being developed in `game/` and has nothing to say about
/// `game.example/`. The demo's entrance is a property of the demo, in the same
/// way [`FIXTURE_START_ROOM`] is a property of the fixture.
pub const EXAMPLE_START_ROOM: &str = "wizard_workshop.entrance";

/// The mudlib this repository ships and this suite tests.
///
/// **`mudlib.default/`, never `mudlib/`.** The two live side by side and hold
/// different things: `mudlib.default/` is tracked here and is what a fresh
/// checkout gets, while `mudlib/` is the working copy a creator brings in from
/// their own private repo — untracked, absent on a clean clone, and free to
/// have diverged arbitrarily. A suite that booted `mudlib/` would be testing
/// somebody's unpublished fork and would fail on a machine that has none.
///
/// It follows that a fix meant for upstream is made *here*, in
/// `mudlib.default/`, which is the only copy any test or reviewer sees.
pub fn default_mudlib_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("mudlib.default")
}

/// The world this repository ships and this suite tests.
///
/// **`game.example/`, never `game/`**, for the reason spelled out on
/// [`default_mudlib_root`]. `game/` is the creator's own game; the demo world
/// the `tests/demo_world/` bucket asserts against is this one.
pub fn example_game_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("game.example")
}

/// A complete, self-contained game layer in one file.
///
/// Everything the mudlib needs a *game* to supply, and nothing else: a trait
/// set (traits are game-layer by design, so without them nothing has hit
/// points), three rooms in a line, one creature and one item. It is the answer
/// to "what is the smallest world in which the mudlib still works" — which is
/// exactly the question a test of mudlib mechanics should be asking.
const FIXTURE_WORLD: &str = r#"
DAEMON.trait.define_all({
    { id = "strength",     label = "Strength",     kind = "attribute",
      group = "attributes", default = 10, min = 1 },
    { id = "dexterity",    label = "Dexterity",    kind = "attribute",
      group = "attributes", default = 10, min = 1 },
    { id = "constitution", label = "Constitution", kind = "attribute",
      group = "attributes", default = 10, min = 1 },
    { id = "intelligence", label = "Intelligence", kind = "attribute",
      group = "attributes", default = 10, min = 1 },
    { id = "wisdom",       label = "Wisdom",       kind = "attribute",
      group = "attributes", default = 10, min = 1 },
    { id = "level", label = "Level", kind = "counter",
      group = "vitals", default = 1, min = 1 },

    -- An authored maximum, so a fixture creature can be weak. Same shape as
    -- the shipped game: `max_hp` is derived and stores nothing.
    { id = "max_hp_flat", label = "Authored Max Health", kind = "attribute",
      group = "derived", default = 0, min = 0, hidden = true },
    { id = "max_hp", label = "Max Health", kind = "derived", group = "derived",
      depends = { "constitution", "level", "max_hp_flat" }, min = 1, round = "floor",
      formula = function(t)
          if t.max_hp_flat > 0 then return t.max_hp_flat end
          return 50 + t.constitution * 5 + (t.level - 1) * 10
      end },
    { id = "max_mp", label = "Max Mana", kind = "derived", group = "derived",
      depends = { "intelligence", "level" }, min = 0, round = "floor",
      formula = function(t) return 20 + t.intelligence * 3 end },

    { id = "hp", label = "Health", kind = "gauge", group = "vitals",
      max = "max_hp", min = 0, round = "floor",
      regen = { rate = 1, per = 3, target = "max", offline = false } },
    { id = "mp", label = "Mana", kind = "gauge", group = "vitals",
      max = "max_mp", min = 0, round = "floor",
      regen = { rate = 1, per = 5, target = "max", offline = false } },

    -- In no seed set, so nobody has it until something teaches them. That is
    -- what a skill is, and it is how an ability's `rank_trait` decides presence
    -- by storage rather than by a declaration.
    { id = "fixture_skill", label = "Fixture Skill", kind = "counter",
      category = "skill", group = "skills", sets = false, min = 0 },

    -- ─── The combat clock ────────────────────────────────────────────────────
    --
    -- A fixture world needs a round length, or `queue_d` falls back to the flat
    -- `game.combat_round_seconds` and every test of pacing passes by measuring
    -- a constant. That is not hypothetical: the first version of
    -- `tests/mudlib/swing_rate.rs` asserted a 3.0s baseline and got it from the
    -- fallback, in a world with no such trait.
    --
    -- Deliberately **not** the shipped game's formula. This one is coarser, so
    -- nobody mistakes it for a statement about balance — the mudlib tests assert
    -- that encumbrance and strength move the number, and `demo_world` asserts
    -- what this game's numbers actually are.
    { id = "encumbrance", label = "Encumbrance", kind = "attribute",
      group = "derived", default = 0, min = 0, hidden = true },
    { id = "round_length", label = "Round Length", kind = "derived",
      group = "derived", depends = { "dexterity", "strength", "encumbrance" },
      min = 1.5, max = 6, round = "none", hidden = true,
      formula = function(t)
          local over = t.encumbrance - t.strength
          if over < 0 then over = 0 end
          return 3.0 - (t.dexterity - 10) * 0.05 + over * 0.1
      end },

    -- A footing resource: a 0-20 pool resting at 10, spent by heavy actions and
    -- restored by recovering. **No `regen` block**, so it never ticks on its
    -- own — which is the whole shape, and `trait_d` already permits it.
    { id = "max_balance", label = "Max Balance", kind = "attribute",
      group = "vitals", default = 20, min = 1, hidden = true },
    { id = "balance", label = "Balance", kind = "gauge", group = "vitals",
      max = "max_balance", min = 0, round = "floor" },
})
DAEMON.trait.seal()

DAEMON.effect.define_all({
    -- A flat reduction, so a test can assert that an ability's damage went
    -- through the pipeline rather than around it.
    { id = "fixture_ward", label = "Fixture Ward", duration = 600, persist = false,
      hooks = { damage_taken = { phase = "reduce", fn = function(ev)
          ev.amount = math.max(0, ev.amount - 3)
      end } } },
    { id = "fixture_mark", label = "Fixture Mark", duration = 600, persist = false },
})

DAEMON.ability.define_all({
    -- Instant: cost, a short cooldown, declarative damage, one requirement.
    { id = "fixture_strike", name = "Fixture Strike", category = "technique",
      open = true, cost = { mp = 5 }, cooldown = 4, target = "creature",
      requires = { { kind = "trait", id = "level", min = 1 } },
      damage = { min = 7, max = 7, type = "physical" },
      messages = { self = "You strike $target.", result = "It takes $dealt." } },

    -- Over the durable threshold, to prove the tier is chosen by duration.
    { id = "fixture_slow", name = "Fixture Slow", category = "technique",
      open = true, cooldown = 90, target = "none",
      messages = { self = "Slowly." } },

    -- A cast you can be knocked out of, and then a channel that ticks.
    { id = "fixture_chant", name = "Fixture Chant", category = "technique",
      open = true, cost = { mp = 7 }, target = "none", cast_time = 3,
      interrupt = { on_damage = true, on_move = true },
      apply = { { effect = "fixture_mark", to = "self" } },
      messages = { begin = "You begin to chant.", self = "The chant finishes." } },

    { id = "fixture_channel", name = "Fixture Channel", category = "technique",
      open = true, target = "none", channel = { duration = 6, tick = 3 },
      heal = { min = 1, max = 1, to = "self" },
      messages = { begin = "You start channelling." } },

    -- Not `open`, and rank-backed: known only once something teaches it.
    { id = "fixture_taught", name = "Fixture Taught", category = "technique",
      rank_trait = "fixture_skill", min_rank = 2, target = "none",
      messages = { self = "You remember how." } },

    -- A heavy blow that costs footing, and a step that recovers it. Between
    -- them they are the reason `adjust` exists: `cost` cannot be positive.
    { id = "fixture_heavy", name = "Fixture Heavy", category = "technique",
      open = true, target = "none", adjust = { balance = -5 },
      messages = { self = "You commit." } },
    { id = "fixture_recover", name = "Fixture Recover", category = "technique",
      open = true, target = "none", adjust = { balance = 3, mp = -1 },
      messages = { self = "You set your feet." } },

    -- Routed through the resolution pipeline, so it can miss and be blunted by
    -- whatever covers where it lands.
    { id = "fixture_lance", name = "Fixture Lance", category = "technique",
      open = true, target = "creature",
      attack = { accuracy = 1.0, defenses = { dodge = 1.0 },
                 damage = { min = 9, max = 9, type = "physical" } },
      messages = { self = "You lance $target.", result = "It takes $dealt.",
                   miss = "$target avoids it." } },

    -- Grantable but tied to nothing, for the equipment/source path.
    { id = "fixture_granted", name = "Fixture Granted", category = "technique",
      target = "none", messages = { self = "Borrowed." } },
})

-- Items exist here so that a test of a *mudlib* command has a subject without
-- reaching for Thornhollow's. Between them they cover the shapes `objdump`'s
-- flags are about: an inherited method chain, and a component nesting three
-- deep so `-d` has something to run out of.
DAEMON.items.register_all({
    require('lib.item'):new{
        id = "fixture_stone", short = "a smooth stone",
        description = "A grey stone, worn smooth.", weight = 1, value = 1,
        tags = { "junk" },
    },

    -- Tagged `weapon`, so "item_d feeds the tag index" has something to find
    -- that is not this game's.
    require('components.weapon'){
        id = "fixture_blade", short = "a plain blade",
        description = "A blade with no history to speak of.",
        slot = "weapon", weight = 2, value = 10,
        damage = { min = 2, max = 4 }, speed = 1.0, weapon_type = "sword",
        tags = { "weapon" },
    },

    -- item -> armour -> resist is three levels, which is exactly where
    -- `objdump`'s default depth of 2 stops. `magic` is the leaf the depth test
    -- looks for.
    require('components.armor'){
        id = "fixture_cloak", short = "a warded cloak",
        description = "Grey wool with thread worked into the hem.",
        slot = "back", weight = 3, value = 100,
        defense = 1, armor_type = "cloth",
        resist = { magic = 6 },
        tags = { "armour" },
    },
})

DAEMON.world.register_area(DAEMON.room.load_area({
    _meta = { name = "fixture", title = "The Fixture", status = "live" },
    {
        id = "fixture.hall", short = "A Plain Hall",
        description = "A plain hall with a door at each end.",
        light = 3, tags = { "indoor" },
        exits = { north = "fixture.store", south = "fixture.cellar",
                  east = "fixture.yard" },
        items = { door = "A plain door. There are two of them." },
    },
    -- The one room whose `description` is a function. `description` is
    -- `lfun = true` in the room schema, so this is legal authored content —
    -- and it gives `objdump -r` something to resolve that is not this game's
    -- weather-keyed marsh.
    {
        id = "fixture.yard", short = "A Walled Yard",
        description = function(room, viewer)
            return "A yard walled in on three sides."
        end,
        light = 4, tags = { "outdoor" },
        exits = { west = "fixture.hall" },
    },
    {
        id = "fixture.store", short = "A Store Room",
        description = "Shelves, mostly empty.",
        light = 2, tags = { "indoor" },
        exits = { south = "fixture.hall" },
    },
    {
        id = "fixture.cellar", short = "A Dark Cellar",
        description = "Steps down into the dark.",
        light = 0, tags = { "indoor", "dark" },
        exits = { north = "fixture.hall" },
    },
}))

DAEMON.mobs.register_all({
    {
        id = "fixture_mouse", name = "mouse", short = "a small mouse",
        description = "A small brown mouse, entirely unbothered.",
        stats = { hp = 12, max_hp_flat = 12, strength = 4, dexterity = 10,
                  constitution = 6, intelligence = 2, wisdom = 4, level = 1 },
        damage = { min = 1, max = 2 },
        xp_award = 3,
        spawn_room = "fixture.store", count = 1, respawn_time = 60,
        tags = { "vermin" },
    },
})
DAEMON.mobs.populate()

function on_gmcp(session_id, package, data) end
"#;

/// A command in the fixture world's own `cmds/`, to prove the loader merges the
/// game and mudlib roots without needing this repository's `game/cmds/`.
const FIXTURE_COMMAND: &str = r#"
local M = {}
M.name = 'fixturecmd'
M.aliases = { 'fx' }
M.category = 'general'
M.summary = 'Prove the game layer is searched for commands.'
M.permission = nil
function M.execute(session_id, args_str, args)
    send(session_id, "\r\nthe fixture command ran\r\n")
end
return M
"#;

/// Run Lua *through the real command dispatcher*.
///
/// The probe boots (`boot_real_mudlib_with_probe`) replace `on_input`, so `eval`
/// works and the dispatcher never runs. The playing boots keep the real
/// dispatcher, so commands work and there is no way to run Lua. Anything that
/// tests the dispatcher *itself* — an interception, a permission gate, what a
/// verb does to a session — needs both at once.
///
/// So this is an ordinary command in the fixture game layer. It lives only in
/// the harness: adding an eval verb to `mudlib/` to make tests easier would be
/// putting a hole in the shipped game for the convenience of the test suite.
///
/// `SESSION` is the calling session id, so a test can say
/// `fixtureeval DAEMON.editor.open(SESSION, {...})`.
const FIXTURE_EVAL: &str = r#"
local M = {}
M.name = 'fixtureeval'
M.aliases = { 'fe' }
M.category = 'general'
M.summary = 'Run Lua through the real dispatcher. Harness only.'
M.permission = nil
function M.execute(session_id, args_str, args)
    SESSION = session_id
    local chunk, err = load(args_str, "=fixtureeval")
    if not chunk then
        send(session_id, "\r\nFXEVAL	COMPILE	" .. tostring(err) .. "\r\n")
        return
    end
    local ok, res = pcall(chunk)
    -- The marker is one line, so a multi-line value has its newlines escaped
    -- rather than truncated. A room description is the whole point of the
    -- editor, and every one of them is multi-line.
    local text = tostring(res):gsub("\\", "\\\\"):gsub("\r", ""):gsub("\n", "\\n")
    send(session_id, "\r\nFXEVAL	" .. (ok and "OK	" or "ERR	") .. text .. "\r\n")
end
return M
"#;

/// The probe dispatcher over a **copy of the real `game/`**.
///
/// This file *is* the copied tree's `init.lua`, overwritten after the copy — so
/// `real_init.lua` beside it is the game's own, and requiring it runs the real
/// game layer from the same tree the file jail is pointed at.
///
/// It used to prepend the real `game/` to `package.path` and require it from
/// there instead, which left `require` and `list_dir` disagreeing about where
/// the game was. See `boot_real_mudlib_with_probe_opts`.
const PROBE_GAME_LAYER: &str = r#"
local loaded, err = pcall(require, 'real_init')
if not loaded then
    log("error", "probe: the real game layer failed to load: " .. tostring(err))
end
{probe}"#;

/// Copy a directory tree. Used to stand the real `game/` up in a temp root.
fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(from) else { return };
    std::fs::create_dir_all(to).ok();
    for entry in entries.flatten() {
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if src.is_dir() {
            copy_dir(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).ok();
        }
    }
}

/// The probe dispatcher on its own, with no game content behind it.
///
/// Appended to whichever world the boot chose. Kept separate from
/// [`PROBE_GAME_LAYER`] so a test of *mudlib* mechanics can have the fixture
/// world instead of Thornhollow and still `eval`.
const PROBE_DISPATCHER: &str = r#"
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

    /// Run Lua through the **real command dispatcher**, on a playing session.
    ///
    /// Only for boots that carry the fixture game layer
    /// ([`RealVm::boot_with_fixture_world`]) — the probe boots have `eval`
    /// instead, and the two exist for opposite reasons: `eval` replaces the
    /// dispatcher, this one goes through it. Anything testing dispatch itself
    /// needs the second.
    ///
    /// Panics on a Lua error, like `Probe::unwrap`.
    ///
    /// When the chunk has side effects that write a *prompt* — opening the
    /// editor does — the reply ends at that prompt and the result marker arrives
    /// after it. There is nothing to return in that case, so the captured output
    /// is, which is what a caller checking a banner wants anyway. An error
    /// never reaches that path: raising produces no prompt, so its marker is
    /// always in the reply.
    pub fn lua(&mut self, src: &str) -> String {
        let out = self.command(&format!("fixtureeval {src}"));
        for line in out.lines() {
            if let Some(rest) = line.trim().strip_prefix("FXEVAL\t") {
                // `trim` has taken the trailing tab off an empty value, so
                // "OK" and "OK\t…" are both successes. Without this an empty
                // string — an unset global, a cleared buffer — reads as a
                // failure, which is a real answer being reported as an error.
                if rest == "OK" {
                    return String::new();
                }
                if let Some(v) = rest.strip_prefix("OK\t") {
                    // Newlines travelled escaped; the marker is one line.
                    return v.replace("\\n", "\n").replace("\\\\", "\\");
                }
                panic!("fixtureeval failed: {rest}\n(source: {src})");
            }
        }
        out
    }

    /// This VM's game root on disk, when it owns a temporary one.
    ///
    /// For anything that writes into the game layer and then wants to check
    /// *where* the file landed — which, with a two-root jail, is half the
    /// question. `None` for a boot pointed at the repository's own `game/`,
    /// where a test has no business writing anyway.
    pub fn game_root(&self) -> Option<&std::path::Path> {
        self._game.as_ref().map(|d| d.path())
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
    /// Mark this session as having negotiated GMCP, as the driver does.
    ///
    /// Production sets this in `publish_capabilities`, from the telnet
    /// negotiation loop — which the harness has no equivalent of, because it
    /// speaks to the engine directly rather than over a socket. Without it every
    /// `gmcp_d` sender returns at its first guard and a test asking what a client
    /// received would answer "nothing" for the wrong reason.
    pub fn negotiate_gmcp(&mut self) {
        let sid: oxigeon::core::SessionId = self.session_id.parse().expect("a session id");
        let mut handler = self.session_handler.write().unwrap();
        if let Some(session) = handler.get_mut(&sid) {
            session.capabilities.gmcp_supported = true;
        }
    }

    /// Deliver an inbound GMCP package, as a client would.
    ///
    /// Goes through the engine's `on_gmcp` dispatch rather than calling
    /// `DAEMON.gmcp.receive` directly, so the whole path a real client takes is
    /// exercised — including whatever the mudlib does in response.
    pub fn gmcp_in(&mut self, session_id: &str, package: &str, json: &str) {
        // A client that is sending GMCP has necessarily negotiated it.
        self.negotiate_gmcp();
        let data: serde_json::Value = serde_json::from_str(json).expect("valid GMCP JSON");
        self.engine().send(LuaCommand::OnGmcp {
            session_id: session_id.to_string(),
            package: package.to_string(),
            data,
        });

        // Wait for the reply itself rather than pumping an input line.
        //
        // Sending a probe line to make the Lua thread catch up is what this used
        // to do, and it raced two ways: on a real-dispatcher VM the line is an
        // unknown command, and the dispatch it triggers runs `prompt_d`, which
        // pushes GMCP of its own. Which messages had arrived by the time the
        // caller looked depended on scheduling, so the same test passed and
        // failed on alternate runs.
        let deadline = Instant::now() + Duration::from_secs(5);
        let before = self.gmcp_seen.len();
        while Instant::now() < deadline {
            while let Ok(msg) = self.output.try_recv() {
                self.keep_if_gmcp(msg);
            }
            if self.gmcp_seen.len() > before {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

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

    /// Boot against the mudlib and world this repository ships —
    /// `mudlib.default/` and `game.example/`.
    ///
    /// Every other constructor writes a throwaway mudlib that answers probes
    /// directly. This one runs an actual game — daemon load, command
    /// dispatch, rooms, the prompt — which is what a benchmark has to measure
    /// and what no synthetic workload can stand in for. The session comes back
    /// logged in and playing, via the real login flow.
    ///
    /// Not the `mudlib/` and `game/` the server actually loads: those are the
    /// creator's own, untracked and absent on a clean clone. See
    /// [`default_mudlib_root`]. Pointing at the shipped pair is also what makes
    /// a benchmark comparable between runs.
    ///
    /// The database and the log directory are still temporary; only the Lua is
    /// real.
    pub fn boot_real_mudlib(instruction_limit: u64) -> Self {
        let logs = TempDir::new().unwrap();
        let mut vm = Self::boot_inner_at(
            None,
            default_mudlib_root(),
            TestCtx {
                instruction_limit,
                game_path: Some(example_game_root()),
                start_room: Some(EXAMPLE_START_ROOM.to_string()),
                log_dir: Some(logs.path().to_path_buf()),
                ..Default::default()
            },
        );
        vm._logs = Some(logs);
        vm.login_as("benchuser", "a good long benchmark password");
        vm
    }

    /// Boot the shipped `mudlib.default/` against a **small self-contained
    /// world** written into a temp directory, instead of `game.example/`.
    ///
    /// The reason this exists: a game layer is content — "this game, and policy
    /// the driver has no view on" — and somebody who brings their own world
    /// should not inherit a broken test suite. Anything asserting mudlib
    /// *mechanics* should be able to say "given a world" without meaning "given
    /// Thornhollow". Tests that genuinely assert the shipped content live in
    /// `tests/demo_world/` and are deleted along with `game.example/`.
    ///
    /// The fixture is deliberately tiny — three rooms, one mob, one item, and
    /// the trait definitions a creature needs to exist. Traits are game-layer by
    /// design, so a world without them has no `hp` for anything to lose.
    pub fn boot_with_fixture_world(instruction_limit: u64) -> Self {
        let logs = TempDir::new().unwrap();
        let game = TempDir::new().unwrap();
        std::fs::write(game.path().join("init.lua"), FIXTURE_WORLD).unwrap();
        // A game-layer command, so "discovery spans both roots" stays a claim
        // about the loader rather than a claim about Thornhollow.
        std::fs::create_dir_all(game.path().join("cmds")).unwrap();
        std::fs::write(game.path().join("cmds/fixturecmd.lua"), FIXTURE_COMMAND).unwrap();
        std::fs::write(game.path().join("cmds/fixtureeval.lua"), FIXTURE_EVAL).unwrap();

        let mut vm = Self::boot_inner_at(
            None,
            default_mudlib_root(),
            TestCtx {
                instruction_limit,
                game_path: Some(game.path().to_path_buf()),
                start_room: Some(FIXTURE_START_ROOM.to_string()),
                log_dir: Some(logs.path().to_path_buf()),
                ..Default::default()
            },
        );
        vm._logs = Some(logs);
        vm._game = Some(game);
        vm.login_as("fixtureuser", "a good long fixture password");
        vm
    }

    /// Boot the shipped `mudlib.default/` behind a probe `on_input`.
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
    /// Two things it deliberately does not do: there is **no start room and
    /// nobody logs in**, and the real command dispatcher is not exercised —
    /// `boot_real_mudlib` already covers that. Use this to ask what mudlib code
    /// does; use that one to ask what a player experiences.
    ///
    /// The `game.example/` content *is* loaded: `PROBE_GAME_LAYER` puts it on
    /// `package.path` and `require`s `init`, so areas, mobs, items, traits,
    /// effects and quests are all registered. (This comment used to claim
    /// otherwise, which is why several tests assert on shipped content through
    /// a boot that supposedly had none.) The `require` is inside a `pcall` that
    /// only logs, so a missing world does not fail the boot — it fails the
    /// assertions that name content, which is the right place for it to hurt.
    pub fn boot_real_mudlib_with_probe() -> Self {
        Self::boot_real_mudlib_with_probe_opts(TestCtx::default())
    }

    /// As above, with control over the config — for a test that needs one of
    /// the periodic subsystems actually registered. `game_path` and `log_dir`
    /// are overwritten; everything else is honoured.
    pub fn boot_real_mudlib_with_probe_opts(mut opts: TestCtx) -> Self {
        let logs = TempDir::new().unwrap();
        let game = TempDir::new().unwrap();

        // `game.example/` is **copied** into the temp root rather than being
        // reached through `package.path`.
        //
        // It used to be the latter: the temp directory held one `init.lua` that
        // prepended the real game to `package.path` and required it. That made
        // `require('areas.thornhollow.rooms')` work and left the *file jail*
        // pointing somewhere else entirely — so `list_dir("areas")` saw nothing,
        // and once areas were discovered rather than listed, this harness booted
        // a world with no areas in it while every other path still looked right.
        //
        // Copying is ~20 small files and makes both halves agree: what `require`
        // resolves and what `list_dir` reports are the same tree. The copy is
        // also what makes it safe for a test to write into the game root.
        copy_dir(&example_game_root(), game.path());
        // The game's own entry point, moved aside so the probe can be the one
        // the engine loads and still hand off to it.
        std::fs::rename(game.path().join("init.lua"), game.path().join("real_init.lua")).unwrap();
        std::fs::write(
            game.path().join("init.lua"),
            PROBE_GAME_LAYER.replace("{probe}", PROBE_DISPATCHER),
        )
        .unwrap();

        opts.game_path = Some(game.path().to_path_buf());
        opts.log_dir = Some(logs.path().to_path_buf());

        let mut vm = Self::boot_inner_at(None, default_mudlib_root(), opts);
        vm._logs = Some(logs);
        vm._game = Some(game);
        assert_eq!(vm.eval("return 'ready'").unwrap(), "ready");
        vm
    }

    /// The probe dispatcher over the **fixture world** rather than
    /// `game.example/`.
    ///
    /// [`RealVm::boot_real_mudlib_with_probe`] gives you `eval` against a fully
    /// wired `DAEMON` table — and Thornhollow with it, so any test that seeds a
    /// creature through it is quietly asserting that *the demo* defines the
    /// traits. Delete `game.example/` and it fails, which is exactly what
    /// `docs/src/testing.md` says a mudlib test must not do.
    ///
    /// This is the same probe over [`FIXTURE_WORLD`]: the smallest world in
    /// which the mudlib still works. Nobody logs in; use it to ask what mudlib
    /// code does.
    pub fn boot_fixture_with_probe() -> Self {
        Self::boot_fixture_with_probe_opts(TestCtx::default())
    }

    /// As above, with control over the config. `game_path` and `log_dir` are
    /// overwritten; everything else is honoured.
    pub fn boot_fixture_with_probe_opts(mut opts: TestCtx) -> Self {
        let logs = TempDir::new().unwrap();
        let game = TempDir::new().unwrap();
        std::fs::write(
            game.path().join("init.lua"),
            format!("{FIXTURE_WORLD}
{PROBE_DISPATCHER}"),
        )
        .unwrap();

        opts.game_path = Some(game.path().to_path_buf());
        opts.log_dir = Some(logs.path().to_path_buf());

        let mut vm = Self::boot_inner_at(None, default_mudlib_root(), opts);
        vm._logs = Some(logs);
        vm._game = Some(game);
        assert_eq!(vm.eval("return 'ready'").unwrap(), "ready");
        vm
    }

    /// Boot with **both jail roots writable**, for anything that tests the file
    /// efuns themselves.
    ///
    /// Every other constructor points the mudlib root at the repository's own
    /// `mudlib/`, which makes a write test either impossible or a test that
    /// litters the tree it is testing. Both roots here are temp directories, so
    /// a refusal and a success are equally safe to assert.
    ///
    /// Returns the VM plus the two real paths, so a test can check *where* a
    /// write landed rather than only that it succeeded — which is the whole
    /// question a two-root jail raises.
    pub fn boot_two_roots(
        permissions: PermissionConfig,
    ) -> (Self, std::path::PathBuf, std::path::PathBuf) {
        let mudlib = TempDir::new().unwrap();
        write_probe_mudlib(mudlib.path());
        let game = TempDir::new().unwrap();
        // The engine requires a game layer to load; an empty one is enough,
        // because these tests ask about the efuns rather than about content.
        std::fs::write(game.path().join("init.lua"), "-- two-root jail fixture\n").unwrap();

        let mudlib_path = mudlib.path().to_path_buf();
        let game_path = game.path().to_path_buf();
        let logs = TempDir::new().unwrap();

        let mut vm = Self::boot_inner_at(
            Some(mudlib),
            mudlib_path.clone(),
            TestCtx {
                permissions,
                game_path: Some(game_path.clone()),
                log_dir: Some(logs.path().to_path_buf()),
                max_connections: 8,
                max_characters_per_account: 1,
                ..Default::default()
            },
        );
        vm._logs = Some(logs);
        vm._game = Some(game);
        assert_eq!(vm.eval("return 'ready'").unwrap(), "ready");
        (vm, mudlib_path, game_path)
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
        let ctx = efun_context(session_handler.clone(), mudlib_path.clone(), pool.clone(), opts);
        let engine = ScriptEngine::start(mudlib_path, ctx, cmd_tx, cmd_rx).unwrap();

        Self {
            engine: Some(engine),
            session_id,
            output,
            pending_auth: Default::default(),
            pending_probe: Default::default(),
            pending_compute: Default::default(),
            gmcp_seen: Vec::new(),
            session_handler,
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

    /// Throw away anything already in the channel — except GMCP, which is kept.
    ///
    /// GMCP used to be discarded here and at `Ok(_) => continue` below, which is
    /// why no test ever noticed that a playing session is never sent
    /// `Char.Vitals`, `Char.Status` or `Char.Effects`: the harness threw away
    /// the only evidence. `tests/gmcp_outbound.rs` asks what a client receives,
    /// and it can only ask if the answer is kept.
    fn discard_pending(&mut self) {
        while let Ok(msg) = self.output.try_recv() {
            self.keep_if_gmcp(msg);
        }
    }

    fn keep_if_gmcp(&mut self, msg: SessionOutput) {
        if let SessionOutput::Gmcp { package, data } = msg {
            self.gmcp_seen.push((package, data));
        }
    }

    /// Every GMCP package this session has been sent, oldest first, and clear.
    pub fn take_gmcp(&mut self) -> Vec<(String, serde_json::Value)> {
        while let Ok(msg) = self.output.try_recv() {
            self.keep_if_gmcp(msg);
        }
        std::mem::take(&mut self.gmcp_seen)
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
                Ok(other) => {
                    self.keep_if_gmcp(other);
                    continue;
                }
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

    /// Send probe source without waiting for its reply.
    ///
    /// [`RealVm::eval`] waits, which is exactly wrong for a dispatch that is
    /// about to stop at a breakpoint and may never answer.
    pub fn send_eval(&self, src: &str) {
        assert!(!src.contains('\n'), "probe source must be one line");
        self.engine.as_ref().unwrap().send(LuaCommand::OnInput {
            session_id: self.session_id.clone(),
            text: src.to_string(),
        });
    }

    /// The next probe reply, or `None` if none arrives inside `window`.
    ///
    /// The `None` is the assertion in a test of a frozen VM, so this must not
    /// give up early the way `drain_for` does on a quiet channel.
    pub fn probe_within(&mut self, window: Duration) -> Option<Probe> {
        if let Some(queued) = self.pending_probe.pop_front() {
            return Some(queued);
        }
        let deadline = Instant::now() + window;
        while Instant::now() < deadline {
            match self.output.try_recv() {
                Ok(SessionOutput::Text(t)) => match parse_line(&t) {
                    Line::Probe(p) => return Some(p),
                    Line::Auth(a) => self.pending_auth.push_back(a),
                    Line::Compute(c) => self.pending_compute.push_back(c),
                },
                Ok(_) => {}
                Err(mpsc::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
        None
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
    /// The debugger's shared state. `None` builds the default one, exactly as
    /// a server without `[servers.debug]` does.
    ///
    /// A test that wants to *drive* a stop passes its own clone in here and
    /// keeps the other: the atomics are the same ones the hook reads, so
    /// setting `pause_req` from the test thread is the same request a DAP
    /// client's `pause` makes, without a TCP client in the way.
    pub debug_state: Option<SharedDebugState>,
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
            debug_state: None,
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
    //
    // A worker is a child process, and under `cargo test` there is no release
    // layout to find one beside. Point every compute test at the binary the
    // harness builds — here rather than in one constructor, so a test that turns
    // compute on through any boot path gets it.
    let compute = opts.cmd_tx.as_ref().and_then(|tx| {
        let mut cfg = opts.compute.clone();
        if cfg.enabled && cfg.worker_path.is_none() {
            cfg.worker_path = Some(compute_worker_binary().to_string_lossy().into_owned());
        }
        ComputeBridge::start(
            cfg,
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
        debug_state: opts.debug_state.clone().unwrap_or_else(|| {
            DebugState::from_config(&DebugServerConfig::default(), opts.instruction_limit)
        }),
        auth_worker,
        compute,
        document_store,
    }
}
