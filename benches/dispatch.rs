//! What the LuaJIT compiler is actually worth to this MUD.
//!
//! `limits.lua_instruction_limit` cannot be enforced with the JIT on — LuaJIT
//! dispatches no debug hooks from inside a compiled trace — so turning the
//! limit on turns the compiler off. That trade is only defensible with real
//! numbers, and the numbers that were in the docs first were not: they came
//! from a synthetic loop, in a debug profile, and they moved two variables at
//! once (losing the JIT *and* gaining a count hook), so they measured neither.
//!
//! This measures the real mudlib — daemon dispatch, command lookup, room
//! rendering, the prompt — and separates the two variables:
//!
//! | Configuration        | JIT | count hook | how                                    |
//! |----------------------|-----|-----------|-----------------------------------------|
//! | `jit-on`             | on  | off       | the shipped default                     |
//! | `jit-off`            | off | off       | `OXIGEON_JIT=off`, benchmark-only       |
//! | `jit-off+budget`     | off | on        | `lua_instruction_limit = 1_000_000`     |
//!
//! `jit-on` minus `jit-off` is the compiler's worth. `jit-off` minus
//! `jit-off+budget` is what the hook itself costs. Only their sum was ever
//! measured before.
//!
//! The `numeric` group is the control. It is a tight arithmetic loop — the
//! shape LuaJIT is best at — so it must show a large delta. If it does not,
//! the toggle is broken and every other number here is noise.
//!
//! Run it:
//!
//! ```text
//!   scripts/bench.ps1              # Windows
//!   scripts/bench.sh               # everything else
//!   cargo bench --bench dispatch   # if your shell is already sane
//! ```
//!
//! Use the wrapper scripts unless you know your environment: LuaJIT's MSVC
//! build invokes the host tools it just built by bare name, which fails if
//! `NoDefaultCurrentDirectoryInExePath` is set. The scripts prepend `.` to
//! PATH so it cannot bite.

use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

#[path = "../tests/common/mod.rs"]
mod common;

use common::RealVm;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Jit {
    On,
    Off,
}

/// Every configuration worth measuring, in report order.
///
/// On PUC Lua there is no compiler, so `jit-on` and `jit-off` would be the same
/// run twice under two names. What is left worth measuring is the interpreter
/// with and without the instruction budget — and the comparison that actually
/// decides the default runtime is between the two *builds*, not within one.
#[cfg(feature = "luajit")]
const CONFIGS: &[(&str, Jit, u64)] = &[
    ("jit-on", Jit::On, 0),
    ("jit-off", Jit::Off, 0),
    ("jit-off+budget", Jit::Off, 1_000_000),
];

#[cfg(not(feature = "luajit"))]
const CONFIGS: &[(&str, Jit, u64)] = &[
    ("lua55", Jit::Off, 0),
    ("lua55+budget", Jit::Off, 1_000_000),
];

/// Boot a VM with the compiler in a known state.
///
/// `OXIGEON_JIT` is read once, on the Lua thread, as the VM is built. Both
/// constructors wait for their VM to answer before returning, so by the time
/// this function returns the variable has definitely been read.
///
/// It is deliberately *not* cleared afterwards. It used to be, and that was a
/// race: `ScriptEngine::start` returns the moment the thread is spawned, so
/// clearing on return removed the variable before the Lua thread got to it and
/// every "JIT off" measurement silently ran with the JIT on. Leaving it set is
/// safe because every call sets or clears it before booting, so the value is
/// always correct for the boot that follows.
fn with_jit<T>(jit: Jit, boot: impl FnOnce() -> T) -> T {
    if jit == Jit::Off {
        std::env::set_var("OXIGEON_JIT", "off");
    } else {
        std::env::remove_var("OXIGEON_JIT");
    }
    boot()
}

/// Prove the toggle works before measuring anything with it.
///
/// A benchmark whose control silently stops controlling is worse than no
/// benchmark, because its numbers still look authoritative. This times a tight
/// arithmetic loop — the shape LuaJIT is best at, measured at 1.86x on a bare
/// VM — under both settings and refuses to continue if they come out alike.
#[cfg(not(feature = "luajit"))]
fn assert_the_jit_toggle_works() {
    // Nothing to assert: this build has no compiler, so there is no toggle that
    // could silently stop working and no compiler claim to be wrong about.
    eprintln!("JIT toggle check: skipped — this build runs PUC Lua");
}

#[cfg(feature = "luajit")]
fn assert_the_jit_toggle_works() {
    const DEFINE: &str = "_probe = function() local s = 0 \
                          for i = 1, 200000 do s = s + i % 7 end return s end return 'ok'";

    fn time_under(jit: Jit) -> Duration {
        let mut vm = with_jit(jit, || RealVm::boot_with_instruction_limit(0));
        assert_eq!(vm.eval(DEFINE).unwrap(), "ok");
        for _ in 0..20 {
            vm.eval("return _probe()");
        }
        let start = std::time::Instant::now();
        for _ in 0..30 {
            vm.eval("return _probe()");
        }
        start.elapsed() / 30
    }

    let on = time_under(Jit::On);
    let off = time_under(Jit::Off);
    let ratio = off.as_secs_f64() / on.as_secs_f64();
    eprintln!("JIT toggle check: on {on:?}, off {off:?} ({ratio:.2}x)");

    assert!(
        ratio > 1.3,
        "OXIGEON_JIT=off made no measurable difference ({ratio:.2}x): jit-on {on:?} vs \
         jit-off {off:?}. The toggle is broken, so every number this benchmark would \
         print about the compiler is meaningless. Refusing to continue."
    );
}

/// One dispatch of a real command, through the real mudlib.
///
/// This is the number that decides whether the instruction limit should be on
/// by default. It includes the channel round trip to the Lua thread, which a
/// player pays too.
fn real_commands(c: &mut Criterion) {
    // `mudstatus` is admin-only; the benchmark account is the first one
    // created, so it is auto-promoted and can run it.
    for verb in ["look", "who", "mudstatus"] {
        let mut group = c.benchmark_group(format!("dispatch/{verb}"));
        group.measurement_time(Duration::from_secs(8));

        for (name, jit, limit) in CONFIGS {
            let mut vm = with_jit(*jit, || RealVm::boot_real_mudlib(*limit));

            // The first dispatch pays for `load_all_commands()`, which
            // `require`s every command module in the mudlib. Criterion's
            // warmup would absorb it, but paying it explicitly means the
            // warmup measures steady state rather than that one-off.
            for _ in 0..20 {
                vm.command(verb);
            }

            group.bench_with_input(BenchmarkId::from_parameter(name), verb, |b, verb| {
                b.iter(|| std::hint::black_box(vm.command(verb)))
            });
        }
        group.finish();
    }
}

/// The control. A tight numeric loop is what LuaJIT compiles best, so this
/// must show a large `jit-on` advantage. If it ever stops doing so, the
/// `OXIGEON_JIT` toggle has broken and nothing else in this file means
/// anything.
///
/// Runs against the probe mudlib rather than the real one, because it needs to
/// evaluate arbitrary Lua and the real mudlib only accepts commands.
///
/// **The loop is defined once and then called**, rather than being sent as
/// source every iteration. That distinction turned out to matter enormously:
/// sending the source each time makes `load` produce a fresh prototype per
/// iteration, so LuaJIT re-records a trace for every single one and thrashes
/// its trace cache. Measured that way the compiler appeared to be worth
/// nothing at all (0.97x) — which is how this control earned its keep, because
/// the reading was an artefact of the benchmark rather than a fact about
/// LuaJIT. Defining it once gives the recorder a stable prototype, which is
/// also what real mudlib code has.
fn numeric_control(c: &mut Criterion) {
    const DEFINE: &str = "_bench = function() local s = 0 \
                          for i = 1, 200000 do s = s + i % 7 end return s end return 'ok'";
    const CALL: &str = "return _bench()";

    let mut group = c.benchmark_group("numeric");
    group.measurement_time(Duration::from_secs(8));

    for (name, jit, limit) in CONFIGS {
        let mut vm = with_jit(*jit, || RealVm::boot_with_instruction_limit(*limit));
        assert_eq!(vm.eval(DEFINE).unwrap(), "ok");
        // 200k iterations against a 1M budget leaves headroom; if this ever
        // tripped, the benchmark would be timing an error path.
        assert!(
            !vm.eval(CALL).is_err(),
            "the control loop must complete under every configuration"
        );
        // Let the recorder settle on a trace before measuring.
        for _ in 0..10 {
            vm.eval(CALL);
        }

        group.bench_with_input(BenchmarkId::from_parameter(name), CALL, |b, src| {
            b.iter(|| std::hint::black_box(vm.eval(src)))
        });
    }
    group.finish();
}

/// What the harness itself costs, so the numbers above can be read net of it.
/// An empty dispatch still crosses the channel twice and wakes the Lua thread.
///
/// Runs first, and gates everything else on the toggle actually working.
fn harness_floor(c: &mut Criterion) {
    assert_the_jit_toggle_works();

    let mut group = c.benchmark_group("floor");
    let mut vm = with_jit(Jit::On, || RealVm::boot_with_instruction_limit(0));
    group.bench_function("round-trip", |b| {
        b.iter(|| std::hint::black_box(vm.eval("return 1")))
    });
    group.finish();
}

criterion_group!(benches, harness_floor, numeric_control, real_commands);
criterion_main!(benches);
