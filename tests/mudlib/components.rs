//! The component index.
//!
//! `cmds/examine.lua` claimed from the day it was written that "a new component
//! describes itself by existing rather than by editing this file", directly
//! above four hard-coded `describe` calls. `components/init.lua` is what makes
//! that true, and these are the assertions that keep it true.
//!
//! The failure this guards is a quiet one: if discovery breaks — a bad
//! `list_dir` path, a module that stops looking like a component — `describe`
//! returns an empty list and `examine` simply stops mentioning that a sword
//! does damage. Nothing raises.

use crate::common::RealVm;

/// Every component the mudlib ships, in the order they should print.
const EXPECTED: [&str; 5] = ["weapon", "armour", "container", "drinkable", "requires"];

#[test]
fn the_index_discovers_every_component_in_the_directory() {
    let mut vm = RealVm::boot_fixture_with_probe();
    vm.eval("_c = require('components') return 'ok'").unwrap();

    let names = vm
        .eval("local out = {} for _, m in ipairs(_c.all()) do out[#out+1] = m.component end \
               return table.concat(out, ',')")
        .unwrap();

    assert_eq!(
        names,
        EXPECTED.join(","),
        "discovery should find every component, in declared `order`"
    );
}

#[test]
fn a_component_is_found_by_the_field_it_owns_not_its_filename() {
    // `armor.lua` owns `item.armour`. Keying the index on the filename would
    // have made that mismatch invisible until something looked it up.
    let mut vm = RealVm::boot_fixture_with_probe();
    vm.eval("_c = require('components') return 'ok'").unwrap();

    assert_eq!(vm.eval("return tostring(_c.get('armour') ~= nil)").unwrap(), "true");
    assert_eq!(
        vm.eval("return tostring(_c.get('armor'))").unwrap(),
        "nil",
        "the American spelling is the file, not the component"
    );
}

#[test]
fn describe_gathers_lines_from_every_component_on_an_item() {
    let mut vm = RealVm::boot_fixture_with_probe();
    vm.eval("_c = require('components') return 'ok'").unwrap();
    vm.eval(
        "local Weapon = require('components.weapon') \
         _sword = Weapon{ id = 'idx_sword', short = 'a test sword', slot = 'weapon', \
                          damage = { min = 3, max = 9 }, weapon_type = 'sword', \
                          required_strength = 40 } return 'ok'",
    )
    .unwrap();

    let lines = vm
        .eval("return table.concat(_c.describe(_sword, {}), '|')")
        .unwrap();

    assert!(lines.contains("Damage: 3-9"), "weapon component: {lines}");
    assert!(lines.contains("sword"), "weapon type: {lines}");
    // Two components on one item, and the requirement sorts last by `order`.
    assert!(lines.contains("Requires"), "requires component: {lines}");
    let damage_at = lines.find("Damage").unwrap();
    let requires_at = lines.find("Requires").unwrap();
    assert!(damage_at < requires_at, "order should be declared, not filesystem");
}

#[test]
fn a_requirement_is_coloured_by_whether_the_viewer_meets_it() {
    let mut vm = RealVm::boot_fixture_with_probe();
    vm.eval("_c = require('components') return 'ok'").unwrap();
    vm.eval(
        "local Weapon = require('components.weapon') \
         _heavy = Weapon{ id = 'idx_heavy', short = 'a heavy bar', slot = 'weapon', \
                          damage = { min = 1, max = 2 }, required_strength = 99 } \
         _weakling = { stats = { strength = 5 } } return 'ok'",
    )
    .unwrap();

    assert!(vm
        .eval("return table.concat(_c.describe(_heavy, { viewer = _weakling }), '|')")
        .unwrap()
        .contains("{red}"));

    // With nobody asking it still says what is needed — `examine` on a shop's
    // stock is a fair question with no viewer in it.
    let anonymous = vm.eval("return table.concat(_c.describe(_heavy, {}), '|')").unwrap();
    assert!(anonymous.contains("Requires"), "{anonymous}");
    assert!(!anonymous.contains("{red}"), "{anonymous}");
}

#[test]
fn a_container_describes_itself_from_its_instance_not_its_template() {
    // The one component that needs more than the item: whether a chest is open
    // is per-instance, so the index has to carry the instance id through.
    let mut vm = RealVm::boot_fixture_with_probe();
    vm.eval("_c = require('components') return 'ok'").unwrap();
    vm.eval(
        "local Container = require('components.container') \
         DAEMON.items.register(Container{ id = 'idx_box', short = 'a box', \
             capacity = 4, closeable = true, starts_closed = true }) return 'ok'",
    )
    .unwrap();
    vm.eval(
        "DAEMON.world.register_area({ DAEMON.room.from_data({ id = 'idx.room', exits = {} }) }) \
         _inst = DAEMON.items.spawn('idx_box', DAEMON.items.location('room', 'idx.room')) \
         return 'ok'",
    )
    .unwrap();

    let closed = vm
        .eval("local item = DAEMON.items.resolve(_inst) \
               return table.concat(_c.describe(item, { instance_id = _inst.id }), '|')")
        .unwrap();
    assert!(closed.contains("closed"), "{closed}");
}

#[test]
fn drinkable_describes_itself_although_examine_never_names_it() {
    // The point of the whole exercise. `drinkable` was never in `examine.lua`'s
    // hard-coded list, so a potion examined like anything else said nothing
    // about being drinkable. It says so now, and nothing was edited to allow
    // it beyond the component gaining a `describe`.
    let mut vm = RealVm::boot_fixture_with_probe();
    vm.eval("_c = require('components') return 'ok'").unwrap();
    vm.eval(
        "local Item = require('lib.item') \
         local drinkable = require('components.drinkable') \
         _flask = Item:new{ id = 'idx_flask', short = 'a flask' } \
         drinkable.apply(_flask, { drink_message = 'You sip it.' }) return 'ok'",
    )
    .unwrap();

    let lines = vm.eval("return table.concat(_c.describe(_flask, {}), '|')").unwrap();
    assert!(lines.contains("drink"), "a potion should mention it: {lines}");
}

#[test]
fn drinkable_is_a_namespaced_component_and_owns_its_wording() {
    let mut vm = RealVm::boot_fixture_with_probe();
    vm.eval(
        "local Item = require('lib.item') \
         _d = require('components.drinkable') \
         _flask = Item:new{ id = 'idx_flask2', short = 'a green flask' } \
         _d.apply(_flask, { drink_message = '{name} sips {short}.' }) return 'ok'",
    )
    .unwrap();

    // Data under one key, not five flat fields splatted onto the item.
    assert_eq!(vm.eval("return type(_flask.drinkable)").unwrap(), "table");
    assert_eq!(
        vm.eval("return tostring(_flask.drink_message)").unwrap(),
        "nil",
        "the old flat field should be gone, not shadowing the component"
    );
    assert_eq!(vm.eval("return tostring(_d.is(_flask))").unwrap(), "true");
    assert_eq!(vm.eval("return tostring(_d.is_consumed(_flask))").unwrap(), "true");

    // The wording lives with the component, so anything that can make a
    // character drink says it identically.
    vm.eval("_who = { name = 'Probe' } return 'ok'").unwrap();
    assert_eq!(
        vm.eval("local a = _d.messages(_flask, _who) return a").unwrap(),
        "Probe sips a green flask."
    );
}

#[test]
fn an_item_with_no_components_describes_nothing_rather_than_raising() {
    let mut vm = RealVm::boot_fixture_with_probe();
    vm.eval("_c = require('components') return 'ok'").unwrap();
    vm.eval("local Item = require('lib.item') \
             _rock = Item:new{ id = 'idx_rock', short = 'a rock' } return 'ok'")
        .unwrap();

    assert_eq!(vm.eval("return tostring(#_c.describe(_rock, {}))").unwrap(), "0");
    assert_eq!(vm.eval("return tostring(#_c.on(_rock))").unwrap(), "0");
    // And the degenerate inputs a command can actually reach it with.
    assert_eq!(vm.eval("return tostring(#_c.describe(nil, {}))").unwrap(), "0");
    assert_eq!(vm.eval("return tostring(#_c.describe(_rock))").unwrap(), "0");
}

#[test]
fn armour_contributes_its_own_equip_effects_through_the_index() {
    // The mitigation logic moved out of lib/equipment.lua into the component.
    // equipment.lua hands over the two effect factories rather than the
    // component reaching back for them.
    let mut vm = RealVm::boot_fixture_with_probe();
    vm.eval("_c = require('components') return 'ok'").unwrap();
    vm.eval(
        "local Armor = require('components.armor') \
         _jerkin = Armor{ id = 'idx_jerkin', short = 'a jerkin', slot = 'chest', \
                          defense = 3, resist = { fire = 2 }, \
                          stat_bonus = { strength = 1 } } return 'ok'",
    )
    .unwrap();

    let n: i64 = vm
        .eval(
            "_specs = _c.equip_specs(_jerkin, { \
               trait_effect = function(id) return 'stub_trait_' .. id end, \
               protection_effect = function() return 'stub_protection' end }) \
             return tostring(#_specs)",
        )
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(n, 2, "a stat bonus and a protection spec");

    assert!(vm
        .eval("local out = {} for _, s in ipairs(_specs) do out[#out+1] = s.def end \
               return table.concat(out, ',')")
        .unwrap()
        .contains("stub_protection"));
}

#[test]
fn a_component_that_raises_does_not_silence_the_others() {
    // One broken component must not take the whole of `examine` with it.
    let mut vm = RealVm::boot_fixture_with_probe();
    vm.eval("_c = require('components') return 'ok'").unwrap();
    vm.eval(
        // Break one component in place, the way a bad edit would.
        "local Weapon = require('components.weapon') \
         _sword = Weapon{ id = 'idx_sword2', short = 'a sword', slot = 'weapon', \
                          damage = { min = 2, max = 4 } } \
         local broken = _c.get('requires') \
         _real_describe = broken.describe \
         broken.describe = function() error('deliberately broken') end return 'ok'",
    )
    .unwrap();

    let lines = vm.eval("return table.concat(_c.describe(_sword, {}), '|')").unwrap();
    vm.eval("_c.get('requires').describe = _real_describe return 'ok'").unwrap();

    assert!(
        lines.contains("Damage"),
        "the weapon should still describe itself: {lines}"
    );
}

/// **A component's hand-written field survives being authored as data.**
///
/// `drinkable` declares `on_drink` as `hand_written`: it is a function, so
/// `from_data` cannot return it and it lives at the top level of the item. But
/// `Item:new` copies a *fixed list* of hooks — `on_use`, `on_pickup`, `on_drop`,
/// `on_equip`, `on_remove` — which naturally does not include it.
///
/// So `on_drink` reached an item only through the archetype path, where
/// `drinkable.apply` assigns it to an already-built object. The moment the same
/// potion was authored as flat data plus a `custom.lua` patch — which is what a
/// generated `items.lua` is — the hook was merged onto the data correctly and
/// then dropped during construction. The item was drinkable and did nothing,
/// with no error anywhere.
#[test]
fn a_components_hand_written_field_survives_construction_from_data() {
    let mut vm = RealVm::boot_fixture_with_probe();

    let out = vm
        .eval(
            "local C = require('components') \
             local item = C.build({ id = 'probe_potion', components = { 'drinkable' }, \
                 short = 'a probe potion', description = 'For testing.', \
                 drink_message = 'Down it goes.', \
                 on_drink = function(it, who) return 'drunk' end }) \
             return type(item.on_drink) .. '|' .. tostring(item.drinkable ~= nil) \
                 .. '|' .. tostring(item.drinkable.drink_message)",
        )
        .unwrap();

    assert_eq!(
        out, "function|true|Down it goes.",
        "the hand-written hook was dropped during construction"
    );

    // Driven off `hand_written`, not a list of hook names — so a component that
    // declares a new one needs no change here or in `lib/item.lua`.
    let declared = vm
        .eval(
            "local d = require('components.drinkable') \
             return table.concat(d.hand_written or {}, ',')",
        )
        .unwrap();
    assert_eq!(declared, "on_drink", "this test is asserting the wrong mechanism");
}
