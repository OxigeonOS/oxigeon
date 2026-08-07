# Performance & the JIT Trade-off

Oxigeon runs Lua on LuaJIT. LuaJIT has a tracing compiler, and one Oxigeon
feature is mutually exclusive with it: `limits.lua_instruction_limit`.

This page explains why, what it costs, and how to re-measure it yourself —
because the numbers below were wrong once already, and the way to stop that
happening again is to make them reproducible by one command.

## Why the limit and the compiler cannot both be on

The instruction limit works by installing a LuaJIT debug hook that charges each
dispatch against a budget and raises when it runs out. **LuaJIT dispatches no
debug hooks from inside a compiled trace.** Once a loop is hot enough to
compile — 56 iterations — it stops calling the hook, and the budget stops
being charged.

That is not a matter of picking a better hook mask. Measured on this build, a
one-line `while true do s = s + 1 end` with the compiler on delivers **no hook
events at all**:

| Trigger mask | Interrupts a runaway one-line loop? |
|---|---|
| `every_nth_instruction(1000)` | no |
| `every_nth_instruction(10)` | no |
| `every_line` | no |
| `every_line` + `every_nth_instruction` | no |
| `on_calls` + `on_returns` + `every_nth_instruction` | no |
| any of the above, after `jit.off()` | **yes** |

So `ScriptEngine::start` calls `jit.off()` whenever a limit is configured, and
`apply_sandbox` removes the `jit` table so game code cannot turn the compiler
back on and silently disarm the budget.

## What it costs

Run the benchmark yourself:

```text
scripts/bench.ps1        # Windows
scripts/bench.sh         # everything else
```

It boots the **real** `mudlib/` and `game/`, logs a session in through the real
login flow, and dispatches real commands. Three configurations are measured so
the two effects can be told apart:

| Configuration | JIT | count hook |
|---|---|---|
| `jit-on` | on | off |
| `jit-off` | off | off |
| `jit-off+budget` | off | on |

`jit-on` → `jit-off` is what the compiler is worth. `jit-off` →
`jit-off+budget` is what the hook itself costs. An earlier version of these
docs quoted a single figure that conflated the two, measured on a synthetic
loop rather than the game — see the note at the end.

### Results

Release build, criterion, 100 samples per configuration, one machine. Times are
per command dispatch and include the round trip to the Lua thread, because a
player pays that too.

| Workload | JIT on | JIT off | JIT off + budget | Compiler worth | Hook costs | **Total** |
|---|---|---|---|---|---|---|
| `look` | 74.9 µs | 74.7 µs | 77.1 µs | 1.00× | 1.03× | **1.03×** |
| `who` | 186.1 µs | 188.4 µs | 190.0 µs | 1.01× | 1.01× | **1.02×** |
| `mudstatus` | 107.9 µs | 114.2 µs | 115.4 µs | 1.06× | 1.01× | **1.07×** |
| `numeric` *(control)* | 911.5 µs | 1912.5 µs | 2363.9 µs | 2.10× | 1.24× | 2.59× |

Harness floor — an empty round trip to the Lua thread and back — is 7.1 µs.

**The compiler is worth 2.10× on a tight arithmetic loop and essentially
nothing on real command dispatch.**

## LuaJIT against Lua 5.5

Which runtime the game thread uses is a build-time choice (`--features lua55`;
see `Cargo.toml`). Both columns below have the instruction budget on, because
that is what `config/server.toml` ships — and it is the honest comparison, since
enabling the budget is what turns the LuaJIT compiler off in the first place.

Debug build, criterion, 20 samples, one machine, so read these as a shape rather
than as figures to three digits.

| Workload | LuaJIT + budget | Lua 5.5 + budget | |
|---|---|---|---|
| `look` | 105.5 µs | **103.4 µs** | 5.5 is 2% faster |
| `who` | 127.3 µs | **122.8 µs** | 5.5 is 4% faster |
| `mudstatus` | **163.9 µs** | 179.1 µs | 5.5 is 9% slower |

**It is a wash.** That reads oddly until you remember the game thread has never
been getting the compiler: `lua_instruction_limit` disables it at boot, so both
columns are interpreter against interpreter, and PUC Lua's is competitive with
LuaJIT's when neither is tracing. `mudstatus` is the outlier because it is the
one command that does real arithmetic — heap fractions, uptime division — which
is exactly where LuaJIT's interpreter is stronger.

**Lua 5.5 is therefore the default**, and not for performance — on the numbers
above that would be a coin toss. It is because **a breakpoint stops one player
instead of the server**. mlua's `VmState::Yield` is Lua 5.3+ only, so on LuaJIT a
stop is implemented by blocking the Lua thread — the only thread — while on 5.5
the hook yields and the engine parks that one command as a suspended coroutine.
See [What a breakpoint costs](./debugging.md#-what-a-breakpoint-costs).

The one real objection to 5.5 used to be that it took the *compute* pool with
it, and the compiler is worth 2.10× on exactly the arithmetic-heavy work that
belongs there. That is why compute now runs as its own process: `oxigeon-compute`
links LuaJIT unconditionally, whatever the server was built with, so the 2.10×
applies where it was worth having and the game thread gets the debugger it
wanted. See [Compute](./compute.md).

That is not surprising once you look at what LuaJIT needs. Its trace recorder
fires after 56 iterations of a loop, and nothing inside a single MUD command
loops 56 times — the dispatcher runs a handful of pattern matches, a few table
lookups, and 5–20 efun calls into Rust. Worse for tracing, `string.find`,
`match`, `gmatch` and `gsub` are not implemented in the recorder and *abort*
traces outright, and the dispatcher's hot path is `gsub`, `gsub`, `match`,
`gmatch`. The one place the compiler pays is exactly the place a MUD does not
usually go: long arithmetic loops.

## Should you turn the limit on?

**Yes, and it is on by default.**

Enforcing costs 2–7% on real commands. What it buys is that a `while true do
end` in a room file raises a Lua error instead of wedging the game thread until
somebody kills the process. At that price the trade is not close.

This is a reversal. The limit shipped disabled by default on the strength of a
"1.28×" figure that came from a synthetic loop and conflated two separate
costs. Measured properly, through the game, the compiler turns out to be worth
almost nothing to a MUD — so the thing that was supposedly being protected was
never really there.

Turn it off (`lua_instruction_limit = 0`) if your game genuinely does heavy
arithmetic in Lua on the game thread. If it does, consider the
[compute bridge](./compute.md) instead: it moves that work to a worker process
where it can keep the compiler *and* stop blocking every player.

> [!WARNING]
> **Known gap.** The limit stops accidents, not sabotage. `pcall` catches the
> error the budget raises and Lua 5.1 has no uncatchable error, so
>
> ```lua
> while true do pcall(function() while true do end end) end
> ```
>
> still wedges the game thread: every raise lands inside the inner loop at a
> fixed offset, so the outer loop is never reached. Treat the ability to write
> Lua on this server as a trusted privilege regardless of this setting.

## Re-measuring

The benchmark lives in `benches/dispatch.rs` and is built on
`tests/common/mod.rs`'s `RealVm::boot_real_mudlib`, the same harness the
integration tests use. `tests/demo_world/real_mudlib_harness.rs` proves that harness
actually boots the game, so a benchmark cannot quietly measure a half-started
server.

Criterion saves baselines, so a change can be compared against a known-good
run:

```text
scripts/bench.ps1 -- --save-baseline main
# ...make a change...
scripts/bench.ps1 -- --baseline main
```

The full HTML report lands in `target/criterion/report/index.html`.

### Reading the results honestly

- **`floor/round-trip`** is what the harness itself costs — sending a command
  to the Lua thread and reading the reply back. Every `dispatch/*` figure
  includes it. So does a real player's, so it is not subtracted.
- **`numeric`** is the control. A tight arithmetic loop is the shape LuaJIT is
  best at, so it must show a large `jit-on` advantage. If it ever stops doing
  so, the `OXIGEON_JIT` toggle has broken and none of the other numbers mean
  anything.
- The first dispatch of any command is far slower than the rest: the mudlib
  lazy-loads and `require`s every command module on first use. The benchmark
  runs twenty dispatches before measuring.

> [!NOTE]
> If the LuaJIT build fails with `'minilua' is not recognized`, your shell has
> `NoDefaultCurrentDirectoryInExePath` set. LuaJIT's MSVC build invokes the host
> tools it just built by bare name. The wrapper scripts prepend `.` to `PATH`
> to work around it; use them rather than `cargo bench` directly.

## A note on the earlier numbers

The first version of this page quoted 1.28× for "string/table work shaped like
a MUD command" and 2.61× for a tight numeric loop. Those came from a throwaway
benchmark that was deleted immediately afterwards, ran in a debug profile with
five samples, measured a synthetic loop rather than the mudlib, and — worst —
compared *(JIT off + hook)* against *(JIT on)*, so the two costs could not be
separated at all.

They are superseded by the table above. The `OXIGEON_JIT` environment variable
exists solely so the two can be measured apart; it is read once at startup, has
no Lua-side equivalent, and is not a supported way to run a server.
