# Testing

Oxigeon uses **Rust integration tests** to verify the driver. All tests live in
the `tests/` directory and run via `cargo test`.

## The one rule about layout

**Every test here is about Rust.**

It used to be three buckets, because this repository shipped a Lua layer for a
game to start from — `mudlib.default/` and `game.example/` — with suites
asserting their content. It ships neither now. Nothing developed them, the one
game built on this engine forked the mudlib long ago and never took a change
back, and a shipped layer nobody ships is a second copy of every decision,
drifting quietly from the first.

| | |
|---|---|
| `tests/driver/` | the engine: the stores, the sandbox, the debugger, the file jail, permissions, telnet/GMCP, websockets, compute. |

So the admission question is no longer which bucket. It is:

> **Would this test survive somebody throwing the whole Lua layer away and
> writing their own?** Yes -> it belongs here. No -> it belongs in that game's
> own repository, against that game's own trees.

That resolves the genuinely ambiguous ones without arguing. `staff` is a driver
test although it drives a mudlib, because the RBAC efuns are Rust and the mudlib
is only the vehicle. Something like the old `fs_shell` — `ls` and `cd` as mudlib
commands — has no home here at all now, because a new mudlib would write its
own. That is not a gap; it is the line being drawn where it belongs.

`tests/compute_wedge.rs` is deliberately its own binary, and the reason is in
its own header: every test in it spins a core in `while true do end` for the
whole of its deadline, so as a neighbour it starves whatever it shares a binary
with. Folded into `tests/driver/` it made the pool-recovery test fail
intermittently -- a job that could not get scheduled inside a *forty-second*
deadline, with nothing wrong with the pool. Merge it into another binary and
that comes back.

`boot_with_fixture_world` writes a small self-contained game layer into a temp
directory — three rooms, one creature, one item, a trait set, a role, and one
game-layer command — and boots the fixture mudlib against it. Traits and roles
are in there because both are game-layer by design: a world with no trait
definitions has no `hp` for anything to lose.

`boot_fixture_with_probe` is the same world behind the probe dispatcher, for a
test that needs `eval` against a wired `DAEMON` table rather than a player's
view. `boot_real_mudlib_with_probe` is the heavier option, copying the whole
fixture world in; prefer the small one when the size of the world is not the
point.

## The Lua the suite boots is a vehicle, not a subject

| | tracked here | what it is |
|---|---|---|
| `tests/fixture/mudlib` | yes | a working Lua system layer, for a driver test to boot |
| `tests/fixture/game` | yes | a world to boot it against |
| `mudlib/` | **no** | the creator's mudlib, from their own repo |
| `game/` | **no** | the creator's game, from their own repo |

`config/server.toml` points the *server* at `./mudlib` and `./game`, because that
is what a creator plays. Every test harness points at the fixture.

They are different questions. `mudlib/` and `game/` are gitignored, absent on a
clean clone, and free to have diverged arbitrarily — a suite that booted them
would assert an unpublished fork on the machine that has one and fail everywhere
else. So `boot_real_mudlib`, `boot_with_fixture_world` and both probe variants
resolve their roots through `fixture_mudlib_root()` and `fixture_game_root()` in
`testkit`, and no test names `mudlib/` or `game/` at all.

The consequence worth stating outright: **a failure here is never a complaint
about the fixture's content.** If a test can be made to pass by editing Lua under
`tests/fixture/`, it was asking the wrong question — it is asserting something
about a Lua layer, and no Lua layer here is anybody's product. Rewrite it against
the Rust, or delete it and let the game that cares assert it.

This also means `boot_real_mudlib` does not read `start_room` from
`config/server.toml`. That key describes the creator's world and would send the
harness to a room the fixture does not contain, so the fixture's entrance is a
constant beside the small world's — `EXAMPLE_START_ROOM`, next to
`FIXTURE_START_ROOM`. A world's start room is a property of that world.

## Quick Start

```bash
# Run the entire test suite
cargo test

# Run the driver suite
cargo test --test driver

# Run one file's worth, by module name
cargo test --test driver sandbox

# Run a single test by name
cargo test --test driver a_websocket_client_reaches_the_login_banner
```

The `--test` argument is the *binary*, not the file. Everything is a module of
`tests/driver/main.rs`, so `cargo test --test sandbox` does not resolve — use
`cargo test --test driver sandbox`, which filters by module path.

All tests should pass before committing — 309 in the driver suite at the time of
writing, green on the default Lua 5.5 build.

> [!WARNING]
> **`cargo test --no-default-features --features luajit` does not currently
> build.** This page used to claim the suite was green on it. It is not, and has
> not been for as long as the dev-dependency has read
> `oxigeon = { path = ".", features = ["testkit"] }` — that pulls the crate's
> *default* features, so `lua55` arrives alongside `luajit` and `mlua-sys`
> refuses both:
>
> ```
> error: You can enable only one of the features: lua55, lua54, …, luajit, …
> ```
>
> The library itself is fine on LuaJIT — `cargo build --lib
> --no-default-features --features luajit` succeeds — so this is a Cargo feature
> plumbing problem, not a code one. The fix is `default-features = false` on the
> dev-dependency plus a way to select the runtime for it, and it is worth doing:
> a build claimed to be supported that nobody can run the suite against is a
> build nobody is testing.
>
> Recorded here rather than quietly dropped, for the same reason the
> `postgresql` note in [Configuration](./configuration.md) is.

`cargo test` does not build `oxigeon-compute`. It is a separate workspace member
that links LuaJIT unconditionally, and cargo unifies features across a single
invocation, so making it a default member would break every `lua55` build. The
harness builds it on demand, into its own `target/compute-worker/` — a shared
target directory would contend for the build lock the outer `cargo test` holds,
which looks like a test run that hangs with no output.

---

## Writing a new test

There used to be a second harness here — a lightweight LuaJIT VM with stubbed
efuns (`lua_unit.rs`), for asserting against Lua modules in isolation — and a
long section on how to write for it. Both went with the Lua layer they tested.
A stub-backed VM is the right tool for testing a *mudlib*, and testing a mudlib
is no longer this repository's job.

What is left is the real thing, and it is the only thing:

```rust
let mut vm = RealVm::boot_real_mudlib_with_probe();
assert_eq!(vm.eval("return 2 + 2").unwrap(), "4");
```

A real `ScriptEngine`, the real efuns, the real sandbox, a temporary database,
and `tests/fixture/` loaded on top as a vehicle. `eval` runs Lua inside it;
`command` sends a line as a logged-in player and returns what the game said.
See `src/testkit.rs` for the boot variants — the small fixture world, the whole
fixture pair, and `boot_roots_*` for a caller supplying its own trees.

Two rules carry over from the harness that is gone, and matter more here:

- **Stub the boundary, never the subject.** Seeding state, pinning a clock and
  making a write refuse are all fair. Reimplementing what you are asking about
  is how a test comes to agree with the bug.
- **A test that a Lua module behaves is not a driver test.** If it passes or
  fails on what `tests/fixture/` says, it is asserting content, and it belongs
  in the repository whose content it is.

## What's Currently Tested

**309 tests** in `tests/driver`, plus `tests/compute_wedge.rs` on its own. Every
one of them is a claim about Rust; where a Lua layer appears it is
`tests/fixture/` being driven, never the thing under test.

### The security and boundary suites

| Module | Covers |
|---|---|
| `sandbox.rs`, `sandbox_reality_check.rs` | `io`, `os.execute`, `debug`, `jit`, bytecode and path traversal, refused **through the engine's own VM** |
| `list_dir_jail.rs` | the second, unjailed `list_dir` that overwrote the jailed one |
| `file_jail_two_roots.rs` | which root a path resolves against, and which one a write defaults to |
| `instruction_limit.rs` | the budget is armed and enforced, not merely parsed |
| `permission_config.rs`, `permissions.rs` | RBAC storage and the session cache |
| `permission_refresh.rs` | a role change reaching a player who is already online |
| `staff.rs` | roles declared in a file, granted in-game, and a gated command actually gated |

### The engine

| Module | Covers |
|---|---|
| `account_store.rs`, `character_store.rs` | persistence |
| `auth_off_thread.rs` | Argon2 off the game thread, and the lockout |
| `clean_shutdown.rs` | `on_shutdown` runs and is waited for |
| `command_dispatch.rs` | a line of input reaching Lua and the reply coming back |
| `compute_bridge.rs`, `compute_wedge.rs` | job delivery, marshalling refusals, a wedged worker |
| `document_store.rs`, `document_efuns.rs` | the store and its twelve efuns |
| `json_bridge.rs` | what survives a round trip between a Lua table and JSON |
| `hot_reload.rs` | `reload`, `on_load`/`on_unload`, DAEMON rebinding |
| `timer_identity.rs` | a timer surviving — or not surviving — a reload of what registered it |
| `observability.rs`, `game_logger.rs` | the journal and the audit trail |
| `output_backpressure.rs` | what happens when a client stops reading |
| `telnet_tls.rs`, `websocket_relay.rs` | framing, negotiation, origins, certificates, and the login flow over both |
| `telnet_mxp.rs` | option 91: the handshake, the injection it exists to prevent, and that game text is byte-identical with it on |
| `dap_attach.rs`, `debug_*.rs`, `yield_pause.rs` | the debug adapter, breakpoints, and stopping a dispatch mid-flight |

---

## Tips

- **Each test gets its own VM.** State doesn't leak between tests, and they run in parallel.
- **Use `r#"..."#` for Lua strings** in Rust to avoid escaping quotes.
- **Check `nil` with `== nil`** in Lua, not with Rust's `Option` — `eval_bool(lua, "return val == nil")` is the cleanest pattern.
- **Errors in `.exec().unwrap()`** will print the Lua stack trace, which makes debugging straightforward.
- **Add a stub if a new efun is needed.** If you add a new efun that modules call at load time, add a minimal stub in `make_test_lua()` so tests don't break.
- **Test the real modules.** Don't copy Lua code into your test — `require()` the actual file so your tests catch regressions in the real code.
