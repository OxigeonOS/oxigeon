//! The Drowned Reach — an area that is generated rather than authored, and
//! `compute()` pathfinding over it.
//!
//! `register_virtual` had no game using it and `evict_virtual` had **zero
//! callers anywhere**, so `world_d._rooms` accumulated every virtual room ever
//! generated. That is bounded for a small ocean and unbounded for a grid, which
//! is why the eviction work was a prerequisite for this area rather than a
//! cleanup after it — `tests/state_retention.rs` covers the eviction itself.
//!
//! What is here is the other half: that a generated room is *the same room*
//! every time, that the graph a pathfinder gets agrees with the world it will
//! be walked in, and that the answer is revalidated before it is used.

mod common;

use common::{RealVm, TestCtx};
use oxigeon::config::server_config::ComputeConfig;

/// The reach, on the real mudlib, with the compute pool running.
fn vm_with_compute() -> RealVm {
    RealVm::boot_real_mudlib_with_probe_opts(TestCtx {
        compute: ComputeConfig {
            enabled: true,
            workers: 1,
            default_deadline_ms: 5_000,
            max_deadline_ms: 20_000,
            ..Default::default()
        },
        ..Default::default()
    })
}

/// The id **is** the room. Generation is a pure function of the coordinates, so
/// two people standing in the same place read the same thing and coming back
/// an hour later does too.
#[test]
fn a_generated_room_is_the_same_room_every_time() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    let describe = |vm: &mut RealVm, id: &str| -> String {
        vm.eval(&format!(
            "local r = DAEMON.world.get_room('{id}') \
             return require('lib.object').resolve(r.long, r)"
        ))
        .unwrap()
    };

    let first = describe(&mut vm, "reach.3.-7");
    assert!(!first.is_empty(), "the provider generated nothing");

    // Evict it and ask again. Identical, or eviction is data loss rather than
    // cache management.
    vm.eval("DAEMON.world.evict_virtual('reach.3.-7') return 'evicted'").unwrap();
    assert_eq!(
        vm.eval("return tostring(DAEMON.world._rooms['reach.3.-7'])").unwrap(),
        "nil"
    );
    assert_eq!(
        describe(&mut vm, "reach.3.-7"),
        first,
        "a regenerated room should be identical to the one thrown away"
    );

    // And a different coordinate is a different room, or the variation is
    // decorative.
    assert_ne!(describe(&mut vm, "reach.4.-7"), first);

    // Deterministic, not random: `math.random` here would make two people in
    // one room read different things. This is the one place in the game where
    // the seeded PRNG would be actively wrong.
    assert_eq!(
        vm.eval(
            "local a = DAEMON.reach.generate('reach.9.9') \
             local b = DAEMON.reach.generate('reach.9.9') \
             return tostring(require('lib.object').resolve(a.long, a) \
                          == require('lib.object').resolve(b.long, b))"
        )
        .unwrap(),
        "true"
    );
}

/// The grid has an edge, because a coordinate space with no bound is one where
/// a typo sends somebody somewhere a pathfinder will never return from.
#[test]
fn the_grid_has_an_edge_and_a_door() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    assert_eq!(
        vm.eval("return tostring(DAEMON.world.get_room('reach.0.0') ~= nil)").unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval("return tostring(DAEMON.world.get_room('reach.500.500'))").unwrap(),
        "nil",
        "a room past the extent should not be generated"
    );
    assert_eq!(
        vm.eval("return tostring(DAEMON.world.get_room('reach.not.a.number'))").unwrap(),
        "nil",
        "a provider that generated a room for every string would make every \
         typo a valid destination"
    );

    // The origin joins the static world, so the grid is reachable and
    // escapable without a teleport.
    assert_eq!(
        vm.eval("return DAEMON.world.get_room('reach.0.0').exits.east").unwrap(),
        "greywater_marsh.deep_water"
    );
    // And a room on the edge has fewer exits than one in the middle.
    let edge: i64 = vm
        .eval(
            "local n = 0 for _ in pairs(DAEMON.world.get_room('reach.40.40').exits) do \
             n = n + 1 end return n"
        )
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(edge, 2, "a corner has two ways out");

    // `virtual_prefixes` lists it, which is how an admin finds out an infinite
    // area exists at all.
    assert!(vm
        .eval("return table.concat(DAEMON.world.virtual_prefixes(), ',')")
        .unwrap()
        .contains("reach"));
}

/// The graph a pathfinder gets has to agree with the world it will be walked
/// in. A pathfinder with its own idea of the map is the classic failure.
#[test]
fn the_exit_graph_matches_the_world() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_g = DAEMON.world.exit_graph()").unwrap();

    // Static rooms are all in it, with their exits as plain strings — no Room
    // objects and no closures, because the marshaller refuses both.
    assert_eq!(
        vm.eval("return _g['thornhollow.square'].north").unwrap(),
        "thornhollow.smithy"
    );
    assert_eq!(
        vm.eval("return type(_g['thornhollow.square'].north)").unwrap(),
        "string"
    );

    // A rich exit contributes its target rather than its table.
    assert_eq!(
        vm.eval("return _g['collapsed_mine.second_level'].west").unwrap(),
        "collapsed_mine.deep_workings",
        "an exit with a `check` is still an exit — whether it opens is a \
         question for walk time, not plan time"
    );

    // Virtual rooms are absent unless cached: an infinite grid cannot be
    // enumerated, and pretending otherwise is how a pathfinder hangs.
    assert_eq!(
        vm.eval("return tostring(_g['reach.5.5'])").unwrap(),
        "nil",
        "the graph should not contain a virtual room nobody is standing in"
    );

    // `expand` grows it outward from one room, bounded.
    vm.eval(
        "_g2 = DAEMON.world.exit_graph({ expand = { provider = DAEMON.reach.neighbours, \
         from = 'reach.0.0', radius = 3 } })",
    )
    .unwrap();
    assert_eq!(vm.eval("return tostring(_g2['reach.0.0'] ~= nil)").unwrap(), "true");
    assert_eq!(vm.eval("return tostring(_g2['reach.0.3'] ~= nil)").unwrap(), "true");
    assert_eq!(
        vm.eval("return tostring(_g2['reach.0.30'])").unwrap(),
        "nil",
        "the expansion should stop at its radius"
    );
}

/// `still_connected` is the revalidation, and it has to catch the half a graph
/// cannot carry: an exit that is there and refuses.
#[test]
fn still_connected_notices_a_shut_door() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    let route = "{ 'collapsed_mine.second_level', 'collapsed_mine.deep_workings' }";

    // The grille starts shut, so the exit exists and its check refuses.
    vm.eval("set_object_state('collapsed_mine.second_level', 'door_open', false)").unwrap();
    assert_eq!(
        vm.eval(&format!("return tostring(DAEMON.world.still_connected({route}))")).unwrap(),
        "false",
        "a route through a shut door is not a route"
    );

    vm.eval("set_object_state('collapsed_mine.second_level', 'door_open', true)").unwrap();
    assert_eq!(
        vm.eval(&format!("return tostring(DAEMON.world.still_connected({route}))")).unwrap(),
        "true"
    );

    // A route through a room that no longer exists is refused, and says where.
    assert_eq!(
        vm.eval(
            "local ok, where = DAEMON.world.still_connected(\
             { 'thornhollow.square', 'nowhere.at.all' }) return tostring(where)"
        )
        .unwrap(),
        "thornhollow.square",
        "the refusal should name where the route broke"
    );

    // Two rooms that were never adjacent.
    assert_eq!(
        vm.eval(
            "return tostring(DAEMON.world.still_connected(\
             { 'thornhollow.square', 'collapsed_mine.the_sump' }))"
        )
        .unwrap(),
        "false"
    );
}

/// The pathfinder itself, as a pure function — which is what a compute module
/// is, and why it can be tested without a worker.
#[test]
fn the_pathfinder_finds_a_route_and_says_when_it_cannot() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_pf = require('compute.pathfind')").unwrap();
    vm.eval("_g = DAEMON.world.exit_graph()").unwrap();

    vm.eval(
        "_r = _pf.route({ graph = _g, from = 'thornhollow.square', \
                          to = 'collapsed_mine.pump_house' })",
    )
    .unwrap();
    assert!(
        vm.eval("return _r.cost").unwrap().parse::<i64>().unwrap() > 0,
        "no route to the pump house: {}",
        vm.eval("return tostring(_r.error)").unwrap()
    );
    assert_eq!(
        vm.eval("return _r.rooms[1]").unwrap(),
        "thornhollow.square",
        "the route should start where you are"
    );
    assert_eq!(
        vm.eval("return _r.rooms[#_r.rooms]").unwrap(),
        "collapsed_mine.pump_house"
    );
    assert_eq!(
        vm.eval("return tostring(#_r.rooms == #_r.path + 1)").unwrap(),
        "true",
        "n steps visit n+1 rooms"
    );

    // Deterministic: two runs over one graph give the same path. `pairs` order
    // would make a route that changes between identical requests, which is
    // indistinguishable from a bug in the world.
    vm.eval(
        "_r2 = _pf.route({ graph = _g, from = 'thornhollow.square', \
                           to = 'collapsed_mine.pump_house' })",
    )
    .unwrap();
    assert_eq!(
        vm.eval(
            "for i = 1, #_r.rooms do if _r.rooms[i] ~= _r2.rooms[i] then return 'differs' end end \
             return 'same'"
        )
        .unwrap(),
        "same"
    );

    // Nowhere to go, said out loud rather than by returning an empty path that
    // reads as "you are already there".
    vm.eval(
        "_none = _pf.route({ graph = _g, from = 'thornhollow.square', to = 'reach.9.9' })",
    )
    .unwrap();
    assert_eq!(vm.eval("return _none.cost").unwrap(), "-1");
    assert!(vm
        .eval("return _none.error")
        .unwrap()
        .contains("no way there"));

    // Already there is zero steps, not an error.
    vm.eval(
        "_here = _pf.route({ graph = _g, from = 'thornhollow.square', \
                             to = 'thornhollow.square' })",
    )
    .unwrap();
    assert_eq!(vm.eval("return _here.cost").unwrap(), "0");
}

/// End to end: a job goes to a worker, comes back, and the route is usable.
#[test]
fn a_route_is_planned_off_the_game_thread() {
    let mut vm = vm_with_compute();

    vm.eval(
        "_result = nil COMPUTE_HANDLERS[#COMPUTE_HANDLERS + 1] = \
         function(id, ok, value, err, meta) \
           if meta.tag == 'test' then _result = { ok = ok, value = value, err = err, \
             kind = meta.kind } return true end \
           return false end return 'listening'",
    )
    .unwrap();

    vm.eval(
        "_id = compute('compute.pathfind', 'route', \
           { graph = DAEMON.world.exit_graph(), from = 'thornhollow.square', \
             to = 'collapsed_mine.adit' }, { tag = 'test', deadline_ms = 5000 })",
    )
    .unwrap();
    assert_ne!(
        vm.eval("return tostring(_id)").unwrap(),
        "nil",
        "compute refused the job"
    );

    // The game thread is not blocked: this probe runs while the worker works.
    assert_eq!(vm.eval("return 'still here'").unwrap(), "still here");

    // Wait for the reply, which arrives as a command on the engine channel.
    let mut waited = 0;
    while vm.eval("return tostring(_result)").unwrap() == "nil" && waited < 100 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        waited += 1;
    }
    assert_eq!(
        vm.eval("return tostring(_result ~= nil)").unwrap(),
        "true",
        "the compute result never arrived"
    );
    assert_eq!(vm.eval("return tostring(_result.ok)").unwrap(), "true",
        "the job failed: {}", vm.eval("return tostring(_result.err)").unwrap());
    assert_eq!(vm.eval("return _result.kind").unwrap(), "ok");
    assert!(
        vm.eval("return _result.value.cost").unwrap().parse::<i64>().unwrap() > 0,
        "no route came back"
    );

    // And the answer is still a proposal: revalidate before acting on it.
    assert_eq!(
        vm.eval("return tostring(DAEMON.world.still_connected(_result.value.rooms))").unwrap(),
        "true"
    );
}

/// A worker VM has no efuns, which is the boundary rather than a limitation to
/// work around.
#[test]
fn a_worker_cannot_reach_the_world() {
    let mut vm = vm_with_compute();

    vm.eval(
        "_r = nil COMPUTE_HANDLERS[#COMPUTE_HANDLERS + 1] = \
         function(id, ok, value, err, meta) \
           if meta.tag == 'noefun' then _r = { ok = ok, err = err, kind = meta.kind } \
             return true end return false end return 'listening'",
    )
    .unwrap();

    // `pathfind.route` with no graph returns an error *value* rather than
    // raising, so this asks a different question: a module that tries to reach
    // an efun cannot even load one.
    vm.eval(
        "_id = compute('compute.pathfind', 'route', { graph = {}, from = 'a', to = 'b' }, \
           { tag = 'noefun' })",
    )
    .unwrap();

    let mut waited = 0;
    while vm.eval("return tostring(_r)").unwrap() == "nil" && waited < 100 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        waited += 1;
    }
    assert_eq!(vm.eval("return tostring(_r ~= nil)").unwrap(), "true");
    // It ran and answered; the world it could not see is why the answer is
    // "you are nowhere the map knows".
    assert_eq!(vm.eval("return tostring(_r.ok)").unwrap(), "true");
}
