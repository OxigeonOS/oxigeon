//! Trait breadth — every documented feature with something using it.
//!
//! A feature with no user is a feature nobody has checked. `round` had four
//! modes and one in use; `hidden` had no trait; there was one gauge with a
//! trait-valued maximum and no second one; there was no derived-of-derived
//! chain anywhere; and `seal`'s cycle-as-a-path message had never been produced
//! by a real file.


use crate::common::RealVm;

/// Derived-of-derived: `spell_power` reads `willpower`, which is itself
/// derived. `seal` has to order all three, and a change at the bottom has to
/// reach the top.
#[test]
fn a_two_level_dependency_chain_resolves_and_propagates() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_c = { stats = {} } DAEMON.trait.seed(_c, 'character') return 'seeded'").unwrap();

    // wisdom 10, level 1 -> willpower 0 -> spell_power 5 + 0 = 5.
    assert_eq!(vm.eval("return _c.stats.wisdom").unwrap(), "10");
    assert_eq!(vm.eval("return DAEMON.trait.value(_c, 'willpower')").unwrap(), "0");
    assert_eq!(vm.eval("return DAEMON.trait.value(_c, 'spell_power')").unwrap(), "5");

    // Raise the bottom of the chain and the top moves.
    vm.eval("DAEMON.trait.set_base(_c, 'wisdom', 20) return 'ok'").unwrap();
    assert_eq!(
        vm.eval("return DAEMON.trait.value(_c, 'willpower')").unwrap(),
        "5",
        "(20-10)/2 + 1/2 = 5"
    );
    assert_eq!(
        vm.eval("return DAEMON.trait.value(_c, 'spell_power')").unwrap(),
        "10",
        "a change two levels down did not reach the top"
    );

    // And the evaluation order really is topological: `willpower` must have a
    // lower rank than `spell_power`, or the second would read a stale first.
    assert_eq!(
        vm.eval(
            "return tostring(DAEMON.trait.get_def('willpower').rank \
                           < DAEMON.trait.get_def('spell_power').rank)"
        )
        .unwrap(),
        "true"
    );
}

/// A gauge whose maximum is a *derived-of-derived* trait. `max_stamina` reads
/// `carry_capacity`, which reads `strength`, so `seal` folds the bound into a
/// three-deep graph.
#[test]
fn a_gauge_bound_can_be_a_derived_of_derived_trait() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_c = { stats = {} } DAEMON.trait.seed(_c, 'character') return 'seeded'").unwrap();

    // strength 10 -> carry_capacity 100 -> max_stamina 40 + 30 + 10 = 80.
    assert_eq!(vm.eval("return DAEMON.trait.value(_c, 'carry_capacity')").unwrap(), "100");
    assert_eq!(vm.eval("return DAEMON.trait.value(_c, 'max_stamina')").unwrap(), "80");

    // The gauge clamps to it.
    vm.eval("DAEMON.trait.set_cur(_c, 'stamina', 500) return 'ok'").unwrap();
    assert_eq!(vm.eval("return DAEMON.trait.value(_c, 'stamina')").unwrap(), "80");

    // Raising strength raises the ceiling through two derivations.
    vm.eval("DAEMON.trait.set_base(_c, 'strength', 20) return 'ok'").unwrap();
    assert_eq!(vm.eval("return DAEMON.trait.value(_c, 'carry_capacity')").unwrap(), "180");
    assert_eq!(vm.eval("return DAEMON.trait.value(_c, 'max_stamina')").unwrap(), "88");

    // The bound is in the dependency graph, which is what `seal` folds it in
    // for: `max_stamina` must rank before `stamina`.
    assert_eq!(
        vm.eval(
            "return tostring(DAEMON.trait.get_def('max_stamina').rank \
                           < DAEMON.trait.get_def('stamina').rank)"
        )
        .unwrap(),
        "true"
    );
}

/// All four `round` modes, on four traits that differ only in rounding.
#[test]
fn every_rounding_mode_is_used_and_they_differ() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_c = { stats = {} } DAEMON.trait.seed(_c, 'character') return 'seeded'").unwrap();

    // dexterity 10 + perception 10 = 20 / 3 = 6.666...
    assert_eq!(
        vm.eval("return DAEMON.trait.value(_c, 'reflex')").unwrap(),
        "6",
        "floor"
    );
    // wisdom 10 + level 1 = 11 / 3 = 3.666...
    assert_eq!(
        vm.eval("return DAEMON.trait.value(_c, 'resolve')").unwrap(),
        "4",
        "ceil"
    );
    // charisma 10 + level 1 = 11 / 3 = 3.666... -> 4
    assert_eq!(
        vm.eval("return DAEMON.trait.value(_c, 'presence')").unwrap(),
        "4",
        "round"
    );
    // wisdom 10 + intelligence 10 = 20 / 4 = 5, exactly — so pick a value that
    // is not exact to prove `none` really does not round.
    vm.eval("DAEMON.trait.set_base(_c, 'wisdom', 11) return 'ok'").unwrap();
    assert_eq!(
        vm.eval("return DAEMON.trait.value(_c, 'attunement')").unwrap(),
        "5.25",
        "`round = none` should keep the fraction"
    );

    // And the four really are declared differently, rather than agreeing by
    // accident on these numbers.
    for (id, mode) in [
        ("reflex", "floor"),
        ("resolve", "ceil"),
        ("presence", "round"),
        ("attunement", "none"),
    ] {
        assert_eq!(
            vm.eval(&format!("return DAEMON.trait.get_def('{id}').round")).unwrap(),
            mode
        );
    }
}

/// `hidden` is computed and present and never shown.
#[test]
fn a_hidden_trait_is_present_and_invisible() {
    let mut vm = RealVm::boot_real_mudlib(0);

    let score = vm.command("score");
    assert!(
        !score.to_lowercase().contains("luck"),
        "a hidden trait leaked into score:\n{score}"
    );

    // It is there, though — present, computed and clamped.
    let dump = vm.command("affect traits");
    assert!(
        dump.contains("luck_seed"),
        "the admin dump should show a hidden trait:\n{dump}"
    );
    assert!(
        vm.command("traits").contains("hidden"),
        "`traits` should mark it as hidden rather than hiding it"
    );
}

/// A gauge with `offline = true` earns while you are away; one with
/// `offline = false` does not. Having one of each is what proves the flag is
/// read rather than assumed.
#[test]
fn offline_regeneration_is_per_gauge() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    assert_eq!(
        vm.eval("return tostring(DAEMON.trait.get_def('hp').regen.offline)").unwrap(),
        "false"
    );
    assert_eq!(
        vm.eval("return tostring(DAEMON.trait.get_def('stamina').regen.offline)").unwrap(),
        "true"
    );

    // `attach` re-anchors the offline = false gauges and leaves the others, so
    // three days away do not arrive as a full health bar.
    vm.eval("_c = { stats = {} } DAEMON.trait.seed(_c, 'character') return 'seeded'").unwrap();
    vm.eval("DAEMON.trait.set_cur(_c, 'hp', 10) DAEMON.trait.set_cur(_c, 'stamina', 10)")
        .unwrap();
    vm.eval("_c.stats._at.hp = os_time() - 100000 \
             _c.stats._at.stamina = os_time() - 100000 return 'aged'")
        .unwrap();

    vm.eval("DAEMON.trait.attach(_c) return 'attached'").unwrap();
    assert_eq!(
        vm.eval("return tostring(_c.stats._at.hp >= os_time() - 1)").unwrap(),
        "true",
        "an offline = false gauge should re-anchor on attach"
    );
    assert_eq!(
        vm.eval("return tostring(_c.stats._at.stamina < os_time() - 1000)").unwrap(),
        "true",
        "an offline = true gauge should keep its anchor and pay out"
    );

    vm.eval("DAEMON.trait.touch(_c) return 'settled'").unwrap();
    assert_eq!(
        vm.eval("return DAEMON.trait.value(_c, 'stamina')").unwrap(),
        "80",
        "a day away should have filled the stamina bar"
    );
    assert!(
        vm.eval("return DAEMON.trait.value(_c, 'hp')").unwrap().parse::<i64>().unwrap() < 30,
        "hp should not have filled while offline"
    );
}

// `a_broken_trait_file_is_reported_and_survivable` used to live here, loaded
// from `game/traits/broken_example.lua`. Both are gone: broken code does not
// belong in a content directory, and nothing about that test depended on this
// game — it is `tests/broken_traits.rs` now, on the fixture world.

/// Memoization, and `bump_all` on a reload.
#[test]
fn values_are_memoized_and_a_reload_invalidates_them() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_evals = 0").unwrap();
    vm.eval(
        "DAEMON.trait.define({ id = 'memo_probe', kind = 'derived', \
           depends = { 'wisdom' }, sets = false, \
           formula = function(t) _evals = _evals + 1 return t.wisdom end }) \
         DAEMON.trait.seal()",
    )
    .unwrap();

    vm.eval("_c = { stats = {} } DAEMON.trait.seed(_c, 'character') return 'seeded'").unwrap();
    vm.eval("_evals = 0").unwrap();

    vm.eval("for i = 1, 20 do DAEMON.trait.value(_c, 'memo_probe') end return 'read'").unwrap();
    assert_eq!(
        vm.eval("return _evals").unwrap(),
        "1",
        "twenty reads should cost one recompute"
    );

    // A change bumps the entity.
    vm.eval("DAEMON.trait.set_base(_c, 'wisdom', 15) return 'ok'").unwrap();
    vm.eval("DAEMON.trait.value(_c, 'memo_probe') return 'read'").unwrap();
    assert_eq!(vm.eval("return _evals").unwrap(), "2");

    // `bump_all` invalidates everyone, which is what a reload does — values
    // recompute and nothing is stale.
    vm.eval("DAEMON.trait.bump_all() return 'bumped'").unwrap();
    vm.eval("DAEMON.trait.value(_c, 'memo_probe') return 'read'").unwrap();
    assert_eq!(vm.eval("return _evals").unwrap(), "3");
}

// ═════════════════════════════════════════════════════════════════════════════
//  Spells
// ═════════════════════════════════════════════════════════════════════════════

/// A spell's power reaches through the trait graph, so a wisdom buff changes a
/// fireball without the spell knowing.
#[test]
fn spell_power_reaches_two_levels_into_the_graph() {
    let mut vm = RealVm::boot_real_mudlib(0);

    // The rendered line carries ANSI colour escapes, and those contain digits —
    // `ESC[36m` is cyan — so they have to come out before anything looks for a
    // number. Strip first, scan second.
    let power = |vm: &mut RealVm| -> i64 {
        let raw = vm.command("cast");
        let mut out = String::new();
        let mut chars = raw.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                for e in chars.by_ref() {
                    if e.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        let tail = out
            .split("Spell power:")
            .nth(1)
            .unwrap_or_else(|| panic!("no spell power in:
{out}"))
            .to_string();
        let digits: String = tail
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(|c| c.is_ascii_digit())
            .collect();
        digits.parse().unwrap_or_else(|_| panic!("no number after the label: {tail:?}"))
    };

    // intelligence 10, wisdom 10, level 1 -> willpower 0 -> spell_power 5.
    assert_eq!(power(&mut vm), 5);

    vm.command("affect learn wisdom 20");
    assert_eq!(
        power(&mut vm),
        10,
        "raising wisdom should reach spell_power through willpower"
    );

    // And an *effect* on wisdom does the same, which is the point of the graph.
    vm.command("affect learn wisdom 10");
    vm.command("spawn scholar_circlet");
    vm.command("wear circlet");
    assert_eq!(
        power(&mut vm),
        6,
        "+2 intelligence from the circlet is +1 spell power"
    );
}

/// Damage goes through the pipeline, so armour and resists apply to a spell
/// exactly as they do to a sword.
#[test]
fn a_damage_spell_goes_through_the_damage_pipeline() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("DAEMON.world.register_area({ DAEMON.room.from_data({ id = 'spell.room', exits = {} }) })")
        .unwrap();
    vm.eval(
        "DAEMON.mobs.register({ id = 'spell_dummy', short = 'a dummy', \
           stats = { hp = 500, max_hp = 500, constitution = 10, level = 1 } })",
    )
    .unwrap();

    vm.eval(
        "_p = { char_id = 900, name = 'Caster', session_id = 'x', inventory = {}, \
                equipment = {}, quest_flags = {}, \
                stats = { level = 5, hp = 100, mp = 100, intelligence = 10, \
                          wisdom = 10, constitution = 10, strength = 10, dexterity = 10 }, \
                send = function() end, send_lines = function() end, \
                message_room = function() end }",
    )
    .unwrap();
    vm.eval("setmetatable(_p, { __index = require('lib.player') }) return 'ok'").unwrap();
    vm.eval("DAEMON.trait.attach(_p) DAEMON.world.place_character(900, 'spell.room')").unwrap();

    vm.eval("_t = DAEMON.mobs.spawn('spell_dummy', 'spell.room')").unwrap();
    vm.eval("_before = _t:trait('hp')").unwrap();

    // Emberlance goes through `resolve_attack` now rather than applying a
    // number, so it can miss — and "the spell dealt nothing" is exactly what a
    // miss looks like from here. Pin the die low, which always hits. The claim
    // is that the damage travels the pipeline, not that it never misses.
    vm.eval("DAEMON.combat._roll = function(n) if n == 100 then return 1 else return n end end \
             return 'loaded'")
        .unwrap();
    assert_eq!(
        vm.eval("return tostring(DAEMON.spell.cast(_p, 'emberlance', 'dummy'))").unwrap(),
        "true"
    );
    assert!(
        vm.eval("return _before - _t:trait('hp')").unwrap().parse::<i64>().unwrap() > 0,
        "the spell dealt nothing"
    );

    // Mana was spent — as a gauge `adjust`, never as a modifier. The starting
    // value is the *clamped* one, because `max_mp` is derived and the seeded
    // 100 is above it; asserting the delta rather than the absolute keeps this
    // about the spending rather than about the formula.
    assert_eq!(
        vm.eval("return _p:trait('max_mp') - _p:trait('mp')").unwrap(),
        "8",
        "emberlance costs 8"
    );

    // Armour applies. The dummy puts on a jerkin and takes less.
    vm.eval("_j = DAEMON.items.spawn('leather_jerkin', nil) \
             require('lib.equipment').equip(_t, _j, DAEMON.items.resolve(_j)) return 'armoured'")
        .unwrap();
    vm.eval("DAEMON.cooldown.clear(900, 'spell.emberlance') return 'ready'").unwrap();
    vm.eval("_before2 = _t:trait('hp')").unwrap();
    vm.eval("DAEMON.spell.cast(_p, 'emberlance', 'dummy') return 'cast'").unwrap();

    let bare: i64 = vm.eval("return _before - _before2").unwrap().parse().unwrap();
    let armoured: i64 = vm
        .eval("return _before2 - _t:trait('hp')")
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        armoured,
        bare - 3,
        "the jerkin's defence of 3 should come off a spell exactly as it comes \
         off a sword — a spell is not a special case"
    );
}

/// A cooldown under the durable threshold lives in memory, which is the tier
/// rule applied to spells.
#[test]
fn a_short_spell_cooldown_is_memory_tier() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    let threshold: i64 = vm
        .eval("return config('game.cooldown_durable_seconds') or 60")
        .unwrap()
        .parse()
        .unwrap();
    for id in ["emberlance", "mend", "farsight"] {
        let cd: i64 = vm
            .eval(&format!("return DAEMON.spell.get('{id}').cooldown"))
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            cd < threshold,
            "'{id}' has a {cd}s cooldown, which would be written to the database"
        );
    }
}

/// A spell with a `condition` is refused before it lands, with a reason.
#[test]
fn a_conditioned_spell_refuses_rather_than_landing() {
    let mut vm = RealVm::boot_real_mudlib(0);

    vm.command("affect learn level 5");
    vm.command("affect learn wisdom 20");
    let out = vm.command("cast wardskin");
    assert!(
        !out.contains("will not take"),
        "a willing caster should be able to cast it:\n{out}"
    );
    assert!(vm.command("affect list").contains("wardskin"));

    // A fresh character for the negative case: the first cast left a
    // thirty-second cooldown, and refusing for *that* reason would make this
    // pass without testing anything.
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("affect learn level 5");
    // Willpower below zero: wisdom 1, level 1 -> floor((1-10)/2) = -5.
    vm.command("affect learn wisdom 1");
    let out = vm.command("cast wardskin");
    assert!(
        out.contains("will not take") || out.contains("will is not in it"),
        "a condition that is false should refuse by name:\n{out}"
    );
    assert!(
        !vm.command("affect list").contains("wardskin"),
        "a refused effect must not be present at all"
    );
}
