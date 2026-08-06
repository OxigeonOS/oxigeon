//! `lib/schema.lua` — the one written-down description of what an authorable
//! thing is.
//!
//! Every OLC defect traced back to its absence: `codegen_d` hardcoded five room
//! fields and re-emitted from that list, so `light`, `smell`, `sound` and `tags`
//! were destroyed on the next `dig`; `olc set` could not exist because there was
//! nothing to enumerate; `adopt` could not report what it would lose because
//! nothing knew what "lose" meant.
//!
//! Through the fixture world rather than `game/`: these are claims about the
//! mudlib's schema, and they must keep meaning something for somebody who
//! deleted the demo content.

mod common;

use common::RealVm;

fn one_line(src: &str) -> String {
    src.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

struct Vm(RealVm);

impl Vm {
    fn new() -> Self {
        let mut vm = RealVm::boot_fixture_with_probe();
        vm.eval("SC = require('lib.schema') CO = require('components') return 'ready'")
            .unwrap();
        Self(vm)
    }
    fn run(&mut self, src: &str) -> String {
        self.0.eval(&one_line(src)).unwrap()
    }
}

/// The three kinds exist and are discovered, not listed.
#[test]
fn the_authorable_kinds_are_discovered() {
    let mut vm = Vm::new();
    assert_eq!(
        vm.run("local k = SC.kinds() table.sort(k) return table.concat(k, ',')"),
        "item,mob,room"
    );
}

/// A component describes itself by existing.
///
/// This is the test that keeps the no-central-registry promise honest. A new
/// component that forgets its schema fails *here*, rather than silently becoming
/// un-authorable somewhere a builder finds out about it — which is exactly the
/// failure mode a central `applies_to` list would have had, in a different
/// place. `CLAUDE.md` gives the reasoning under the trait rules.
#[test]
fn every_component_declares_its_fields_and_its_inverse() {
    let mut vm = Vm::new();

    let missing = vm.run(
        r#"
        local bad = {}
        for _, mod in ipairs(CO.all()) do
            if type(mod.fields) ~= 'table' then
                bad[#bad+1] = mod.component .. ' has no fields'
            end
            if type(mod.to_data) ~= 'function' then
                bad[#bad+1] = mod.component .. ' has no to_data'
            end
            if type(mod.from_data) ~= 'function' then
                bad[#bad+1] = mod.component .. ' has no from_data'
            end
            for _, f in ipairs(mod.fields or {}) do
                if type(f.name) ~= 'string' or type(f.type) ~= 'string' then
                    bad[#bad+1] = mod.component .. ' has a malformed descriptor'
                end
            end
        end
        table.sort(bad) return table.concat(bad, '; ')
    "#,
    );
    assert_eq!(missing, "");
}

/// Component fields flatten into the item schema, in component order.
#[test]
fn a_components_fields_join_the_item_schema() {
    let mut vm = Vm::new();

    // A plain item has none of them.
    let plain = vm.run(
        "local n = {} for _, f in ipairs(SC.fields_for('item', { id = 'x' })) do \
         n[#n+1] = f.name end return table.concat(n, ',')",
    );
    assert!(!plain.contains("damage"), "a plain item should have no weapon fields: {plain}");

    // Declaring the component brings them in.
    let armed = vm.run(
        "local n = {} \
         for _, f in ipairs(SC.fields_for('item', { id = 'x', components = { 'weapon' } })) do \
           n[#n+1] = f.name end \
         return table.concat(n, ',')",
    );
    assert!(armed.contains("damage"), "{armed}");
    assert!(armed.contains("speed"), "{armed}");
    // The item's own fields come first, then the component block.
    let short = armed.find("short").unwrap();
    let damage = armed.find("damage").unwrap();
    assert!(short < damage, "kind fields should precede component fields: {armed}");
}

/// `requires` is implicit: `required_strength = 16` on a sword has always meant
/// the component, and making a builder also declare it would be a second way to
/// say one thing.
#[test]
fn an_implicit_component_is_claimed_by_its_own_fields() {
    let mut vm = Vm::new();

    let claimed = vm.run(
        "local n = {} \
         for _, c in ipairs(CO.claimed({ id = 'x', required_strength = 16 })) do \
           n[#n+1] = c.component end \
         return table.concat(n, ',')",
    );
    assert_eq!(claimed, "requires");

    // …and every other component stays explicit, so a stray `speed` cannot
    // silently weaponise a lantern.
    let inferred = vm.run(
        "local n = {} \
         for _, c in ipairs(CO.claimed({ id = 'lantern', speed = 1.1 })) do \
           n[#n+1] = c.component end \
         return table.concat(n, ',')",
    );
    assert_eq!(inferred, "", "a stray field must not claim a component: {inferred}");
}

/// Flat data in, a built Item out — and back again.
///
/// P3: `build` composes with `to_data` as an inverse. `Weapon{...}` is a one-way
/// function, which is why the flat form is the interchange format.
#[test]
fn flat_data_builds_an_item_and_survives_the_return_trip() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        local data = {
            id = 'bone_saw', short = 'a corroded bone saw', slot = 'weapon',
            weight = 4, value = 12, tags = { 'weapon' },
            components = { 'weapon' },
            damage = { min = 3, max = 7 }, speed = 0.8,
            weapon_type = 'saw', damage_type = 'physical',
            required_strength = 12,
        }
        local item, err = CO.build(data)
        if not item then return 'build failed: ' .. tostring(err) end
        if type(item.weapon) ~= 'table' then return 'no weapon component' end
        if item.weapon.min ~= 3 or item.weapon.max ~= 7 then return 'damage lost' end
        if type(item.requires) ~= 'table' then return 'requires not implied' end
        if item.requires.strength ~= 12 then return 'requirement lost' end
        if item.display_name == nil then return 'not a real Item' end

        local back, lossy = CO.to_data(item)
        if #lossy ~= 0 then return 'unexpectedly lossy' end
        if back.damage.min ~= 3 or back.damage.max ~= 7 then return 'damage did not return' end
        if back.speed ~= 0.8 then return 'speed did not return' end
        if back.required_strength ~= 12 then return 'requirement did not return' end
        local names = table.concat(back.components or {}, ',')
        if names ~= 'weapon' then return 'components wrong: ' .. names end
        return 'ok'
    "#,
    );
    assert_eq!(out, "ok");
}

/// The archetype and the loader agree about what a weapon defaults to.
///
/// `weapon.new` set `slot = "weapon"` inline; `components.build` reads
/// `M.item_defaults`. Two places deciding one thing is how they come to disagree.
#[test]
fn the_archetype_and_the_loader_build_the_same_item() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        local flat = { id = 'blade', short = 'a blade', damage = { min = 2, max = 5 },
                       speed = 1.1, components = { 'weapon' } }
        local via_loader = CO.build(flat)
        local Weapon = require('components.weapon')
        local via_archetype = Weapon{ id = 'blade', short = 'a blade',
                                      damage = { min = 2, max = 5 }, speed = 1.1 }
        if via_loader.slot ~= via_archetype.slot then
            return 'slot differs: ' .. tostring(via_loader.slot) .. ' vs ' .. tostring(via_archetype.slot)
        end
        for k, v in pairs(via_archetype.weapon) do
            if via_loader.weapon[k] ~= v then return 'weapon.' .. k .. ' differs' end
        end
        return 'ok'
    "#,
    );
    assert_eq!(out, "ok");
}

/// `M.set` is the only string-to-value converter, and it refuses by type.
#[test]
fn set_coerces_and_refuses_by_type() {
    let mut vm = Vm::new();

    // Booleans: exactly eight spellings, and nothing is truthy-by-default.
    for word in ["true", "yes", "on", "1"] {
        assert_eq!(
            vm.run(&format!(
                "local d = {{ id = 'x' }} local ok = SC.set('mob', d, 'aggressive', '{word}') \
                 return tostring(ok) .. '|' .. tostring(d.aggressive)"
            )),
            "true|true"
        );
    }
    for word in ["false", "no", "off", "0"] {
        assert_eq!(
            vm.run(&format!(
                "local d = {{ id = 'x' }} local ok = SC.set('mob', d, 'aggressive', '{word}') \
                 return tostring(ok) .. '|' .. tostring(d.aggressive)"
            )),
            "true|false"
        );
    }
    let out = vm.run(
        "local d = { id = 'x' } local ok, err = SC.set('mob', d, 'aggressive', 'maybe') \
         return tostring(ok) .. '|' .. tostring(err)",
    );
    assert!(out.starts_with("false|"), "{out}");
    assert!(out.contains("true false yes no on off 1 0"), "the error should list them: {out}");

    // Enum: the error names what is allowed.
    let out = vm.run(
        "local d = { id = 'x', components = { 'weapon' } } \
         local ok, err = SC.set('item', d, 'range', 'lobbed') \
         return tostring(ok) .. '|' .. tostring(err)",
    );
    assert!(out.contains("melee"), "{out}");

    // Range: `3-7`, or a single number for a fixed one.
    assert_eq!(
        vm.run(
            "local d = { id = 'x' } SC.set('mob', d, 'damage', '3-7') \
             return d.damage.min .. '-' .. d.damage.max"
        ),
        "3-7"
    );
    assert_eq!(
        vm.run(
            "local d = { id = 'x' } SC.set('mob', d, 'damage', '5') \
             return d.damage.min .. '-' .. d.damage.max"
        ),
        "5-5"
    );

    // Bounds are enforced at input, not at write time.
    let out = vm.run(
        "local d = { id = 'x' } local ok, err = SC.set('room', d, 'light', '12') \
         return tostring(ok) .. '|' .. tostring(err)",
    );
    assert!(out.starts_with("false|") && out.contains("maximum"), "{out}");

    // An unknown field is refused rather than quietly stored.
    let out = vm.run(
        "local d = { id = 'x' } local ok, err = SC.set('room', d, 'sparkle', 'yes') \
         return tostring(ok) .. '|' .. tostring(err)",
    );
    assert!(out.starts_with("false|") && out.contains("no field"), "{out}");
}

/// Map keys must be keywords, because codegen writes them bare and `examine`
/// and `talk` match on them.
#[test]
fn a_map_key_must_be_a_keyword() {
    let mut vm = Vm::new();

    assert_eq!(
        vm.run(
            "local d = { id = 'x' } SC.set('room', d, 'items.well', 'A stone well.') \
             return d.items.well"
        ),
        "A stone well."
    );

    let out = vm.run(
        "local d = { id = 'x' } \
         local ok, err = SC.set('mob', d, 'dialogue.the wall', 'Nothing.') \
         return tostring(ok) .. '|' .. tostring(err)",
    );
    assert!(out.starts_with("false|"), "{out}");
    assert!(out.contains("keyword"), "{out}");
}

/// An lfun field is refused at input, and the message points at `custom.lua`.
///
/// Refusing without saying where the value *should* go leaves a builder stuck,
/// and stuck is when people start editing generated files by hand.
#[test]
fn an_lfun_field_is_refused_and_names_custom_lua() {
    let mut vm = Vm::new();

    let out = vm.run(
        "local d = { id = 'x', components = { 'weapon' } } \
         local ok, err = SC.set('item', d, 'hit_message', 'You saw into {target}.') \
         return tostring(ok) .. '|' .. tostring(err)",
    );
    assert!(out.starts_with("false|"), "{out}");
    assert!(out.contains("custom.lua"), "the refusal should say where it goes: {out}");
}

/// The grammar's keywords are not field names, checked rather than assumed.
#[test]
fn no_field_collides_with_a_grammar_keyword() {
    let mut vm = Vm::new();

    let clashes = vm.run(
        r#"
        local bad = {}
        for _, kind in ipairs(SC.kinds()) do
            for _, f in ipairs(SC.fields_for(kind, { id = 'x', components = CO.names() })) do
                if SC.RESERVED[f.name] then bad[#bad+1] = kind .. '.' .. f.name end
            end
        end
        table.sort(bad) return table.concat(bad, ',')
    "#,
    );
    assert_eq!(clashes, "", "a field name collides with an OLC keyword: {clashes}");
}

/// `lossy` and `unknown` are the whole of adoption's detection.
#[test]
fn lossy_and_unknown_separate_what_moves_from_what_is_kept() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        local data = {
            id = 'crypt.hall', short = 'The Hall',
            description = function() return 'computed' end,
            actions = { pull = { func = function() end } },
            puzzle_seed = 17,
        }
        local lossy = SC.lossy('room', data)
        local names = {}
        for _, l in ipairs(lossy) do names[#names+1] = l.path .. '=' .. l.why end
        table.sort(names)
        local unknown = table.concat(SC.unknown('room', data), ',')
        return table.concat(names, ' ') .. ' || ' .. unknown
    "#,
    );

    // A function-valued lfun and a hand-written field both move.
    assert!(out.contains("actions=hand-written"), "{out}");
    assert!(out.contains("description=function"), "{out}");
    // A serializable field no schema names is KEPT, and reported — dropping it
    // silently is the bug class this design exists to end.
    assert!(out.contains("|| puzzle_seed"), "{out}");
    assert!(!out.contains("puzzle_seed=") , "an unknown but writable field is not lossy: {out}");
}

/// Emit order is schema order, then leftovers sorted — and a room's exits come
/// out in compass order rather than alphabetically.
#[test]
fn the_orderer_puts_schema_fields_first_and_exits_in_compass_order() {
    let mut vm = Vm::new();

    let keys = vm.run(
        "local o = SC.orderer('room') \
         return table.concat(o({ description = 'd', short = 's', id = 'i', zzz = 1 }, ''), ',')",
    );
    assert!(keys.starts_with("id,short,description"), "{keys}");
    assert!(keys.ends_with("zzz"), "leftovers go last: {keys}");

    let exits = vm.run(
        "local o = SC.orderer('room') \
         return table.concat(o({ west = 'a', north = 'b', down = 'c', east = 'd' }, 'exits'), ',')",
    );
    assert_eq!(exits, "north,east,west,down");
}

/// One direction list, not three.
///
/// `dig` had a private `REVERSE` with no entry for `in` or `out`, so digging
/// either way created a one-way passage and said nothing — while
/// `docs/src/lua-api/olc.md` claimed it used `movement.lua`'s table all along.
#[test]
fn every_direction_has_an_opposite_and_a_command() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        local m = require('lib.movement')
        local bad = {}
        for _, dir in ipairs(m.ORDER) do
            if not m.OPPOSITES[dir] then bad[#bad+1] = dir .. ' has no opposite' end
        end
        for dir in pairs(m.OPPOSITES) do
            local found = false
            for _, d in ipairs(m.ORDER) do if d == dir then found = true end end
            if not found then bad[#bad+1] = dir .. ' is not in ORDER' end
        end
        for short, long in pairs(m.ABBREVIATIONS) do
            if not m.OPPOSITES[long] then bad[#bad+1] = short .. ' expands to a non-direction' end
        end
        table.sort(bad) return table.concat(bad, '; ')
    "#,
    );
    assert_eq!(out, "");

    // And `in`/`out` really do round-trip, which the private table got wrong.
    assert_eq!(vm.run("return require('lib.movement').OPPOSITES['in']"), "out");
    assert_eq!(vm.run("return require('lib.movement').expand('nw')"), "northwest");
    assert_eq!(vm.run("return tostring(require('lib.movement').expand('i'))"), "nil");
}

/// `defaults` gives `olc new` something to start from.
#[test]
fn defaults_produce_a_usable_starting_point() {
    let mut vm = Vm::new();

    assert_eq!(
        vm.run("local d = SC.defaults('room') return d.light .. '|' .. d.short"),
        "2|A Room"
    );

    let out = vm.run(
        "local d = SC.defaults('item', 'weapon') \
         return table.concat(d.components, ',') .. '|' .. tostring(d.speed) \
                .. '|' .. tostring(d.damage_type)",
    );
    assert_eq!(out, "weapon|1.0|physical");
}
