//! The `db_*` efuns as game code reaches them.
//!
//! `tests/document_store.rs` covers the SQL. These drive the VM the engine
//! actually builds, because that is where the efuns are registered, where the
//! sandbox could hide them, and where permission gating and system-dispatch
//! identity apply.

mod common;

use std::collections::HashMap;

use common::RealVm;
use oxigeon::config::permissions_config::PermissionConfig;

// ─── the basics, through Lua ─────────────────────────────────────────────────

#[test]
fn the_db_efuns_survive_the_sandbox() {
    let mut vm = RealVm::boot();
    for name in [
        "db_put", "db_insert", "db_get", "db_exists", "db_delete", "db_find", "db_count",
        "db_update", "db_unset", "db_incr", "db_collections", "db_clear",
    ] {
        assert!(vm.reaches(name), "`{name}` should be callable from game code");
    }
}

#[test]
fn a_document_round_trips_through_lua() {
    let mut vm = RealVm::boot();
    assert_eq!(
        vm.eval(
            "db_put('t', 'a', { n = 7, s = 'hi', b = true, list = {1,2,3}, nested = { x = 1 } }) \
             local r = db_get('t', 'a') \
             return r.data.n .. '/' .. r.data.s .. '/' .. tostring(r.data.b) \
                 .. '/' .. r.data.list[3] .. '/' .. r.data.nested.x"
        )
        .unwrap(),
        "7/hi/true/3/1"
    );
}

/// The envelope, not a bare document — so an id and timestamps come back
/// without a reserved key name.
#[test]
fn a_read_returns_a_record_with_its_id_and_timestamps() {
    let mut vm = RealVm::boot();
    assert_eq!(
        vm.eval(
            "db_put('t', 'abc', { v = 1 }) local r = db_get('t', 'abc') \
             return r.collection .. '|' .. r.id .. '|' .. tostring(#r.created_at > 0) \
                 .. '|' .. tostring(r.data.v)"
        )
        .unwrap(),
        "t|abc|true|1"
    );
}

#[test]
fn a_missing_document_is_nil_not_an_error() {
    let mut vm = RealVm::boot();
    assert_eq!(vm.eval("return tostring(db_get('t', 'nope'))").unwrap(), "nil");
    assert_eq!(vm.eval("return tostring(db_exists('t', 'nope'))").unwrap(), "false");
    assert_eq!(vm.eval("return #db_find('nothing_here')").unwrap(), "0");
}

// ─── the failure convention ──────────────────────────────────────────────────

/// Author errors raise rather than returning false, so a report that was not
/// saved cannot look like one that was.
#[test]
fn author_errors_raise_and_name_the_offender() {
    let mut vm = RealVm::boot();

    let bad_field = vm.eval("return db_find('t', { [\"x' OR 1=1--\"] = 1 })");
    assert!(bad_field.is_err());
    assert!(bad_field.err().contains("not a valid document field"));

    let bad_op = vm.eval("return db_find('t', { n = { ['drop'] = 1 } })");
    assert!(bad_op.err().contains("unknown operator"));

    let bad_order = vm.eval("return db_find('t', nil, { order = 'sideways' })");
    assert!(bad_order.err().contains("asc"));

    let bad_collection = vm.eval("return db_put('Reports', 'a', {})");
    assert!(bad_collection.err().contains("lowercase"));

    let bad_id = vm.eval("return db_put('t', 'has space', {})");
    assert!(bad_id.err().contains("may only contain"));
}

/// The mixed-table refusal from `lua_to_json` reaches here too — a document
/// that is both a list and a map cannot be stored, and says so.
#[test]
fn a_document_json_cannot_represent_is_refused() {
    let mut vm = RealVm::boot();
    let err = vm.eval("return db_put('t', 'a', {1, 2, name = 'x'})").err();
    assert!(err.contains("'name'"), "should name the key at risk: {err}");
}

// ─── the filter language ─────────────────────────────────────────────────────

fn seeded() -> RealVm {
    let mut vm = RealVm::boot();
    vm.eval(
        "db_put('reports','R1',{status='open',priority=1,reporter='amy',tags={'bug'}}) \
         db_put('reports','R2',{status='open',priority=5,reporter='bo',tags={'bug','urgent'}}) \
         db_put('reports','R3',{status='closed',priority=3,reporter='amy'}) \
         db_put('reports','R4',{priority=2,reporter='cy'}) return 'ok'",
    )
    .unwrap();
    vm
}

#[test]
fn filters_work_from_lua() {
    let mut vm = seeded();

    assert_eq!(vm.eval("return #db_find('reports', { status = 'open' })").unwrap(), "2");
    assert_eq!(
        vm.eval("return #db_find('reports', { priority = { ['>='] = 3 } })").unwrap(),
        "2"
    );
    assert_eq!(
        vm.eval("return #db_find('reports', { tags = { contains = 'urgent' } })").unwrap(),
        "1"
    );
    assert_eq!(
        vm.eval("return #db_find('reports', { status = { exists = false } })").unwrap(),
        "1"
    );
    assert_eq!(
        vm.eval("return #db_find('reports', { reporter = { ['in'] = {'amy','cy'} } })").unwrap(),
        "3"
    );
    assert_eq!(vm.eval("return db_count('reports')").unwrap(), "4");
}

#[test]
fn sorting_and_paging_work_from_lua() {
    let mut vm = seeded();
    assert_eq!(
        vm.eval("local r = db_find('reports', nil, { sort='priority', order='desc' }) return r[1].id")
            .unwrap(),
        "R2"
    );
    assert_eq!(
        vm.eval("return #db_find('reports', nil, { limit = 2, offset = 2 })").unwrap(),
        "2"
    );
}

// ─── atomic operations ───────────────────────────────────────────────────────

#[test]
fn update_merges_and_incr_counts() {
    let mut vm = RealVm::boot();
    assert_eq!(
        vm.eval(
            "db_put('t','a',{keep=1, nested={x=1,y=2}}) \
             db_update('t','a',{nested={y=9}, added=true}) \
             local r = db_get('t','a') \
             return r.data.keep .. '/' .. r.data.nested.x .. '/' .. r.data.nested.y \
                 .. '/' .. tostring(r.data.added)"
        )
        .unwrap(),
        "1/1/9/true"
    );

    // db_incr upserts, so the very first call creates the counter document —
    // no bootstrap step for a sequence number.
    assert_eq!(vm.eval("return db_incr('counters','reports','next')").unwrap(), "1");
    assert_eq!(vm.eval("return db_incr('counters','reports','next')").unwrap(), "2");
    assert_eq!(vm.eval("return db_incr('counters','reports','next', 5)").unwrap(), "7");
}

#[test]
fn incr_on_a_text_field_raises_rather_than_clobbering_it() {
    let mut vm = RealVm::boot();
    vm.eval("db_put('t','a',{v='five'}) return 'ok'").unwrap();
    assert!(vm.eval("return db_incr('t','a','v')").err().contains("not a number"));
    assert_eq!(vm.eval("return db_get('t','a').data.v").unwrap(), "five");
}

#[test]
fn unset_removes_a_field() {
    let mut vm = RealVm::boot();
    assert_eq!(
        vm.eval(
            "db_put('t','a',{keep=1, drop=2}) db_unset('t','a','drop') \
             local r = db_get('t','a') \
             return tostring(r.data.keep) .. '/' .. tostring(r.data.drop)"
        )
        .unwrap(),
        "1/nil"
    );
}

// ─── permissions and system dispatch ─────────────────────────────────────────

fn gate(efun: &str) -> PermissionConfig {
    let mut efuns = HashMap::new();
    efuns.insert(efun.to_string(), "admin".to_string());
    PermissionConfig { efuns, directories: HashMap::new() }
}

/// Any `db_*` efun can be gated from permissions.toml without a code change,
/// because they all consult the table unconditionally.
#[test]
fn a_gated_db_efun_is_refused_for_an_unprivileged_session() {
    let mut vm = RealVm::boot_with_permissions(gate("db_clear"));
    vm.eval("db_put('t','a',{}) return 'ok'").unwrap();

    let denied = vm.eval("return db_clear('t')");
    assert!(denied.is_err());
    assert!(denied.err().contains("Permission denied"));

    // Ungated calls still work.
    assert_eq!(vm.eval("return tostring(db_exists('t','a'))").unwrap(), "true");
}

/// A sweeper daemon on a ticker has no session behind it. This is the path
/// that used to fail closed and silently — worth pinning for the store, since
/// "prune old reports on a tick" is the obvious first thing anyone writes.
#[test]
fn a_timer_tick_may_use_a_gated_db_efun() {
    let mut vm = RealVm::boot_with_permissions(gate("db_clear"));
    vm.eval("db_put('t','a',{}) db_put('t','b',{}) return 'ok'").unwrap();

    assert_eq!(vm.eval_on_timer("return db_clear('t')").unwrap(), "2");
    assert_eq!(vm.eval("return db_count('t')").unwrap(), "0");
}

// ─── the worked example ──────────────────────────────────────────────────────

/// The whole point, end to end: a player-report tool with no Rust, no
/// migration, and no schema change.
#[test]
fn a_player_report_tool_works_entirely_from_lua() {
    let mut vm = RealVm::boot();

    // File three reports, numbered from a counter the store bootstraps itself.
    let filed = vm
        .eval(
            "local ids = {} \
             for _, who in ipairs({'amy','bo','amy'}) do \
               local n = db_incr('counters', 'reports', 'next') \
               local id = string.format('R%04d', n) \
               db_put('reports', id, { status='open', reporter=who, \
                                       summary='something broke', priority=1 }) \
               ids[#ids+1] = id \
             end \
             return table.concat(ids, ',')",
        )
        .unwrap();
    assert_eq!(filed, "R0001,R0002,R0003");

    // The staff view.
    assert_eq!(vm.eval("return db_count('reports', { status = 'open' })").unwrap(), "3");
    assert_eq!(
        vm.eval("return db_find('reports', { status='open' }, { limit = 20 })[1].id").unwrap(),
        "R0001"
    );

    // Resolve one — a partial merge that leaves everything else alone.
    vm.eval(
        "db_update('reports', 'R0002', { status='closed', resolved_by='staff' }) return 'ok'",
    )
    .unwrap();

    assert_eq!(vm.eval("return db_count('reports', { status = 'open' })").unwrap(), "2");
    assert_eq!(
        vm.eval("return db_get('reports','R0002').data.reporter").unwrap(),
        "bo",
        "the merge must not have dropped the untouched fields"
    );
    assert_eq!(
        vm.eval("return db_get('reports','R0002').data.resolved_by").unwrap(),
        "staff"
    );

    // And a per-reporter view, which is the query a moderation tool wants.
    assert_eq!(
        vm.eval("return #db_find('reports', { reporter = 'amy' })").unwrap(),
        "2"
    );
}
