//! What a write costs, and what write-behind buys.
//!
//! `task_list.md` recorded these numbers from a throwaway measurement and asked
//! for them to become a bench group so they stay honest. This is that group.
//!
//! The design claim is narrow and worth stating precisely: **write-behind is
//! not a faster write.** One change costs the same either way — the same single
//! `db_put`, plus a little bookkeeping. What it buys is that the second,
//! third and thousandth change to the same scope are nearly free, because they
//! are one table assignment each and the document is written once per interval
//! rather than once per change.
//!
//! So the headline group measures N changes to one scope, not one change.
//!
//! **The control counts writes rather than comparing times.** Before timing
//! anything, each pair is run once and the document writes are counted: N for
//! write-through, exactly 1 for write-behind. A timing comparison alone cannot
//! tell "write-behind is fast" from "write-behind is not writing", and that
//! distinction is the entire point. The control has already earned its keep —
//! it caught the byte estimate in `cache_d` counting a key's name on every
//! write instead of only the first, which made a long-lived scope creep toward
//! the document ceiling and eventually refuse everything.
//!
//! Two rules inherited from `benches/dispatch.rs`, both learned the hard way:
//!
//!   * **Loop inside the VM.** The harness round trip is ~7 microseconds, which
//!     is three times a memory write. Measuring one write per `eval` measures
//!     the harness.
//!   * **Define the loop once, then call it.** Sending source every iteration
//!     makes `load` produce a fresh prototype each time, LuaJIT re-records a
//!     trace for every one, and the measurement becomes an artefact of the
//!     benchmark rather than a fact about the code.
//!
//! Run it:
//!
//! ```text
//!   cargo bench --bench writes
//! ```

use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

#[path = "../tests/common/mod.rs"]
mod common;

use common::RealVm;

/// How many operations each in-VM loop performs. Large enough that the ~7 us
/// harness round trip amortises to noise, small enough that a `db_put` loop
/// still finishes in a sensible time.
const MEMORY_REPS: usize = 1000;
const DISK_REPS: usize = 100;

/// A VM with the real mudlib, the real cache daemon, and a namespace to write
/// into. The probe game layer is what makes `eval` work against it.
fn vm() -> RealVm {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    vm.eval(
        "DAEMON.cache.define('bench', { tier = 'write_behind', flush_seconds = 0, \
             scope_prefix = 'b:' }) \
         DAEMON.cache.define('bench_through', { tier = 'write_through', \
             scope_prefix = 't:' }) \
         return 'ok'",
    )
    .unwrap();
    vm
}

/// Define a function once, warm it, and return the call expression.
fn define(vm: &mut RealVm, name: &str, body: &str) -> String {
    let src = format!("_{name} = function() {body} end return 'ok'");
    assert_eq!(vm.eval(&src).unwrap(), "ok", "defining {name}");
    let call = format!("return _{name}()");
    for _ in 0..5 {
        assert!(!vm.eval(&call).is_err(), "warming {name}");
    }
    call
}

/// What the harness itself costs, so everything below can be read net of it.
fn harness_floor(c: &mut Criterion) {
    let mut group = c.benchmark_group("floor");
    let mut vm = vm();
    group.bench_function("round-trip", |b| {
        b.iter(|| std::hint::black_box(vm.eval("return 1")))
    });
    group.finish();
}

/// The in-memory primitives, per 1000 operations.
///
/// The row that matters most is `table-assign`: that is what a `cache.set`
/// actually costs once the daemon has done its bookkeeping, and it is the
/// number the whole design rests on. `set_persistent` is included because
/// `task_list.md` quoted it at 2.7 us — a figure that included a full harness
/// round trip per call, so it was mostly measuring the channel.
fn primitives(c: &mut Criterion) {
    let mut group = c.benchmark_group("primitive");
    let mut vm = vm();

    let cases: &[(&str, String)] = &[
        (
            "table-assign",
            format!("local t = _bench_table for i = 1, {MEMORY_REPS} do t.k = i end return 'ok'"),
        ),
        (
            "set_persistent",
            format!("for i = 1, {MEMORY_REPS} do set_persistent('bench_key', i) end return 'ok'"),
        ),
        (
            "set_object_state",
            format!("for i = 1, {MEMORY_REPS} do set_object_state('bench', 'k', i) end return 'ok'"),
        ),
        (
            "cache.set",
            format!("for i = 1, {MEMORY_REPS} do DAEMON.cache.set('bench', 1, 'k', i) end return 'ok'"),
        ),
        (
            "cache.get",
            format!("for i = 1, {MEMORY_REPS} do local _ = DAEMON.cache.get('bench', 1, 'k') end return 'ok'"),
        ),
    ];

    vm.eval("_bench_table = {} return 'ok'").unwrap();
    for (name, body) in cases {
        let call = define(&mut vm, &name.replace('.', "_").replace('-', "_"), body);
        group.bench_with_input(BenchmarkId::from_parameter(name), &call, |b, src| {
            b.iter(|| std::hint::black_box(vm.eval(src)))
        });
    }
    group.finish();
}

/// The document store, per 100 operations.
///
/// `put-new` and `put-overwrite` are separated deliberately: `db_put` runs an
/// existence check before the upsert, so the gap between them is what that
/// extra round trip costs and whether removing it in Rust would be worth
/// anything.
fn document_store(c: &mut Criterion) {
    let mut group = c.benchmark_group("document");
    group.measurement_time(Duration::from_secs(10));
    let mut vm = vm();

    vm.eval("_small = { n = 1, s = 'hello' } \
             _kb = { s = string.rep('x', 1000) } \
             _big = { s = string.rep('x', 8000) } \
             db_put('bench_doc', 'existing', _small) \
             return 'ok'")
        .unwrap();

    let cases: &[(&str, String)] = &[
        ("put-new", format!(
            "for i = 1, {DISK_REPS} do db_put('bench_doc', 'n' .. i, _small) end return 'ok'")),
        ("put-overwrite", format!(
            "for i = 1, {DISK_REPS} do db_put('bench_doc', 'existing', _small) end return 'ok'")),
        ("put-1kb", format!(
            "for i = 1, {DISK_REPS} do db_put('bench_doc', 'existing', _kb) end return 'ok'")),
        ("put-8kb", format!(
            "for i = 1, {DISK_REPS} do db_put('bench_doc', 'existing', _big) end return 'ok'")),
        ("update-one-field", format!(
            "for i = 1, {DISK_REPS} do db_update('bench_doc', 'existing', {{ n = i }}) end return 'ok'")),
        ("get-hit", format!(
            "for i = 1, {DISK_REPS} do local _ = db_get('bench_doc', 'existing') end return 'ok'")),
        ("get-miss", format!(
            "for i = 1, {DISK_REPS} do local _ = db_get('bench_doc', 'absent') end return 'ok'")),
    ];

    for (name, body) in cases {
        let call = define(&mut vm, &name.replace('-', "_"), body);
        group.bench_with_input(BenchmarkId::from_parameter(name), &call, |b, src| {
            b.iter(|| std::hint::black_box(vm.eval(src)))
        });
    }
    group.finish();
}

/// The headline: N changes to one scope, written through versus written behind.
///
/// Write-through is N document writes. Write-behind is N table assignments and
/// one document write, whatever N is. The two should cross almost immediately
/// and then diverge linearly.
///
/// **N = 10 is the number to quote** — roughly one combat round for one actor.
/// The control: prove the two paths differ in the way that matters before
/// timing them.
///
/// A timing comparison alone cannot tell "write-behind is fast" from
/// "write-behind is not writing". Counting the document writes can, exactly,
/// so it runs first and aborts the benchmark if either path has stopped doing
/// what its name says.
fn assert_write_counts(vm: &mut RealVm, through: &str, behind: &str, n: usize) {
    fn puts(vm: &mut RealVm) -> usize {
        vm.eval("return tostring(DAEMON.cache.stats().db_puts)")
            .unwrap()
            .parse()
            .unwrap()
    }

    let before = puts(vm);
    vm.eval(through).unwrap();
    let through_writes = puts(vm) - before;

    let before = puts(vm);
    vm.eval(behind).unwrap();
    let behind_writes = puts(vm) - before;

    let why = vm
        .eval(
            "local i = DAEMON.cache.inspect('bench_through', 1) or {} \
             local s = DAEMON.cache.stats() \
             return 'dirty=' .. tostring(i.dirty) .. ' poisoned=' .. tostring(i.poisoned) \
                 .. ' fails=' .. tostring(i.fails) .. ' loadfail=' .. tostring(i.load_failed) \
                 .. ' failures=' .. tostring(s.flush_failures) \
                 .. ' refused=' .. tostring(s.rejected_writes)",
        )
        .unwrap();

    assert_eq!(
        through_writes, n,
        "write-through/{n} performed {through_writes} document writes, not {n} — \
         it is not writing through, so every number in this group is meaningless ({why})"
    );
    assert_eq!(
        behind_writes, 1,
        "write-behind/{n} performed {behind_writes} document writes, not 1 — \
         it is not batching, so every number in this group is meaningless"
    );
}

fn write_behind(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_behind");
    group.measurement_time(Duration::from_secs(10));
    let mut vm = vm();

    for n in [1usize, 10, 100, 1000] {
        let through = define(
            &mut vm,
            &format!("through_{n}"),
            &format!(
                "for i = 1, {n} do DAEMON.cache.set('bench_through', 1, 'k' .. i, i) end return 'ok'"
            ),
        );
        let behind = define(
            &mut vm,
            &format!("behind_{n}"),
            &format!(
                "for i = 1, {n} do DAEMON.cache.set('bench', 1, 'k' .. i, i) end \
                 DAEMON.cache.flush('bench', 1) return 'ok'"
            ),
        );

        assert_write_counts(&mut vm, &through, &behind, n);

        group.bench_with_input(
            BenchmarkId::new("write-through", n), &through,
            |b, src| b.iter(|| std::hint::black_box(vm.eval(src))),
        );
        group.bench_with_input(
            BenchmarkId::new("write-behind", n), &behind,
            |b, src| b.iter(|| std::hint::black_box(vm.eval(src))),
        );
    }
    group.finish();
}

/// What traits and the effect pipeline cost, because they land on the prompt
/// and the prompt lands on every command.
///
/// The three rows to compare are `value-memo-hit` (the common case, and what
/// makes putting trait resolution on the prompt defensible), `value-recompute`
/// (what a change costs), and `pipeline-none` against `pipeline-three` (what
/// having effects at all costs).
fn traits_and_effects(c: &mut Criterion) {
    let mut group = c.benchmark_group("traits");
    let mut vm = vm();

    vm.eval(
        "local Player = require('lib.player') \
         _p = Player:from_save(1, { name = 'Bench', account_id = 1 }, {}) \
         return 'ok'",
    )
    .unwrap();

    let cases: &[(&str, String)] = &[
        ("value-memo-hit", format!(
            "for i = 1, {MEMORY_REPS} do local _ = DAEMON.trait.value(_p, 'max_hp') end return 'ok'")),
        ("value-recompute", format!(
            "for i = 1, {MEMORY_REPS} do DAEMON.trait.bump(_p) \
             local _ = DAEMON.trait.value(_p, 'max_hp') end return 'ok'")),
        ("touch-idle", format!(
            "for i = 1, {MEMORY_REPS} do DAEMON.trait.touch(_p) end return 'ok'")),
        ("pipeline-none", format!(
            "for i = 1, {MEMORY_REPS} do DAEMON.effect.modify(_p, 'nothing', 30) end return 'ok'")),
    ];

    for (name, body) in cases {
        let call = define(&mut vm, &name.replace('-', "_"), body);
        group.bench_with_input(BenchmarkId::from_parameter(name), &call, |b, src| {
            b.iter(|| std::hint::black_box(vm.eval(src)))
        });
    }

    // The same reads with three effects on, so the difference is the pipeline.
    vm.eval(
        "DAEMON.effect.apply(_p, 'stoneskin', { duration = 99999 }) \
         DAEMON.effect.apply(_p, 'hearty', { duration = 99999 }) \
         DAEMON.effect.apply(_p, 'insight', { duration = 99999 }) return 'ok'",
    )
    .unwrap();

    let buffed: &[(&str, String)] = &[
        ("value-recompute-3-effects", format!(
            "for i = 1, {MEMORY_REPS} do DAEMON.trait.bump(_p) \
             local _ = DAEMON.trait.value(_p, 'max_hp') end return 'ok'")),
        ("pipeline-three", format!(
            "for i = 1, {MEMORY_REPS} do DAEMON.effect.modify(_p, 'trait:constitution', 10) end return 'ok'")),
    ];
    for (name, body) in buffed {
        let call = define(&mut vm, &name.replace('-', "_"), body);
        group.bench_with_input(BenchmarkId::from_parameter(name), &call, |b, src| {
            b.iter(|| std::hint::black_box(vm.eval(src)))
        });
    }
    group.finish();
}

criterion_group!(benches, harness_floor, primitives, document_store, write_behind, traits_and_effects);
criterion_main!(benches);
