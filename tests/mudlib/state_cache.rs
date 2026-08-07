//! What the state cache actually writes, against the real document store.
//!
//! `tests/lua_unit.rs` covers the cache's logic — scheduling, dirty tracking,
//! ephemerality, the flush *plan* — with no database at all, which is what
//! makes those tests fast and honest. It cannot cover any of this:
//!
//!   * that a flush reaches SQLite and honours the document ceiling
//!   * that a deleted key is really gone from the stored document
//!   * that a scope round-trips through `lua_to_json` and back
//!   * that the Lua safety check agrees with the Rust encoder it mirrors
//!   * that a failed read is not mistaken for an absent one
//!   * that `on_shutdown` flushes, under system identity, before the VM stops
//!
//! A stubbed `db_put` recording into a table would prove a function was called
//! and nothing else. This is the half that has to be real — the same lesson
//! `CLAUDE.md` records about the sandbox and the instruction budget.

use std::time::Duration;

use crate::common::RealVm;

/// Long enough that a healthy mudlib always finishes.
const GENEROUS: Duration = Duration::from_secs(10);

/// The real mudlib — so `DAEMON.cache` is the one `mudlib/init.lua` wired up,
/// not one the test required for itself.
fn vm() -> RealVm {
    let mut vm = RealVm::boot_fixture_with_probe();
    vm.eval(
        "DAEMON.cache.define('probe', { tier = 'write_behind', flush_seconds = 0, \
             scope_prefix = 'p:', delete_when_empty = true }) return 'ok'",
    )
    .unwrap();
    vm
}

fn flush_all(vm: &mut RealVm) -> String {
    vm.eval("return tostring(DAEMON.cache.flush_all({ reason = 'test' }))").unwrap()
}

#[test]
fn the_cache_is_wired_into_the_real_mudlib() {
    let mut vm = RealVm::boot_fixture_with_probe();
    assert_eq!(vm.eval("return type(DAEMON.cache)").unwrap(), "table");
    assert_eq!(vm.eval("return type(DAEMON.cooldown)").unwrap(), "table");
    assert_eq!(vm.eval("return type(DAEMON.trait)").unwrap(), "table");
    assert_eq!(vm.eval("return type(DAEMON.effect)").unwrap(), "table");
    // The whole lua_unit suite would stay green if init.lua never mentioned
    // any of them.
}

#[test]
fn a_flush_writes_the_whole_scope_as_one_document() {
    let mut vm = vm();
    vm.eval("for i = 1, 50 do DAEMON.cache.set('probe', 7, 'k' .. i, i * 3) end return 'ok'")
        .unwrap();

    assert_eq!(flush_all(&mut vm), "1", "fifty keys are one document write");

    assert_eq!(
        vm.eval("local r = db_get('probe', 'p:7') local n = 0 \
                 for _ in pairs(r.data) do n = n + 1 end return n .. '/' .. r.data.k50")
            .unwrap(),
        "50/150"
    );
}

/// The argument for `db_put` over `db_update`, made executable.
///
/// RFC 7396 expresses deletion as a JSON null, Lua tables cannot hold nil, so
/// a merge-based flush can add and change but never remove. An effect that
/// ended would linger in the document forever while memory disagreed. This is
/// the test that would catch someone "optimising" the flush into a patch.
#[test]
fn a_deleted_key_is_gone_from_the_stored_document() {
    let mut vm = vm();
    vm.eval("DAEMON.cache.set('probe', 1, 'keep', 'yes') \
             DAEMON.cache.set('probe', 1, 'gone', 'for now') return 'ok'")
        .unwrap();
    flush_all(&mut vm);
    assert_eq!(vm.eval("return db_get('probe', 'p:1').data.gone").unwrap(), "for now");

    vm.eval("DAEMON.cache.delete('probe', 1, 'gone') return 'ok'").unwrap();
    flush_all(&mut vm);

    assert_eq!(
        vm.eval("return tostring(db_get('probe', 'p:1').data.gone)").unwrap(),
        "nil",
        "a merge flush would have left this behind forever"
    );
    assert_eq!(vm.eval("return db_get('probe', 'p:1').data.keep").unwrap(), "yes");
}

#[test]
fn a_scope_that_empties_is_deleted_rather_than_left_as_an_empty_document() {
    let mut vm = vm();
    vm.eval("DAEMON.cache.set('probe', 2, 'only', 1) return 'ok'").unwrap();
    flush_all(&mut vm);
    assert_eq!(vm.eval("return tostring(db_exists('probe', 'p:2'))").unwrap(), "true");

    vm.eval("DAEMON.cache.delete('probe', 2, 'only') return 'ok'").unwrap();
    flush_all(&mut vm);
    assert_eq!(
        vm.eval("return tostring(db_exists('probe', 'p:2'))").unwrap(),
        "false",
        "otherwise every player who ever had one counts against max_per_collection"
    );
}

/// `lua_to_json` composed with `json_to_lua` is *not* an identity, and the
/// places it is not are worth pinning as documented behaviour rather than
/// leaving to be discovered by whoever stores the wrong shape.
#[test]
fn a_scope_round_trips_through_json_with_known_edges() {
    let mut vm = vm();
    vm.eval(
        "DAEMON.cache.set('probe', 3, 'nested', { a = { b = { c = 1 } } }) \
         DAEMON.cache.set('probe', 3, 'list', { 10, 20, 30 }) \
         DAEMON.cache.set('probe', 3, 'flag', false) \
         DAEMON.cache.set('probe', 3, 'float', 1.5) \
         DAEMON.cache.set('probe', 3, 'empty', {}) return 'ok'",
    )
    .unwrap();
    flush_all(&mut vm);
    // Drop it from memory so the read has to come from the database.
    vm.eval("DAEMON.cache.drop('probe', 3) return 'ok'").unwrap();

    assert_eq!(
        vm.eval("local s = DAEMON.cache.get_scope('probe', 3) \
                 return s.nested.a.b.c .. '/' .. s.list[3] .. '/' .. tostring(s.flag) \
                     .. '/' .. s.float .. '/' .. type(s.empty)")
            .unwrap(),
        "1/30/false/1.5/table"
    );
}

/// The Lua safety check is a reimplementation of Rust logic, and
/// reimplementations drift. This is what stops it: the same hostile values go
/// to both, and they must agree on every one.
#[test]
fn the_validator_and_the_real_encoder_agree() {
    let mut vm = vm();
    let cases = [
        ("a plain table", "{ a = 1, b = 'two' }"),
        ("an array", "{ 1, 2, 3 }"),
        ("a nested table", "{ x = { y = { z = 1 } } }"),
        ("a mixed list and map", "{ 1, 2, name = 'x' }"),
        ("a function", "{ f = function() end }"),
        ("infinity", "{ n = 1/0 }"),
        ("negative infinity", "{ n = -1/0 }"),
        ("NaN", "{ n = 0/0 }"),
        ("a boolean key", "{ [true] = 1 }"),
        ("a deeply nested table", "(function() local t = {} local c = t \
             for _ = 1, 80 do c.n = {} c = c.n end return t end)()"),
        ("an empty table", "{}"),
        ("a float", "{ n = 1.5 }"),
        ("a negative number", "{ n = -3 }"),
        ("a long string", "{ s = string.rep('x', 500) }"),
    ];

    for (name, expr) in cases {
        let verdict = vm
            .eval(&format!(
                "local js = require('lib.jsonsafe') \
                 local v = {expr} \
                 local lua_ok = js.check(v) \
                 local rust_ok = pcall(db_put, 'agree', 'x', v) \
                 return tostring(lua_ok) .. '/' .. tostring(rust_ok)"
            ))
            .unwrap();
        let (lua_ok, rust_ok) = verdict.split_once('/').unwrap();
        assert_eq!(
            lua_ok, rust_ok,
            "the Lua check and lua_to_json disagree about {name}: \
             lib/jsonsafe.lua says {lua_ok}, the encoder says {rust_ok}"
        );
    }
}

/// A value the store would refuse must not be kept in memory either, or the
/// two disagree forever and nothing ever says so.
#[test]
fn an_unwritable_value_is_refused_before_it_is_stored() {
    let mut vm = vm();
    assert_eq!(
        vm.eval("return tostring(DAEMON.cache.set('probe', 4, 'bad', { f = function() end }))")
            .unwrap(),
        "false"
    );
    assert_eq!(
        vm.eval("return tostring(DAEMON.cache.get('probe', 4, 'bad'))").unwrap(),
        "nil"
    );
    // And the scope is still writable afterwards.
    vm.eval("DAEMON.cache.set('probe', 4, 'good', 1) return 'ok'").unwrap();
    flush_all(&mut vm);
    assert_eq!(vm.eval("return db_get('probe', 'p:4').data.good").unwrap(), "1");
}

/// Repeatedly asking whether a player has any effects must not be a database
/// query every time.
#[test]
fn an_absent_document_is_remembered_as_absent() {
    let mut vm = vm();
    let gets = |vm: &mut RealVm| {
        vm.eval("return tostring(DAEMON.cache.stats().db_gets)").unwrap()
    };
    vm.eval("local _ = DAEMON.cache.get('probe', 999, 'nothing') return 'ok'").unwrap();
    let after_first = gets(&mut vm);
    for _ in 0..5 {
        vm.eval("local _ = DAEMON.cache.get('probe', 999, 'nothing') return 'ok'").unwrap();
    }
    assert_eq!(gets(&mut vm), after_first, "five more reads, no more queries");
}

/// The most dangerous mistake available in this design: if a read that *failed*
/// were remembered as "absent", the next flush would `db_put` an empty
/// document over the player's real data.
#[test]
fn a_failed_read_is_not_mistaken_for_an_absent_document() {
    let mut vm = vm();
    // Get a real document on disk first.
    vm.eval("DAEMON.cache.set('probe', 5, 'precious', 'do not lose me') return 'ok'").unwrap();
    flush_all(&mut vm);
    vm.eval("DAEMON.cache.drop('probe', 5) return 'ok'").unwrap();

    // Now make the read fail, the way a locked or broken database would.
    vm.eval("_real_db_get = db_get function db_get() error('database is down') end return 'ok'")
        .unwrap();
    vm.eval("local _ = DAEMON.cache.get('probe', 5, 'precious') return 'ok'").unwrap();

    assert_eq!(
        vm.eval("return tostring(DAEMON.cache.inspect('probe', 5).load_failed)").unwrap(),
        "true"
    );

    // Writing during the outage and flushing must not wipe what is on disk.
    vm.eval("DAEMON.cache.set('probe', 5, 'written_during_outage', 1) return 'ok'").unwrap();
    vm.eval("db_get = _real_db_get return 'ok'").unwrap();
    flush_all(&mut vm);

    assert_eq!(
        vm.eval("return tostring(db_get('probe', 'p:5').data.precious)").unwrap(),
        "do not lose me",
        "a failed load must never be allowed to become an empty document"
    );
}

/// A scope is a document, and a document has a 64 KB ceiling. The write that
/// would cross it is refused at the call site — naming the tenant — rather
/// than raising inside `on_shutdown` for one unlucky player.
#[test]
fn an_oversize_write_is_refused_before_it_can_break_a_flush() {
    let mut vm = vm();
    let refused = vm
        .eval(
            "local blob = string.rep('x', 4000) \
             local refusals = 0 \
             for i = 1, 40 do \
                 if not DAEMON.cache.set('probe', 6, 'blob' .. i, blob) then refusals = refusals + 1 end \
             end \
             return tostring(refusals > 0)",
        )
        .unwrap();
    assert_eq!(refused, "true", "the ceiling has to bite somewhere before 160 KB");

    // And what did fit still flushes cleanly.
    assert_eq!(flush_all(&mut vm), "1");
    assert_eq!(
        vm.eval("return tostring(DAEMON.cache.stats().rejected_writes > 0)").unwrap(),
        "true"
    );
}

/// The whole point of the tier, measured in writes rather than microseconds:
/// a hundred changes to one scope are one document write.
#[test]
fn a_hundred_changes_are_one_write() {
    let mut vm = vm();
    let before = vm.eval("return tostring(DAEMON.cache.stats().db_puts)").unwrap();
    vm.eval("for i = 1, 100 do DAEMON.cache.set('probe', 8, 'counter', i) end return 'ok'")
        .unwrap();
    flush_all(&mut vm);
    let after = vm.eval("return tostring(DAEMON.cache.stats().db_puts)").unwrap();

    assert_eq!(
        after.parse::<i64>().unwrap() - before.parse::<i64>().unwrap(),
        1,
        "write-through would have made this a hundred"
    );
    assert_eq!(vm.eval("return db_get('probe', 'p:8').data.counter").unwrap(), "100");
}

/// The tier rule for cooldowns, through the real store: a daily gate is on
/// disk the moment it is set, a six-second one never touches it.
#[test]
fn a_durable_cooldown_is_on_disk_and_a_fast_one_is_not() {
    let mut vm = RealVm::boot_fixture_with_probe();
    vm.eval("DAEMON.cooldown.mark(42, 'manasteel', 86400) \
             DAEMON.cooldown.mark(42, 'fireball', 6) return 'ok'")
        .unwrap();

    assert_eq!(
        vm.eval("return tostring(db_get('cooldowns', 'char:42').data.manasteel ~= nil)").unwrap(),
        "true",
        "write-through means it is on disk without waiting for a flush"
    );
    assert_eq!(
        vm.eval("return tostring(db_exists('cooldowns_fast', 'char:42'))").unwrap(),
        "false",
        "the memory tier must never create a collection at all"
    );
}

/// A restart clears exactly one thing: `_persistent_store`. Clearing it by hand
/// is a faithful simulation and needs no second VM.
#[test]
fn a_restart_keeps_durable_cooldowns_and_forgets_the_fast_ones() {
    let mut vm = RealVm::boot_fixture_with_probe();
    vm.eval("DAEMON.cooldown.mark(7, 'daily', 86400) \
             DAEMON.cooldown.mark(7, 'gcd', 6) return 'ok'")
        .unwrap();

    vm.eval("set_persistent('cache_d', nil) \
             package.loaded['daemons.cache_d'] = nil \
             package.loaded['daemons.cooldown_d'] = nil \
             DAEMON.cache = require('daemons.cache_d') \
             DAEMON.cooldown = require('daemons.cooldown_d') return 'ok'")
        .unwrap();

    assert_eq!(
        vm.eval("return tostring(DAEMON.cooldown.remaining(7, 'daily') > 0)").unwrap(),
        "true",
        "a 24-hour gate is a promise to the player and must survive"
    );
    assert_eq!(
        vm.eval("return tostring(DAEMON.cooldown.remaining(7, 'gcd'))").unwrap(),
        "0",
        "a six-second gate is a game mechanic and should not"
    );
}

/// The load-bearing test for the whole design: the flush happens on the real
/// shutdown path, under the engine's identity, before the VM stops — and the
/// data outlives the process.
#[test]
fn the_shutdown_flush_reaches_the_database() {
    let mut vm = vm();
    vm.eval("DAEMON.cache.set('probe', 11, 'written_at_the_end', 'yes') return 'ok'")
        .unwrap();
    // Not flushed yet.
    assert_eq!(vm.eval("return tostring(db_exists('probe', 'p:11'))").unwrap(), "false");

    assert!(vm.shutdown_within(GENEROUS), "the mudlib did not finish shutting down");

    // `vm.pool()` outlives the VM, so this is the only honest way to ask.
    use diesel::prelude::*;
    use diesel::sql_types::Text;
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        data: String,
    }
    let mut conn = vm.pool().get_sqlite().unwrap();
    let rows: Vec<Row> = diesel::sql_query(
        "SELECT data FROM documents WHERE collection = 'probe' AND id = 'p:11'",
    )
    .load(&mut conn)
    .unwrap();

    assert_eq!(rows.len(), 1, "on_shutdown did not flush the state cache");
    assert!(
        rows[0].data.contains("written_at_the_end"),
        "the document reached disk but without the data: {}",
        rows[0].data
    );
}
