//! The spell migration, and what it did not change.
//!
//! Casting moved onto `mudlib/daemons/ability_d.lua` and three of the four
//! spells lost all their Lua. The value of that is entirely in what stayed the
//! same: `cast` prints what it always printed, `DAEMON.spell.cast` returns what
//! it always returned, and the spells behave identically.
//!
//! `levelling.rs` and `traits_breadth.rs` are the other half of this assertion
//! and are deliberately **untouched** — they were written against the old
//! implementation and still pass against the new one.

use crate::common::RealVm;

/// The four spells are abilities now, and they are still spells.
#[test]
fn the_spells_are_abilities_in_the_spell_category() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    let out = vm
        .eval(
            "local ids = {} for _, e in ipairs(DAEMON.ability.known(\
             { char_id = 1, stats = {}, trait = function() return 99 end }, 'spell')) do \
             ids[#ids+1] = e.id end table.sort(ids) return table.concat(ids, ',')",
        )
        .unwrap();
    assert_eq!(out, "emberlance,farsight,mend,wardskin");

    // And `cleave` is a technique, not a spell — `category` is the lens.
    assert_eq!(
        vm.eval("return DAEMON.ability.get('cleave').category").unwrap(),
        "technique"
    );
}

/// Two of the four are pure data. That is the deliverable.
#[test]
fn emberlance_and_mend_carry_no_lua_at_all() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    for id in ["emberlance", "mend"] {
        let out = vm
            .eval(&format!(
                "local s = DAEMON.ability.get('{id}') \
                 return type(s.run) .. '|' .. type(s.on_complete) .. '|' .. type(s.cast)"
            ))
            .unwrap();
        assert_eq!(out, "nil|nil|nil", "'{id}' should be a data bag");
    }

    // `cleave` proves the whole surface with no code either.
    let out = vm
        .eval(
            "local s = DAEMON.ability.get('cleave') return type(s.run) .. '|' \
             .. tostring(s.rank_trait) .. '|' .. tostring(s.cast_time) .. '|' \
             .. tostring(s.cooldown.shared) .. '|' .. tostring(s.requires[1].kind)",
        )
        .unwrap();
    assert_eq!(out, "nil|swordsmanship|1|technique.heavy|equipped");
}

/// `cast` still prints exactly what it printed, and still gates on level.
#[test]
fn cast_is_unchanged_from_the_players_side() {
    let mut vm = RealVm::boot_real_mudlib(0);

    let known = vm.command("cast");
    assert!(known.contains("emberlance"), "{known}");
    assert!(known.contains("mana"), "the cost column survives:\n{known}");
    assert!(known.contains("Spell power:"), "the footer survives:\n{known}");
    assert!(!known.contains("wardskin"), "wardskin is level 3:\n{known}");

    // It still works, and it still refuses in the old words.
    let out = vm.command("cast emberlance at nothing");
    assert!(out.contains("There is no nothing here."), "{out}");
    let out = vm.command("cast nosuchspell");
    assert!(out.contains("You do not know any such thing."), "{out}");
}

/// `spell_d` projects the ability spec back to the shape it always exposed.
///
/// The legacy surface a game-layer caller reads: `cost` a bare mana number,
/// `cooldown` bare seconds. Two of the shipped tests parse exactly these out of
/// what `cast` prints.
#[test]
fn spell_d_still_exposes_the_shape_it_always_did() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    let out = vm
        .eval(
            "local s = DAEMON.spell.get('emberlance') \
             return tostring(s.cost) .. '|' .. tostring(s.cooldown) .. '|' \
                 .. tostring(s.level) .. '|' .. tostring(s.target) .. '|' .. tostring(s.name)",
        )
        .unwrap();
    assert_eq!(out, "8|4|1|creature|Emberlance");

    // And a non-spell is not visible through the spell vocabulary.
    assert_eq!(vm.eval("return tostring(DAEMON.spell.get('cleave'))").unwrap(), "nil");
}

/// `DAEMON.spell.cast` and `DAEMON.ability.use` are one call, so the two verbs
/// over them cannot disagree.
#[test]
fn cast_and_perform_take_the_same_path() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    let out = vm
        .eval(
            "DAEMON.ability._roll = function(n) return 1 end \
             _p = { char_id = 4242, id = 'p4242', name = 'P', stats = {}, equipment = {}, \
                    room_id = 'greywater_marsh.herb_beds' } \
             function _p:send(t) end function _p:message_room(t) end \
             setmetatable(_p, { __index = require('lib.mobile') }) \
             DAEMON.trait.seed(_p, 'character') \
             DAEMON.trait.set_cur(_p, 'mp', _p:trait('max_mp')) return 'ready'",
        )
        .unwrap();
    assert_eq!(out, "ready");

    // Emberlance goes through `resolve_attack` now, so it can miss — and a test
    // that compares two damage numbers from something that can miss is a test
    // that fails one morning for no reason. Pin the die low, which always hits;
    // the damage itself is `min == max` plus a fixed slope, so the comparison
    // below is exact rather than approximate.
    vm.eval("DAEMON.combat._roll = function(n) if n == 100 then return 1 else return n end end \
             return 'loaded'")
        .unwrap();

    let via_ability = vm
        .eval(
            "local m = DAEMON.mobs.spawn('reed_crawler', 'greywater_marsh.herb_beds') \
             local before = m:trait('hp') \
             DAEMON.ability.use(_p, 'emberlance', { target = m }) \
             return tostring(before - m:trait('hp'))",
        )
        .unwrap();

    // **The roundtime as well as the cooldown.** Emberlance costs a round now,
    // so without clearing `rt.combat` the second cast is *queued* rather than
    // resolved, comes back having dealt nothing, and this reads as a routing
    // bug — which is correct behaviour meeting a comparison that assumed
    // otherwise.
    let via_spell = vm
        .eval(
            "DAEMON.cooldown.clear(_p, 'spell.emberlance') \
             DAEMON.cooldown.clear(_p, 'rt.combat') \
             DAEMON.trait.set_cur(_p, 'mp', _p:trait('max_mp')) \
             local m = DAEMON.mobs.spawn('reed_crawler', 'greywater_marsh.herb_beds') \
             local before = m:trait('hp') DAEMON.spell.cast(_p, 'emberlance', m) \
             return tostring(before - m:trait('hp'))",
        )
        .unwrap();

    assert_ne!(via_ability, "0", "it should actually do something: {via_ability}");
    assert_eq!(via_ability, via_spell, "one call, two names for it");
}

/// `abilities` lists both categories, and says where a thing came from.
#[test]
fn the_abilities_command_lists_what_you_can_do() {
    let mut vm = RealVm::boot_real_mudlib(0);

    let out = vm.command("abilities");
    assert!(out.contains("spell"), "{out}");
    assert!(out.contains("emberlance"), "{out}");
    assert!(out.contains("mana"), "the cost column:\n{out}");

    // `cleave` needs `swordsmanship`, which nothing has taught yet — so it is
    // not listed. Presence decided by storage, all the way down.
    assert!(!out.contains("cleave"), "cleave is not known until taught:\n{out}");

    vm.command("affect learn swordsmanship 3");
    let out = vm.command("abilities");
    assert!(out.contains("cleave"), "learning the trait is learning the ability:\n{out}");
    assert!(out.contains("technique"), "and it lists under its own category:\n{out}");
}

/// An ability a wielded thing grants comes and goes with it.
#[test]
fn wielding_something_can_grant_an_ability() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval(
        "_g = { char_id = 4243, id = 'p4243', name = 'G', stats = {}, equipment = {} } \
         setmetatable(_g, { __index = require('lib.mobile') }) \
         DAEMON.trait.seed(_g, 'character') return 'ready'",
    )
    .unwrap();

    assert_eq!(
        vm.eval("return tostring(DAEMON.ability.knows(_g, 'cleave'))").unwrap(),
        "false",
        "nothing has taught it and nothing is granting it"
    );

    assert_eq!(
        vm.eval(
            "DAEMON.ability.set_source_abilities(_g, 'equip:weapon', { 'cleave' }) \
             return tostring(DAEMON.ability.knows(_g, 'cleave'))"
        )
        .unwrap(),
        "true",
        "a source grant is enough on its own, with no trait involved"
    );

    assert_eq!(
        vm.eval(
            "DAEMON.ability.set_source_abilities(_g, 'equip:weapon', {}) \
             return tostring(DAEMON.ability.knows(_g, 'cleave'))"
        )
        .unwrap(),
        "false",
        "and letting go takes it back, by the same reconciliation"
    );
}
