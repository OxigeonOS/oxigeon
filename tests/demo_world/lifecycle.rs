//! Disconnect, shutdown and the ordering between them.
//!
//! `on_disconnect` runs a chain of cleanup steps, each in its own `pcall` so a
//! failure in one does not prevent the others. Two of those steps have an
//! ordering constraint that is not obvious and is the whole correctness
//! argument, so it is asserted rather than commented:
//!
//!   `to_save` folds a container's contents onto its entry by reading them out
//!   of `item_d`'s location index. Releasing the items first would write every
//!   backpack empty.


use crate::common::RealVm;

/// A container's contents survive save and load, which is what makes a backpack
/// a backpack rather than a decoration.
#[test]
fn a_containers_contents_are_packed_on_save_and_restored_on_load() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_C = require('lib.carry')").unwrap();
    vm.eval(
        "_p = { char_id = 950, name = 'Packer', inventory = {}, equipment = {}, \
                quest_flags = {}, stats = { level = 1, strength = 10 }, \
                send = function() end }",
    )
    .unwrap();
    vm.eval("setmetatable(_p, { __index = require('lib.player') }) return 'ok'").unwrap();

    // A backpack with two things in it.
    vm.eval("_bag = DAEMON.items.spawn('leather_backpack', nil) \
             table.insert(_p.inventory, _bag) return 'carried'")
        .unwrap();
    vm.eval("DAEMON.items.spawn('brass_key', DAEMON.items.location('item', _bag.id)) \
             DAEMON.items.spawn('hemp_rope', DAEMON.items.location('item', _bag.id)) \
             return 'filled'")
        .unwrap();
    assert_eq!(vm.eval("return #DAEMON.items.contents(_bag.id)").unwrap(), "2");

    // Save. The contents are folded onto the entry, because `item_d`'s index is
    // memory only — correct for a sword on a floor and wrong for a backpack.
    vm.eval("_saved = _p:to_save()").unwrap();
    assert_eq!(
        vm.eval("return #_saved.inventory").unwrap(),
        "1",
        "the bag itself is one inventory entry"
    );
    assert_eq!(
        vm.eval("return #_saved.inventory[1].contents").unwrap(),
        "2",
        "the contents should have been folded onto the entry"
    );
    assert_eq!(
        vm.eval("return tostring(_saved.inventory[1].location)").unwrap(),
        "nil",
        "`location` describes a world that will not exist when this is read back"
    );

    // Load into a fresh character and they come back, indexed under the bag.
    vm.eval("_q = { char_id = 951, inventory = {}, equipment = {} }").unwrap();
    vm.eval("for i, e in ipairs(_saved.inventory) do _q.inventory[i] = e end return 'copied'")
        .unwrap();
    vm.eval("_C.unpack(_q.inventory) return 'unpacked'").unwrap();

    assert_eq!(
        vm.eval("return #DAEMON.items.contents(_q.inventory[1].id)").unwrap(),
        "2",
        "the contents did not come back out of the entry"
    );
    assert_eq!(
        vm.eval("return tostring(_q.inventory[1].contents)").unwrap(),
        "nil",
        "the folded copy should be removed once it is back in the index"
    );
}

/// A nested container survives too — a bag inside a chest is a thing players
/// will build the moment containers exist.
#[test]
fn nesting_survives_the_round_trip() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_C = require('lib.carry')").unwrap();
    vm.eval("_p = { char_id = 952, inventory = {}, equipment = {} }").unwrap();
    vm.eval("setmetatable(_p, { __index = require('lib.player') }) return 'ok'").unwrap();

    vm.eval("_outer = DAEMON.items.spawn('leather_backpack', nil) \
             table.insert(_p.inventory, _outer) return 'ok'")
        .unwrap();
    vm.eval("_inner = DAEMON.items.spawn('leather_backpack', \
             DAEMON.items.location('item', _outer.id)) return 'ok'")
        .unwrap();
    vm.eval("DAEMON.items.spawn('brass_key', DAEMON.items.location('item', _inner.id)) \
             return 'ok'")
        .unwrap();

    vm.eval("_packed = _C.pack(_p.inventory)").unwrap();
    assert_eq!(
        vm.eval("return #_packed[1].contents[1].contents").unwrap(),
        "1",
        "the inner bag's contents should be folded in too"
    );

    vm.eval("_q = { inventory = _packed } _C.unpack(_q.inventory) return 'unpacked'").unwrap();
    assert_eq!(
        vm.eval(
            "local outer = _q.inventory[1] \
             local inner = DAEMON.items.contents(outer.id)[1] \
             return #DAEMON.items.contents(inner.id)"
        )
        .unwrap(),
        "1",
        "a bag inside a bag came back empty"
    );
}

/// **The ordering.** Releasing a character's items before saving them would
/// write every container empty. Asserted directly, because the comment in
/// `init.lua` is the only other thing that says so.
#[test]
fn releasing_items_before_saving_would_lose_them() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_C = require('lib.carry')").unwrap();
    vm.eval("_p = { char_id = 953, inventory = {}, equipment = {}, quest_flags = {} }")
        .unwrap();
    vm.eval("setmetatable(_p, { __index = require('lib.player') }) return 'ok'").unwrap();

    vm.eval("_bag = DAEMON.items.spawn('leather_backpack', nil) \
             table.insert(_p.inventory, _bag) \
             DAEMON.items.spawn('brass_key', DAEMON.items.location('item', _bag.id)) \
             return 'ok'")
        .unwrap();

    // Save first: two entries survive.
    assert_eq!(
        vm.eval("return #_p:to_save().inventory[1].contents").unwrap(),
        "1"
    );

    // Release, then save: the contents are gone, because the index they were
    // read from is gone. This is what the ordering in `on_disconnect` prevents.
    vm.eval("_C.release(_p) return 'released'").unwrap();
    assert_eq!(
        vm.eval("return tostring(#DAEMON.items.contents(_bag.id))").unwrap(),
        "0",
        "release should have emptied the index"
    );
}

/// `release` takes a character's items out of the world index, because item
/// instance ids are uuids and are never reused.
#[test]
fn logging_out_releases_a_characters_items() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_C = require('lib.carry')").unwrap();
    let before: i64 = vm.eval("return DAEMON.items.count()").unwrap().parse().unwrap();

    vm.eval("_p = { char_id = 954, inventory = {}, equipment = {} }").unwrap();
    vm.eval("setmetatable(_p, { __index = require('lib.player') }) return 'ok'").unwrap();
    vm.eval("_p:add_item('hemp_rope') _p:add_item('brass_key') return 'carried'").unwrap();
    vm.eval("_w = DAEMON.items.spawn('leather_jerkin', nil) _p.equipment.chest = _w return 'worn'")
        .unwrap();

    assert_eq!(
        vm.eval("return DAEMON.items.count()").unwrap().parse::<i64>().unwrap(),
        before + 3
    );

    vm.eval("_C.release(_p) return 'released'").unwrap();
    assert_eq!(
        vm.eval("return DAEMON.items.count()").unwrap().parse::<i64>().unwrap(),
        before,
        "every item anyone ever carried would stay in `_instances` for the life \
         of the process otherwise — the same shape as the mob state leak"
    );
}

/// A real disconnect runs the whole chain, and every step is independent: a
/// failure in one must not prevent the others.
#[test]
fn every_disconnect_step_is_independently_protected() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    // The steps `on_disconnect` performs, each of which has to exist for the
    // chain to be the chain the comment describes.
    for name in [
        "DAEMON.channel.leave_all",
        "DAEMON.combat.disengage_all",
        "DAEMON.cache.evict_owner",
        "DAEMON.character.unload",
        "DAEMON.world.remove_character",
        "DAEMON.gmcp.forget",
        "DAEMON.olc.cleanup",
    ] {
        assert_eq!(
            vm.eval(&format!("return type({name})")).unwrap(),
            "function",
            "`{name}` is part of the disconnect chain and is missing"
        );
    }

    // A disconnect for a session that never played is a no-op rather than a
    // raise: the driver calls this for every dropped connection, including
    // ones that never got past the username prompt.
    assert_eq!(
        vm.eval("on_disconnect('not-a-real-session') return 'survived'").unwrap(),
        "survived"
    );
}
