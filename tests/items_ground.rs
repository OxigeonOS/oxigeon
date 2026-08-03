//! G1/G2/G5 — items that exist somewhere, and the verbs that move them.
//!
//! Before this there were **no ground items**. Items existed as templates in a
//! registry and as entries in `player.inventory`, and nothing in the mudlib
//! could put one on a floor: no `get`, no `drop`, no `put`, no `give`, no
//! `use`, no `examine`. Combat loot "went straight to the killer" because there
//! was nowhere else for it to go.
//!
//! `Item.on_pickup`, `on_drop` and `on_use` and the events `item.picked_up`,
//! `item.dropped` and `item.used` were all declared and none of them had ever
//! been called, for the same reason.
//!
//! Two harnesses, deliberately. `boot_real_mudlib` sends commands and is how a
//! player meets this; `boot_real_mudlib_with_probe` reaches the daemon and is
//! how the index, the cycle guard and the leak are checked. Both run the real
//! engine per `CLAUDE.md`.

mod common;

use common::RealVm;

// ═════════════════════════════════════════════════════════════════════════════
//  Through the daemon
// ═════════════════════════════════════════════════════════════════════════════

/// An instance has an id and a location, and the location index agrees with it.
#[test]
fn an_instance_knows_where_it_is() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_i = DAEMON.items.spawn('apprentice_dagger', DAEMON.items.location('room', 'a.b'))")
        .unwrap();
    assert_eq!(
        vm.eval("return tostring(_i ~= nil)").unwrap(),
        "true",
        "the gear file did not register, or spawn refused"
    );
    assert_eq!(vm.eval("return _i.template").unwrap(), "apprentice_dagger");
    assert_eq!(vm.eval("return _i.location").unwrap(), "room:a.b");

    // A uuid, not a counter: a container in somebody's inventory is saved, and
    // a counter restarting at zero on the next boot would collide with it.
    assert_eq!(
        vm.eval("return tostring(#_i.id > 20 and _i.id:sub(1,5) == 'item:')").unwrap(),
        "true",
        "expected a uuid-shaped instance id, got: {}",
        vm.eval("return _i.id").unwrap()
    );

    assert_eq!(vm.eval("return #DAEMON.items.in_room('a.b')").unwrap(), "1");
    assert_eq!(vm.eval("return #DAEMON.items.in_room('somewhere.else')").unwrap(), "0");

    // Two of them are two, and the room lists both.
    vm.eval("DAEMON.items.spawn('apprentice_dagger', DAEMON.items.location('room', 'a.b'))")
        .unwrap();
    assert_eq!(vm.eval("return #DAEMON.items.in_room('a.b')").unwrap(), "2");
}

/// Moving updates both sides of the index. An instance whose `location` field
/// and index entry disagree is findable in one place and not the other, which
/// is the bug this asserts against.
#[test]
fn moving_an_instance_reindexes_it() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_i = DAEMON.items.spawn('leather_jerkin', DAEMON.items.location('room', 'one.room'))")
        .unwrap();
    vm.eval("DAEMON.items.move(_i, DAEMON.items.location('room', 'two.room'))").unwrap();

    assert_eq!(vm.eval("return #DAEMON.items.in_room('one.room')").unwrap(), "0");
    assert_eq!(vm.eval("return #DAEMON.items.in_room('two.room')").unwrap(), "1");
    assert_eq!(vm.eval("return _i.location").unwrap(), "room:two.room");

    // nil takes it out of the world without destroying it — which is what
    // happens when a player picks it up.
    vm.eval("DAEMON.items.move(_i, nil)").unwrap();
    assert_eq!(vm.eval("return #DAEMON.items.in_room('two.room')").unwrap(), "0");
    assert_eq!(
        vm.eval("return tostring(DAEMON.items.get_instance(_i.id) ~= nil)").unwrap(),
        "true",
        "an item taken out of the index must still exist — a carried container's \
         contents are keyed on its id"
    );
}

/// A container cannot end up inside itself at any depth. `put bag in box` then
/// `put box in bag` is two legal-looking moves that between them make a cycle
/// nothing can ever reach again.
#[test]
fn a_container_cannot_contain_itself() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_bag = DAEMON.items.spawn('leather_backpack', nil)").unwrap();
    vm.eval("_box = DAEMON.items.spawn('iron_strongbox', nil)").unwrap();

    assert_eq!(
        vm.eval("return tostring(DAEMON.items.move(_bag, DAEMON.items.location('item', _bag.id)))")
            .unwrap(),
        "false",
        "a container went inside itself"
    );

    vm.eval("DAEMON.items.move(_bag, DAEMON.items.location('item', _box.id))").unwrap();
    assert_eq!(vm.eval("return #DAEMON.items.contents(_box.id)").unwrap(), "1");

    assert_eq!(
        vm.eval("return tostring(DAEMON.items.move(_box, DAEMON.items.location('item', _bag.id)))")
            .unwrap(),
        "false",
        "the box went inside the bag that is inside the box"
    );
}

/// Destroying a container destroys what is in it, and clears the object state
/// keyed on every id involved — the same lesson as the mob despawn leak.
#[test]
fn destroying_a_container_takes_its_contents_and_their_state() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    let count = |vm: &mut RealVm| -> i64 {
        vm.eval("return DAEMON.items.count()").unwrap().parse().unwrap()
    };
    let state_keys = |vm: &mut RealVm| -> i64 {
        vm.eval("local n = 0 for _ in pairs(_object_state_store) do n = n + 1 end return n")
            .unwrap()
            .parse()
            .unwrap()
    };

    let before_items = count(&mut vm);
    let before_state = state_keys(&mut vm);

    vm.eval("_box = DAEMON.items.spawn('iron_strongbox', nil)").unwrap();
    vm.eval("_key = DAEMON.items.spawn('brass_key', DAEMON.items.location('item', _box.id))")
        .unwrap();
    vm.eval("set_object_state(_box.id, 'closed', false)").unwrap();
    vm.eval("set_object_state(_key.id, 'bent', true)").unwrap();

    assert_eq!(count(&mut vm), before_items + 2);
    assert_eq!(state_keys(&mut vm), before_state + 2);

    vm.eval("_box_id = _box.id _key_id = _key.id DAEMON.items.destroy(_box)").unwrap();

    assert_eq!(count(&mut vm), before_items, "the contents survived the container");
    assert_eq!(
        state_keys(&mut vm),
        before_state,
        "object state outlived the items it belonged to"
    );
    assert_eq!(
        vm.eval("return tostring(DAEMON.items.get_instance(_key_id))").unwrap(),
        "nil"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
//  Through the commands, as a player meets them
// ═════════════════════════════════════════════════════════════════════════════

/// Spawn something onto the floor of the room the test character is in.
///
/// The template id is used as the name in both verbs, which is also a small
/// assertion in itself: `apprentice_dagger` and `apprentice dagger` have to
/// mean the same thing, or the id printed by `spawn` is not something a player
/// can type back.
fn drop_in_room(vm: &mut RealVm, template: &str) {
    vm.command(&format!("spawn {template}"));
    let out = vm.command(&format!("drop {template}"));
    assert!(out.contains("You drop"), "setup drop failed for {template}:\n{out}");
}

/// The round trip a player actually does.
#[test]
fn a_player_can_drop_and_take_an_item() {
    let mut vm = RealVm::boot_real_mudlib(0);

    vm.command("spawn apprentice_dagger");
    assert!(
        vm.command("inventory").contains("dagger"),
        "spawn should have put it in the inventory"
    );

    let out = vm.command("drop dagger");
    assert!(out.contains("You drop"), "drop said nothing:\n{out}");

    let look = vm.command("look");
    assert!(
        look.contains("Lying here") && look.contains("dagger"),
        "a dropped item should be visible in the room:\n{look}"
    );
    assert!(
        !vm.command("inventory").contains("dagger"),
        "it should have left the inventory"
    );

    let out = vm.command("get dagger");
    assert!(out.contains("You take"), "get said nothing:\n{out}");
    assert!(vm.command("inventory").contains("dagger"));
    assert!(
        !vm.command("look").contains("Lying here"),
        "the floor should be empty again"
    );
}

/// `drop lantern` must never mean the one already at your feet, however
/// reasonable a prefix match makes that look.
#[test]
fn drop_only_looks_at_what_you_are_carrying() {
    let mut vm = RealVm::boot_real_mudlib(0);

    drop_in_room(&mut vm, "apprentice_dagger");
    let out = vm.command("drop dagger");
    assert!(
        out.contains("not carrying"),
        "drop reached into the room:\n{out}"
    );
}

/// Containers: `put`, `get from`, and a capacity that refuses.
#[test]
fn a_container_holds_things_and_says_when_it_cannot() {
    let mut vm = RealVm::boot_real_mudlib(0);

    vm.command("spawn leather_backpack");
    vm.command("spawn brass_key");

    let out = vm.command("put key in backpack");
    assert!(out.contains("You put"), "put failed:\n{out}");
    assert!(
        !vm.command("inventory").contains("brass key"),
        "it should no longer be loose in the inventory"
    );

    let out = vm.command("examine backpack");
    assert!(
        out.contains("It contains") && out.contains("brass key"),
        "examine should list the contents:\n{out}"
    );

    let out = vm.command("get key from backpack");
    assert!(out.contains("You take"), "get from failed:\n{out}");
    assert!(vm.command("inventory").contains("brass key"));

    // A closed, locked container refuses. `iron_strongbox` starts both.
    vm.command("spawn iron_strongbox");
    let out = vm.command("put key in strongbox");
    assert!(
        out.contains("closed") || out.contains("locked"),
        "a locked strongbox should refuse:\n{out}"
    );
}

/// The hooks and events that had never once fired.
#[test]
fn pickup_and_drop_fire_their_hooks_and_events() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    // A template with hooks, registered at runtime the way an area file does.
    vm.eval(
        "_fired = {}; DAEMON.items.register(require('lib.item'):new({ \
           id = 'hooked_thing', short = 'a hooked thing', \
           on_pickup = function(_, who) _fired.pickup = who end, \
           on_drop   = function(_, who) _fired.drop = who end, \
           on_use    = function(_, who) _fired.use = who return 'It hums.' end }))",
    )
    .unwrap();

    vm.eval(
        "DAEMON.event.on('item.picked_up', 'test', function(d) _fired.ev_pickup = d.template_id end) \
         DAEMON.event.on('item.dropped', 'test', function(d) _fired.ev_drop = d.template_id end) \
         return 'listening'",
    )
    .unwrap();

    // A player-shaped table is enough: `Carry` wants `char_id`, `inventory`
    // and a room, and going through the real Player would need a login.
    vm.eval(
        "_p = { char_id = 99, inventory = {}, name = 'Tester', \
                send = function() end, message_room = function() end }",
    )
    .unwrap();
    vm.eval("DAEMON.world.register_area({ DAEMON.room.from_data({ id = 'hook.room', exits = {} }) })")
        .unwrap();
    vm.eval("DAEMON.world.place_character(99, 'hook.room')").unwrap();

    vm.eval("_C = require('lib.carry')").unwrap();
    vm.eval("_i = DAEMON.items.spawn('hooked_thing', DAEMON.items.location('room', 'hook.room'))")
        .unwrap();
    vm.eval("_C.take(_p, _i, DAEMON.items.resolve(_i))").unwrap();

    assert_eq!(
        vm.eval("return tostring(_fired.pickup)").unwrap(),
        "99",
        "Item.on_pickup did not fire — it never had before this change"
    );
    assert_eq!(
        vm.eval("return tostring(_fired.ev_pickup)").unwrap(),
        "hooked_thing",
        "the documented item.picked_up event did not fire"
    );

    vm.eval("_C.drop(_p, _i, DAEMON.items.resolve(_i))").unwrap();
    assert_eq!(vm.eval("return tostring(_fired.drop)").unwrap(), "99");
    assert_eq!(vm.eval("return tostring(_fired.ev_drop)").unwrap(), "hooked_thing");
}

/// Loot lands on the floor rather than in the killer's pack. That was never a
/// design decision — there was nowhere else for it to go.
#[test]
fn combat_loot_falls_to_the_floor() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("DAEMON.world.register_area({ DAEMON.room.from_data({ id = 'loot.room', exits = {} }) })")
        .unwrap();
    vm.eval(
        "DAEMON.mobs.register({ id = 'loot_rat', short = 'a rat', \
           stats = { hp = 1, max_hp = 1, level = 1 }, xp_award = 1, \
           loot_table = { { item_id = 'brass_key', chance = 1.0 } } })",
    )
    .unwrap();

    assert_eq!(vm.eval("return #DAEMON.items.in_room('loot.room')").unwrap(), "0");

    vm.eval("_rat = DAEMON.mobs.spawn('loot_rat', 'loot.room')").unwrap();
    vm.eval(
        "_killer = { char_id = 5, name = 'Killer', inventory = {}, \
                     award_xp = function() return 0 end, \
                     is_alive = function() return true end, \
                     send = function() end }",
    )
    .unwrap();
    // Pin the dice. `_roll` is overridable precisely so a test is deterministic
    // by choice rather than by the PRNG happening not to be seeded — which it
    // now is, so this matters more than it used to. 1 always hits and always
    // rolls minimum damage; the rat has one hit point.
    vm.eval("_real_roll = DAEMON.combat._roll; DAEMON.combat._roll = function() return 1 end")
        .unwrap();
    vm.eval("DAEMON.combat.engage(_killer, _rat)").unwrap();
    vm.eval("DAEMON.combat.round() return 'rounded'").unwrap();
    vm.eval("DAEMON.combat._roll = _real_roll return 'restored'").unwrap();

    assert_eq!(
        vm.eval("return tostring(_rat:is_alive())").unwrap(),
        "false",
        "the round did not kill the rat, so nothing can have dropped"
    );

    let ground: i64 = vm
        .eval("return #DAEMON.items.in_room('loot.room')")
        .unwrap()
        .parse()
        .unwrap();
    let carried: i64 = vm.eval("return #_killer.inventory").unwrap().parse().unwrap();

    assert_eq!(
        (ground, carried),
        (1, 0),
        "loot should fall in the room, not go straight to the killer"
    );
}
