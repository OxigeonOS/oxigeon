//! L1 and L2 — what the game stops holding on to when things stop existing.
//!
//! The Lua GC will not fall over from normal gameplay; LuaJIT handles far
//! larger heaps than a MUD produces, and the design already avoids the
//! expensive mistakes. What matters is **unbounded retention**, which raises
//! the cost of every mark phase forever, and the fact that nothing measured
//! any of it.
//!
//! Two leaks, both of the same shape — something was detached on the way out
//! and its object state was not:
//!
//!   L1  A mob's object state outlived its despawn. Instance ids are
//!       `"mob:" .. seq`, monotonic and never reused, so a respawn loop
//!       churned ids and left a sub-table behind for each one. The only
//!       pruning anywhere is `world_d`'s on area reset, which covers rooms in
//!       a registered area source — not mobs, not items, not virtual rooms.
//!
//!   L2  `evict_virtual` had zero callers. `world-building.md` said a virtual
//!       room is "cached in the registry while occupied"; nothing un-cached
//!       it. Bounded for a small ocean, unbounded for an infinite grid.
//!
//! Counted rather than measured: a key count is exact, and a heap number under
//! a running GC is not. `mudstatus` grew heap counters in the same pass so the
//! drill in `roadmap.md` has something to read, but a *test* should assert the
//! thing that is deterministic.

use crate::common::RealVm;

/// How many objects have any state at all.
fn state_keys(vm: &mut RealVm) -> i64 {
    vm.eval("local n = 0 for _ in pairs(_object_state_store) do n = n + 1 end return n")
        .unwrap()
        .parse()
        .expect("key count")
}

/// L1 — a despawned mob leaves nothing behind.
#[test]
fn a_despawned_mob_takes_its_object_state_with_it() {
    let mut vm = RealVm::boot_fixture_with_probe();

    vm.eval(
        "DAEMON.mobs.register({ id = 'leaky_rat', short = 'a rat', \
         stats = { hp = 5, max_hp = 5 } })",
    )
    .unwrap();

    let before = state_keys(&mut vm);

    vm.eval("_m = DAEMON.mobs.spawn('leaky_rat', 'nowhere.at.all')").unwrap();
    assert_eq!(
        vm.eval("return tostring(_m ~= nil)").unwrap(),
        "true",
        "the mob did not spawn"
    );
    vm.eval("set_object_state(_m.id, 'looted', true)").unwrap();
    assert_eq!(
        vm.eval("return tostring(get_object_state(_m.id, 'looted'))").unwrap(),
        "true"
    );
    assert_eq!(state_keys(&mut vm), before + 1, "the write should have made an entry");

    vm.eval("_id = _m.id; DAEMON.mobs.despawn(_m)").unwrap();

    assert_eq!(
        vm.eval("return tostring(get_all_object_state(_id))").unwrap(),
        "nil",
        "the mob's object state outlived its despawn"
    );
    assert_eq!(
        state_keys(&mut vm),
        before,
        "the store should be back where it started"
    );
}

/// The shape that actually hurts: ids are never reused, so the store grows with
/// `seq` rather than with the number of mobs alive. A thousand cycles is a few
/// seconds of a respawning area.
#[test]
fn a_thousand_spawn_despawn_cycles_leave_the_store_flat() {
    let mut vm = RealVm::boot_fixture_with_probe();

    vm.eval(
        "DAEMON.mobs.register({ id = 'churn_rat', short = 'a rat', \
         stats = { hp = 5, max_hp = 5 } })",
    )
    .unwrap();

    let before = state_keys(&mut vm);
    // The real game layer loads under the probe, so its own area is already
    // populated. The claim is about the delta this loop leaves behind, not
    // about the absolute count.
    let alive_before: i64 = vm
        .eval("return tostring(DAEMON.mobs.count())")
        .unwrap()
        .parse()
        .unwrap();

    vm.eval(
        "for i = 1, 1000 do \
           local m = DAEMON.mobs.spawn('churn_rat', 'nowhere.at.all') \
           set_object_state(m.id, 'seen', i) \
           DAEMON.mobs.despawn(m) \
         end return 'done'",
    )
    .unwrap();

    assert_eq!(
        state_keys(&mut vm),
        before,
        "the object state store grew with the instance counter — every mob that \
         ever had state written is still being walked by every mark phase"
    );

    // And the daemon's own bookkeeping agrees that none of them is still alive.
    let alive_after: i64 = vm
        .eval("return tostring(DAEMON.mobs.count())")
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        alive_after, alive_before,
        "instances leaked as well as their state"
    );
}

/// L2 — a virtual room is dropped when its last occupant leaves, and its object
/// state goes with it.
#[test]
fn a_virtual_room_is_evicted_when_its_last_occupant_leaves() {
    let mut vm = RealVm::boot_fixture_with_probe();

    // A minimal provider: any `reach.X.Y` is a room. This is the shape the
    // drowned reach uses, reduced to what the eviction question needs.
    vm.eval(
        "DAEMON.world.register_virtual('reach', function(room_id) \
           return DAEMON.room.from_data({ id = room_id, short = 'Open water', \
             long = 'Grey water in every direction.', exits = {} }) \
         end) return 'registered'",
    )
    .unwrap();

    let cached = |vm: &mut RealVm| -> i64 {
        vm.eval("local n = 0 for _ in pairs(DAEMON.world._rooms) do n = n + 1 end return n")
            .unwrap()
            .parse()
            .unwrap()
    };
    let before = cached(&mut vm);

    // Walk out into the grid. Each step generates and caches a room.
    vm.eval("DAEMON.world.place_character(7, 'reach.0.0')").unwrap();
    vm.eval("set_object_state('reach.0.0', 'buoy', true)").unwrap();
    assert_eq!(cached(&mut vm), before + 1, "the first room should be cached");

    vm.eval("DAEMON.world.move_character(7, 'reach.0.1')").unwrap();
    assert_eq!(
        cached(&mut vm),
        before + 1,
        "leaving one virtual room and entering another should be a wash — the \
         one behind you is evicted, the one you arrived in is cached"
    );
    assert_eq!(
        vm.eval("return tostring(DAEMON.world._rooms['reach.0.0'])").unwrap(),
        "nil",
        "the abandoned room is still in the registry"
    );
    assert_eq!(
        vm.eval("return tostring(get_all_object_state('reach.0.0'))").unwrap(),
        "nil",
        "the evicted room's object state was left behind"
    );

    // Walking a long way and coming back must not accumulate.
    vm.eval(
        "for y = 2, 200 do DAEMON.world.move_character(7, 'reach.0.' .. y) end return 'walked'",
    )
    .unwrap();
    assert_eq!(
        cached(&mut vm),
        before + 1,
        "200 steps through an infinite grid cached 200 rooms — this is the \
         unbounded half of the leak, and it is what blocks the drowned reach"
    );

    // Leaving the world entirely evicts the last one too: disconnecting at sea
    // is the common case for a virtual room's last occupant departing.
    vm.eval("DAEMON.world.remove_character(7)").unwrap();
    assert_eq!(
        cached(&mut vm),
        before,
        "the last virtual room survived its occupant disconnecting"
    );
}

/// The room id is the persistence. Regeneration on return has to still work, or
/// eviction would be data loss rather than cache management.
#[test]
fn an_evicted_virtual_room_regenerates_on_return() {
    let mut vm = RealVm::boot_fixture_with_probe();

    vm.eval(
        "DAEMON.world.register_virtual('reach', function(room_id) \
           local x, y = room_id:match('^reach%.(%-?%d+)%.(%-?%d+)$') \
           if not x then return nil end \
           return DAEMON.room.from_data({ id = room_id, \
             short = 'Open water at ' .. x .. ',' .. y, exits = {} }) \
         end) return 'registered'",
    )
    .unwrap();

    vm.eval("DAEMON.world.place_character(7, 'reach.3.4')").unwrap();
    assert_eq!(
        vm.eval("return DAEMON.world.get_room('reach.3.4').short").unwrap(),
        "Open water at 3,4"
    );

    vm.eval("DAEMON.world.move_character(7, 'reach.9.9')").unwrap();
    assert_eq!(
        vm.eval("return tostring(DAEMON.world._rooms['reach.3.4'])").unwrap(),
        "nil",
        "expected the room to have been evicted"
    );

    vm.eval("DAEMON.world.move_character(7, 'reach.3.4')").unwrap();
    assert_eq!(
        vm.eval("return DAEMON.world.get_room('reach.3.4').short").unwrap(),
        "Open water at 3,4",
        "a room regenerated from its id should be identical to the one evicted"
    );
}

/// A static room must never be evicted, whoever leaves it. `evict_virtual`
/// already checked the prefix; this pins that the new automatic caller has not
/// widened it.
#[test]
fn a_static_room_is_never_evicted() {
    let mut vm = RealVm::boot_fixture_with_probe();

    vm.eval(
        "DAEMON.world.register_area({ DAEMON.room.from_data({ id = 'solid.hall', \
         short = 'A hall', exits = {} }) }) return 'registered'",
    )
    .unwrap();
    vm.eval("set_object_state('solid.hall', 'door_open', true)").unwrap();

    vm.eval("DAEMON.world.place_character(8, 'solid.hall')").unwrap();
    vm.eval("DAEMON.world.remove_character(8)").unwrap();

    assert_eq!(
        vm.eval("return tostring(DAEMON.world._rooms['solid.hall'] ~= nil)").unwrap(),
        "true",
        "a static room was evicted when its last occupant left"
    );
    assert_eq!(
        vm.eval("return tostring(get_object_state('solid.hall', 'door_open'))").unwrap(),
        "true",
        "a static room's object state must survive everyone leaving — that is \
         what makes a door stay open"
    );
}

/// M1 — the heap counters exist and answer, so the drill in `roadmap.md` has a
/// number to read rather than an argument to have.
#[test]
fn server_info_reports_the_lua_heap() {
    let mut vm = RealVm::boot_fixture_with_probe();

    assert_eq!(
        vm.eval("return tostring(server_info().lua ~= nil)").unwrap(),
        "true",
        "server_info() should carry a `lua` sub-table"
    );

    let heap: f64 = vm
        .eval("return tostring(server_info().lua.heap_bytes)")
        .unwrap()
        .parse()
        .expect("heap_bytes");
    assert!(heap > 0.0, "the heap cannot be empty with a mudlib loaded");

    // The ceiling is the number that makes the heap reading mean something.
    let limit: f64 = vm
        .eval("return tostring(server_info().lua.limit_bytes)")
        .unwrap()
        .parse()
        .expect("limit_bytes");
    assert_eq!(limit, 64.0 * 1024.0 * 1024.0, "expected the configured 64 MB ceiling");

    // A full collection reports what it cost, and the counter moves.
    assert_eq!(
        vm.eval("return tostring(server_info().lua.gc_full_count)").unwrap(),
        "0"
    );
    vm.eval("_gc = gc_collect() return 'collected'").unwrap();
    assert_eq!(
        vm.eval("return tostring(_gc.ms >= 0 and _gc.heap_bytes > 0)").unwrap(),
        "true",
        "gc_collect should report a duration and the heap it left behind"
    );
    assert_eq!(
        vm.eval("return tostring(server_info().lua.gc_full_count)").unwrap(),
        "1",
        "the cumulative counter did not move"
    );
}
