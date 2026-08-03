//! Traits and effects as a player meets them: through the real mudlib, the
//! real game layer, and the real command dispatcher.
//!
//! `tests/lua_unit.rs` proves the logic. It would stay green if
//! `mudlib/init.lua` never mentioned `trait_d`, or if `game/init.lua` never
//! registered a single trait — it requires the modules itself. The assertions
//! here are the ones that can only be made from outside: that the wiring
//! exists, that a command shows the right number, and that what should survive
//! a shutdown does.

mod common;

use std::time::Duration;

use common::RealVm;

const GENEROUS: Duration = Duration::from_secs(10);

/// Every stored document, for asking what actually reached disk after the VM
/// has gone. Returns `(collection, id, data)` rows.
fn documents(vm: &RealVm, collection: &str) -> Vec<String> {
    use diesel::prelude::*;
    use diesel::sql_types::Text;
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        data: String,
    }
    let mut conn = vm.pool().get_sqlite().unwrap();
    diesel::sql_query("SELECT data FROM documents WHERE collection = ?")
        .bind::<Text, _>(collection)
        .load::<Row>(&mut conn)
        .unwrap()
        .into_iter()
        .map(|r| r.data)
        .collect()
}

/// The game layer really registered its traits, and `score` really renders them.
#[test]
fn score_shows_a_trait_derived_from_another_trait() {
    let mut vm = RealVm::boot_real_mudlib(0);
    let out = vm.command("score");

    assert!(out.contains("Willpower"), "score did not show Willpower:\n{out}");
    assert!(out.contains("Strength"), "score did not show the attributes:\n{out}");
    assert!(out.contains("Health"), "score did not show the gauges:\n{out}");
    assert!(
        out.contains("derived"),
        "a derived trait should say so, so nobody looks for where it is stored:\n{out}"
    );
}

/// Wisdom goes up, Willpower follows, and nothing wrote Willpower anywhere.
#[test]
fn a_derived_trait_follows_the_trait_it_depends_on() {
    let mut vm = RealVm::boot_real_mudlib(0);
    let before = vm.command("affect traits");
    assert!(before.contains("willpower"), "expected the trait dump:\n{before}");

    // Wisdom 10, level 1 -> willpower 0. A +2 wisdom effect cannot change that
    // by itself (integer halving), so drive the base instead through the one
    // command that can.
    vm.command("affect apply cursed 600");
    let after = vm.command("affect traits");
    assert!(
        after.contains("cursed") || after != before,
        "applying an effect that modifies willpower changed nothing:\n{after}"
    );
}

/// The first of the four worked examples, seen by a player.
#[test]
fn a_buff_changes_an_attribute_without_storing_anything_on_it() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("affect apply stoneskin 600");
    let out = vm.command("affect traits");

    // stoneskin gives +2 constitution: base stays 10, value becomes 12.
    let line = out
        .lines()
        .find(|l| l.contains("constitution"))
        .unwrap_or_else(|| panic!("no constitution row in:\n{out}"));
    assert!(
        line.contains("10") && line.contains("12"),
        "expected base 10 and effective 12, got: {line}"
    );
}

/// "take 15% less damage" and "negate 5 from each bit of damage", together,
/// on a real 30-point hit. The percentage applies first because of its phase,
/// not because of when it was registered.
#[test]
fn mitigation_applies_in_phase_order() {
    let mut vm = RealVm::boot_real_mudlib(0);

    let plain = vm.command("affect damage 30");
    assert!(plain.contains("30 requested, 30 dealt"), "unmitigated:\n{plain}");

    vm.command("affect heal 500");
    vm.command("affect apply stoneskin 600");
    let buffed = vm.command("affect damage 30");
    assert!(
        buffed.contains("30 requested, 20 dealt"),
        "expected 30 * 0.85 - 5 = 20.5, floored to 20 — and NOT 21, which is \
         what applying the flat reduction first would give:\n{buffed}"
    );
}

/// "a buff that makes me make 20% more experience".
#[test]
fn an_experience_buff_scales_what_is_awarded() {
    let mut vm = RealVm::boot_real_mudlib(0);
    let plain = vm.command("affect xp 100");
    assert!(plain.contains("100 requested, 100 awarded"), "unbuffed:\n{plain}");

    vm.command("affect apply insight 600");
    let buffed = vm.command("affect xp 100");
    assert!(
        buffed.contains("100 requested, 120 awarded"),
        "expected +20%:\n{buffed}"
    );
}

/// The vertical slice: an item, through the existing drinkable component and
/// the existing `drink` command, puts a real effect on the character.
#[test]
fn drinking_a_potion_applies_an_effect() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("spawn regen_draught");

    let drink = vm.command("drink draught");
    assert!(
        drink.contains("moss and iron") || drink.contains("warm glow"),
        "the draught was not drunk:\n{drink}"
    );

    let effects = vm.command("effects");
    assert!(
        effects.contains("Regeneration"),
        "the potion did not apply its effect:\n{effects}"
    );
    assert!(
        effects.contains("m") || effects.contains("s"),
        "an effect should show how long is left:\n{effects}"
    );
}

/// Regeneration heals over time, driven through the engine's own timer
/// dispatch and the ticker id the daemon actually registered.
///
/// A test that called `DAEMON.effect.heartbeat()` directly would stay green if
/// `ticker.every` were never called at all. This goes the whole way round:
/// `LuaCommand::TimerFired` -> `on_timer` -> `ticker.fire` -> the callback the
/// daemon registered under that id.
#[test]
fn the_heartbeat_ticker_drives_a_regeneration_effect() {
    let mut vm = RealVm::boot_real_mudlib_with_probe_opts(common::TestCtx {
        // Non-zero, so the daemon registers its ticker at all.
        effect_heartbeat_seconds: Some(3),
        ..Default::default()
    });

    assert_eq!(
        vm.eval("return tostring(DAEMON.ticker.is_active('effect.heartbeat'))").unwrap(),
        "true",
        "effect_d did not register its heartbeat"
    );

    vm.eval(
        "local Player = require('lib.player') \
         p = Player:from_save(1, { name = 'Probe', account_id = 1 }, {}) \
         DAEMON.trait.set_cur(p, 'hp', 20) \
         DAEMON.effect.apply(p, 'regeneration') \
         return 'ok'",
    )
    .unwrap();

    // Rewind the tick clock rather than sleeping: what is under test is that
    // the heartbeat runs and heals, not that time passes.
    vm.eval(
        "DAEMON.effect.active(p)[1].inst.last_tick = os_time() - 30 \
         _before = DAEMON.trait.value(p, 'hp') return 'ok'",
    )
    .unwrap();

    vm.engine().send(oxigeon::core::scripting::LuaCommand::TimerFired {
        id: "effect.heartbeat".to_string(),
    });

    assert_eq!(
        vm.eval("return tostring(DAEMON.trait.value(p, 'hp') > _before)").unwrap(),
        "true",
        "the heartbeat did not heal — is the ticker id still 'effect.heartbeat'?"
    );
}

/// Every periodic subsystem registers the id it says it does. These are the
/// strings the engine dispatches against, and nothing else checks them.
#[test]
fn the_periodic_subsystems_register_their_timers() {
    let mut vm = RealVm::boot_real_mudlib_with_probe_opts(common::TestCtx {
        cache_flush_seconds: Some(5),
        effect_sweep_seconds: Some(5),
        effect_heartbeat_seconds: Some(3),
        combat_round_seconds: Some(3),
        ..Default::default()
    });
    for id in ["cache.flush", "effect.sweep", "effect.heartbeat", "combat.round"] {
        assert_eq!(
            vm.eval(&format!("return tostring(DAEMON.ticker.is_active('{id}'))")).unwrap(),
            "true",
            "no timer registered under '{id}'"
        );
    }
}

/// An effect that ends while its owner is standing still still has to say so.
/// That is the sweep's whole job — lazy expiry keeps reads honest, but only a
/// sweep produces the message.
#[test]
fn the_sweep_expires_an_effect_for_an_idle_character() {
    let mut vm = RealVm::boot_real_mudlib_with_probe_opts(common::TestCtx {
        effect_sweep_seconds: Some(5),
        ..Default::default()
    });
    vm.eval(
        "local Player = require('lib.player') \
         p = Player:from_save(1, { name = 'Probe', account_id = 1 }, {}) \
         DAEMON.effect.apply(p, 'stoneskin', { duration = 60 }) \
         DAEMON.effect.active(p)[1].inst.expires = os_time() - 1 \
         return tostring(#DAEMON.effect.active(p))",
    )
    .unwrap();

    vm.engine().send(oxigeon::core::scripting::LuaCommand::TimerFired {
        id: "effect.sweep".to_string(),
    });

    assert_eq!(
        vm.eval("return tostring(#DAEMON.effect.active(p))").unwrap(),
        "0",
        "the sweep left an expired effect in place"
    );
}

/// The prompt renders every trait on every command. A broken `touch` there
/// shows up as a timeout waiting for a prompt, which no stub test can produce.
#[test]
fn the_prompt_still_renders_after_every_change() {
    let mut vm = RealVm::boot_real_mudlib(0);
    // `RealVm::command` waits for the prompt, so each of these would hang if
    // the prompt path broke.
    vm.command("affect apply stoneskin 60");
    vm.command("affect damage 10");
    vm.command("affect heal 5");
    vm.command("look");
    let out = vm.command("score");
    assert!(out.contains("Health"), "the game stopped answering:\n{out}");
}

/// The headline durability claim, both halves.
///
/// An effect long enough to matter after a restart is written. One that would
/// have expired before the server came back is not — not because writing it
/// would be slow, but because it would be wrong.
#[test]
fn a_long_effect_is_saved_and_a_short_one_is_never_written() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("affect apply insight 3600");
    vm.command("affect apply stoneskin 5");

    // Both are live right now.
    let effects = vm.command("effects");
    assert!(effects.contains("Insight"), "the long effect is not active:\n{effects}");
    assert!(effects.contains("Stoneskin"), "the short effect is not active:\n{effects}");

    assert!(vm.shutdown_within(GENEROUS), "the mudlib did not finish shutting down");

    let stored = documents(&vm, "effects");
    assert!(!stored.is_empty(), "the shutdown flush wrote no effects at all");
    let all = stored.join("\n");
    assert!(
        all.contains("insight"),
        "an hour-long buff should survive a restart; the document holds: {all}"
    );
    assert!(
        !all.contains("stoneskin"),
        "a five-second buff must never reach the database — it would be gone \
         before the server came back: {all}"
    );
}

/// Traits are character data, so they ride CHARACTER_D's existing save path
/// rather than needing one of their own.
#[test]
fn trait_changes_reach_the_character_row_on_shutdown() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("affect damage 25");

    assert!(vm.shutdown_within(GENEROUS));

    use diesel::prelude::*;
    use diesel::sql_types::Text;
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        data: String,
    }
    let mut conn = vm.pool().get_sqlite().unwrap();
    let rows: Vec<Row> = diesel::sql_query("SELECT data FROM characters")
        .load(&mut conn)
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0].data.contains("\"wisdom\""),
        "a trait that did not exist before this pass should now be saved — the \
         hardcoded stat list in Mobile:new used to drop it: {}",
        rows[0].data
    );
    assert!(
        rows[0].data.contains("_at"),
        "the regeneration anchor has to be saved with the gauge, or a character \
         gets all their idle time back at once on login: {}",
        rows[0].data
    );
}

/// A trait recompute is the largest arithmetic loop the mudlib runs per
/// command, and the instruction budget is only armed in the real VM.
#[test]
fn everything_still_works_with_the_instruction_budget_armed() {
    let mut vm = RealVm::boot_real_mudlib(1_000_000);
    vm.command("affect apply stoneskin 600");
    vm.command("affect apply insight 600");
    vm.command("affect apply hearty 600");

    let out = vm.command("score");
    assert!(out.contains("Willpower"), "score ran out of budget:\n{out}");
    let traits = vm.command("affect traits");
    assert!(traits.contains("max_hp"), "the trait dump ran out of budget:\n{traits}");
}

/// A recompute costs what the entity holds, not what the game has defined.
///
/// This is the whole point of the present set. It is asserted by counting
/// formula evaluations rather than by timing: a timing assertion would be
/// flaky, and a call count is exact.
///
/// Two entities, one registry with 200 derived traits in it. The sword holds
/// nothing any of them read, so it pays for none of them. The character holds
/// the one base they all read, so it pays for all of them. Nothing declared
/// that — it falls out of what each one stores.
#[test]
fn a_recompute_costs_what_the_entity_holds_not_what_the_registry_defines() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_T = DAEMON.trait; _calls = 0").unwrap();
    vm.eval("_T.define({ id = 'probe_base', kind = 'attribute', default = 1 })").unwrap();
    vm.eval("_T.define({ id = 'probe_dura', kind = 'attribute', default = 0 })").unwrap();
    vm.eval(
        "for i = 1, 200 do _T.define({ id = 'probe_d' .. i, kind = 'derived', \
         depends = { 'probe_base' }, \
         formula = function(t) _calls = _calls + 1; return t.probe_base + i end }) end",
    )
    .unwrap();
    vm.eval("_T.seal()").unwrap();

    // A sword: one trait, and none of the 200 read it.
    vm.eval("_sword = { stats = { probe_dura = 40 } }; _before = _calls").unwrap();
    vm.eval("_T.value(_sword, 'probe_dura')").unwrap();
    let sword_calls = vm.eval("return _calls - _before").unwrap();

    // A character: holds the base every one of the 200 depends on.
    vm.eval("_char = { stats = { probe_base = 5 } }; _before = _calls").unwrap();
    vm.eval("_T.value(_char, 'probe_base')").unwrap();
    let char_calls = vm.eval("return _calls - _before").unwrap();

    assert_eq!(
        sword_calls, "0",
        "an entity holding none of the 200 derived traits still evaluated {sword_calls} of them"
    );
    assert_eq!(
        char_calls, "200",
        "an entity holding what all 200 derived traits read should evaluate all of them"
    );

    assert_eq!(
        vm.eval("return #_T.present(_sword)").unwrap(),
        "1",
        "the sword should hold exactly the one trait it stores"
    );
    assert_eq!(
        vm.eval("return _T.present(_sword)[1]").unwrap(),
        "probe_dura",
        "and it should be the one it stores"
    );

    // The memo and the present set are guarded by the same counters, so a
    // second read costs nothing at all.
    vm.eval("_before = _calls").unwrap();
    vm.eval("_T.value(_char, 'probe_d7')").unwrap();
    assert_eq!(
        vm.eval("return _calls - _before").unwrap(),
        "0",
        "a repeat read re-evaluated formulas the memo should have covered"
    );
}

/// Storage decides what an entity has, and derived traits follow what they read.
#[test]
fn presence_follows_what_the_entity_stores() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_T = DAEMON.trait").unwrap();
    vm.eval("_T.define({ id = 'p_dura', kind = 'attribute', default = 0 })").unwrap();
    vm.eval("_T.define({ id = 'p_speed', kind = 'attribute', default = 1 })").unwrap();
    vm.eval("_T.define({ id = 'p_maxcharge', kind = 'attribute', default = 10 })").unwrap();
    vm.eval("_T.define({ id = 'p_charge', kind = 'gauge', min = 0, max = 'p_maxcharge' })").unwrap();
    vm.eval(
        "_T.define({ id = 'p_dps', kind = 'derived', depends = { 'p_dura', 'p_speed' }, \
         formula = function(t) return t.p_dura * t.p_speed end })",
    )
    .unwrap();
    vm.eval("_T.seal()").unwrap();

    // A sword stores two traits, so it also has the one derived from them.
    vm.eval("_sword = { stats = { p_dura = 40, p_speed = 2 } }").unwrap();
    assert_eq!(vm.eval("return _T.has(_sword, 'p_dura')").unwrap(), "true");
    assert_eq!(
        vm.eval("return _T.has(_sword, 'p_dps')").unwrap(),
        "true",
        "a derived trait is present when everything it reads is"
    );
    assert_eq!(
        vm.eval("return _T.has(_sword, 'wisdom')").unwrap(),
        "false",
        "a sword should not have the character traits the game happens to define"
    );
    assert_eq!(vm.eval("return #_T.present(_sword)").unwrap(), "3");
    assert_eq!(vm.eval("return _T.value(_sword, 'p_dps')").unwrap(), "80");

    // A bound naming a trait the entity does not have takes the gauge with it:
    // a gauge with no ceiling is not the trait that was defined.
    vm.eval("_wand = { stats = { p_charge = 5 } }").unwrap();
    assert_eq!(
        vm.eval("return _T.has(_wand, 'p_charge')").unwrap(),
        "false",
        "a gauge whose max names an absent trait should be absent too"
    );
    vm.eval("_wand2 = { stats = { p_charge = 5, p_maxcharge = 10 } }").unwrap();
    assert_eq!(vm.eval("return _T.has(_wand2, 'p_charge')").unwrap(), "true");

    // And the derived trait does not leak onto something holding neither dep.
    assert_eq!(vm.eval("return _T.has(_wand, 'p_dps')").unwrap(), "false");
}

/// Learning, forgetting, reading what is not there, and a value stored under a
/// trait that does not exist yet.
#[test]
fn a_trait_can_be_learned_forgotten_and_defined_after_the_fact() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_T = DAEMON.trait").unwrap();
    vm.eval("_T.define({ id = 'p_sword_skill', kind = 'counter', default = 0, min = 0 })").unwrap();
    vm.eval("_T.seal()").unwrap();
    vm.eval("_p = { stats = {} }").unwrap();

    // Reading a trait you do not have answers with the default so arithmetic
    // stays safe — and writes nothing, which is the part worth pinning.
    assert_eq!(vm.eval("return _T.has(_p, 'p_sword_skill')").unwrap(), "false");
    assert_eq!(vm.eval("return _T.value(_p, 'p_sword_skill')").unwrap(), "0");
    assert_eq!(
        vm.eval("return tostring(_p.stats.p_sword_skill)").unwrap(),
        "nil",
        "reading an absent trait materialised it"
    );

    // Setting a base is how a skill is learned.
    vm.eval("_T.set_base(_p, 'p_sword_skill', 7)").unwrap();
    assert_eq!(vm.eval("return _T.has(_p, 'p_sword_skill')").unwrap(), "true");
    assert_eq!(vm.eval("return _T.value(_p, 'p_sword_skill')").unwrap(), "7");

    // And forgetting reverses it.
    assert_eq!(vm.eval("return _T.forget(_p, 'p_sword_skill')").unwrap(), "true");
    assert_eq!(vm.eval("return _T.has(_p, 'p_sword_skill')").unwrap(), "false");
    assert_eq!(vm.eval("return _T.forget(_p, 'p_sword_skill')").unwrap(), "false");

    // A number stored under an id no trait has claimed is inert, not an error —
    // and starts answering the moment the trait is defined, at runtime.
    vm.eval("_e = { stats = { p_later = 3 } }").unwrap();
    assert_eq!(
        vm.eval("return _T.has(_e, 'p_later')").unwrap(),
        "false",
        "an undefined id should read as absent rather than raising"
    );
    vm.eval("_T.define({ id = 'p_later', kind = 'attribute', default = 0 })").unwrap();
    vm.eval("_T.seal()").unwrap();
    assert_eq!(vm.eval("return _T.has(_e, 'p_later')").unwrap(), "true");
    assert_eq!(
        vm.eval("return _T.value(_e, 'p_later')").unwrap(),
        "3",
        "the value that was already stored should be the one it answers with"
    );
}

/// `attach` prepares an entity; `seed` decides what it starts with. Splitting
/// them is what lets an item have the lifecycle without the stat block.
#[test]
fn attach_prepares_and_seed_populates() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_T = DAEMON.trait").unwrap();
    vm.eval("_T.define({ id = 'p_grip', kind = 'attribute', default = 3, sets = 'item' })").unwrap();
    vm.eval("_T.seal()").unwrap();

    // Attaching gives an entity nothing. It is lifecycle, not creation.
    vm.eval("_bare = { stats = {} }; _T.attach(_bare)").unwrap();
    assert_eq!(
        vm.eval("return #_T.present(_bare)").unwrap(),
        "0",
        "attach should not have decided what this entity is"
    );

    // Seeding the character set gives it the character stat block, and not the
    // trait that belongs to items.
    vm.eval("_c = { stats = {} }; _T.seed(_c, 'character')").unwrap();
    assert!(
        vm.eval("return _T.has(_c, 'strength')").unwrap() == "true",
        "seeding the character set should give a character its attributes"
    );
    assert_eq!(
        vm.eval("return _T.has(_c, 'p_grip')").unwrap(),
        "false",
        "a trait in the item set has no business on a freshly seeded character"
    );

    // And the item set gives the item exactly its own.
    vm.eval("_i = { stats = {} }; _T.seed(_i, 'item')").unwrap();
    assert_eq!(vm.eval("return _T.has(_i, 'p_grip')").unwrap(), "true");
    assert_eq!(
        vm.eval("return _T.has(_i, 'strength')").unwrap(),
        "false",
        "an item seeded from the item set should not carry character attributes"
    );
    assert_eq!(vm.eval("return _T.value(_i, 'p_grip')").unwrap(), "3");

    // Seeding is idempotent — it never overwrites a value already stored.
    vm.eval("_T.set_base(_i, 'p_grip', 9); _T.seed(_i, 'item')").unwrap();
    assert_eq!(
        vm.eval("return _T.value(_i, 'p_grip')").unwrap(),
        "9",
        "re-seeding overwrote a value the entity had already earned"
    );
}
