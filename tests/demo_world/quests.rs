//! Quests — and the three persistence tiers a quest system needs at once.
//!
//! `Player:set_quest_flag` / `get_quest_flag` / `has_quest_flag` and the
//! `quest:` effect source scheme all existed and had **no callers**. The
//! interesting property is not that quests work; it is that choosing the wrong
//! tier for each kind of quest state is invisible until it is not:
//!
//!   flags     "have I ever finished this"  -> a SAVE_FIELD, a forever answer
//!   counters  "how many rats so far"       -> write-behind; losing 30s on a
//!                                            crash is fine, a write per rat
//!                                            is not
//!   dailies   "have I done this today"     -> a durable cooldown, which
//!                                            survives an area reset. Room
//!                                            object state does not, and that
//!                                            is `task_list.md`'s opening bug.


use crate::common::RealVm;

fn go_to(vm: &mut RealVm, room: &str) {
    let out = vm.command(&format!("goto {room}"));
    assert!(!out.contains("Unknown"), "could not reach {room}:\n{out}");
}

/// Offers come from whoever is standing here, so a giver is more than a label.
#[test]
fn quests_are_offered_by_whoever_is_here() {
    let mut vm = RealVm::boot_real_mudlib(0);

    go_to(&mut vm, "thornhollow.square");
    assert!(
        vm.command("quest").contains("Nobody here"),
        "an empty square should offer nothing"
    );

    go_to(&mut vm, "thornhollow.apothecary");
    let out = vm.command("quest");
    assert!(
        out.contains("roots_for_the_apothecary"),
        "the apothecary should be offering her quest:\n{out}"
    );
    assert!(out.contains("apothecary"), "the offer should name the giver:\n{out}");

    // And you cannot take one from somebody who is not there.
    go_to(&mut vm, "thornhollow.square");
    let out = vm.command("quest accept roots_for_the_apothecary");
    assert!(
        out.contains("ask them yourself"),
        "accepting at a distance should be refused:\n{out}"
    );
}

/// FETCH — counted from what you are holding, so picking an item up, dropping
/// it and picking it up again is one item rather than two.
#[test]
fn a_fetch_quest_counts_what_you_hold() {
    let mut vm = RealVm::boot_real_mudlib(0);
    go_to(&mut vm, "thornhollow.apothecary");
    vm.command("quest accept roots_for_the_apothecary");

    assert!(vm.command("quests").contains("0/3"), "should start at zero");

    vm.command("spawn dried_marshroot");
    vm.command("drop marshroot");
    vm.command("get marshroot");
    assert!(vm.command("quests").contains("1/3"), "one root, one counted");

    // Down and up again is still one.
    vm.command("drop marshroot");
    vm.command("get marshroot");
    let out = vm.command("quests");
    assert!(
        out.contains("1/3"),
        "putting an item down and picking it up again counted it twice:\n{out}"
    );

    vm.command("spawn dried_marshroot");
    vm.command("drop marshroot");
    vm.command("get marshroot");
    vm.command("spawn dried_marshroot");
    vm.command("drop marshroot");
    vm.command("get marshroot");
    assert!(vm.command("quests").contains("ready"), "three should finish it");

    let out = vm.command("quest complete roots_for_the_apothecary");
    assert!(out.contains("That will do"), "hand-in failed:\n{out}");

    // The items were taken.
    assert!(
        !vm.command("inventory").contains("marshroot"),
        "a collect quest should take the items it asked for"
    );
    // And the reward landed: `skill` teaches a trait the character did not have.
    assert!(
        vm.command("skills").contains("Herbalism"),
        "the skill reward should have created the trait"
    );
}

/// KILL-COUNT — the write-behind counter, advanced by the `mob.died` event
/// rather than by combat knowing quests exist.
#[test]
fn a_kill_quest_counts_through_the_event() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    // A character the daemons can find by id.
    vm.eval(
        "_p = { char_id = 700, name = 'Hunter', inventory = {}, equipment = {}, \
                quest_flags = {}, stats = { level = 5, hp = 100 }, \
                send = function() end }",
    )
    .unwrap();
    vm.eval("setmetatable(_p, { __index = require('lib.player') }) return 'ok'").unwrap();
    vm.eval("DAEMON.character._cache[700] = _p return 'cached'").unwrap();

    vm.eval("DAEMON.quest.accept(_p, 'thin_the_crawlers') return 'accepted'").unwrap();
    assert_eq!(vm.eval("return DAEMON.quest.progress(_p, 'thin_the_crawlers')").unwrap(), "0");

    // Emit the event four times: the quest system never hears from combat.
    vm.eval(
        "for i = 1, 4 do DAEMON.event.emit('mob.died', \
         { template_id = 'reed_crawler', killer_char_id = 700 }) end return 'killed'",
    )
    .unwrap();
    assert_eq!(vm.eval("return DAEMON.quest.progress(_p, 'thin_the_crawlers')").unwrap(), "4");
    assert_eq!(
        vm.eval("return tostring(DAEMON.quest.is_ready(_p, 'thin_the_crawlers'))").unwrap(),
        "false"
    );

    // The wrong creature does not count.
    vm.eval(
        "DAEMON.event.emit('mob.died', { template_id = 'workshop_rat', killer_char_id = 700 }) \
         return 'ok'",
    )
    .unwrap();
    assert_eq!(vm.eval("return DAEMON.quest.progress(_p, 'thin_the_crawlers')").unwrap(), "4");

    vm.eval(
        "DAEMON.event.emit('mob.died', { template_id = 'reed_crawler', killer_char_id = 700 }) \
         return 'ok'",
    )
    .unwrap();
    assert_eq!(
        vm.eval("return tostring(DAEMON.quest.is_ready(_p, 'thin_the_crawlers'))").unwrap(),
        "true"
    );

    // And it clamps: a sixth kill does not make it "6 / 5".
    vm.eval(
        "DAEMON.event.emit('mob.died', { template_id = 'reed_crawler', killer_char_id = 700 }) \
         return 'ok'",
    )
    .unwrap();
    assert_eq!(vm.eval("return DAEMON.quest.progress(_p, 'thin_the_crawlers')").unwrap(), "5");
}

/// The counter is in the **write-behind** tier, which is the whole argument:
/// a kill counter is written constantly and read almost never.
#[test]
fn the_counter_is_write_behind_and_the_flag_is_a_save_field() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    assert_eq!(
        vm.eval("return DAEMON.cache.spec('quests').tier").unwrap(),
        "write_behind",
        "a kill counter in the write-through tier would be a database write per rat"
    );

    // The flag is on the Player, in `quest_flags`, which is already saved.
    vm.eval(
        "_p = { char_id = 701, name = 'Flagged', inventory = {}, equipment = {}, \
                quest_flags = {}, stats = { level = 5 }, send = function() end }",
    )
    .unwrap();
    vm.eval("setmetatable(_p, { __index = require('lib.player') }) return 'ok'").unwrap();
    vm.eval("_p:set_quest_flag('quest.done.example', true) return 'set'").unwrap();
    assert_eq!(
        vm.eval("return tostring(_p:has_quest_flag('quest.done.example'))").unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval(
            "local saved = _p:to_save() \
             return tostring(saved.quest_flags['quest.done.example'])"
        )
        .unwrap(),
        "true",
        "a forever answer belongs in the tier for forever answers"
    );
}

/// DAILY — a durable cooldown, which survives an area reset. Room object state
/// would not, and that is the bug.
#[test]
fn a_daily_quest_gate_survives_an_area_reset() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval(
        "_p = { char_id = 702, name = 'Daily', inventory = {}, equipment = {}, \
                quest_flags = {}, gold = 0, xp = 0, \
                stats = { level = 5, hp = 100 }, send = function() end }",
    )
    .unwrap();
    vm.eval("setmetatable(_p, { __index = require('lib.player') }) return 'ok'").unwrap();

    vm.eval("DAEMON.quest.accept(_p, 'the_days_ore') return 'accepted'").unwrap();
    vm.eval("_p:add_item('iron_ore') return 'ore'").unwrap();
    vm.eval("DAEMON.event.emit('item.picked_up', \
             { char_id = 702, template_id = 'iron_ore' }) return 'counted'")
        .unwrap();
    // The counter is recomputed from what they hold, which needs the character
    // to be findable — so set it directly for this one.
    vm.eval("DAEMON.cache.set('quests', 'char:702', 'the_days_ore', 1) return 'ok'").unwrap();

    assert_eq!(
        vm.eval("return tostring(DAEMON.quest.complete(_p, 'the_days_ore'))").unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval("return tostring(DAEMON.quest.can_accept(_p, 'the_days_ore'))").unwrap(),
        "false",
        "a daily quest should not be available twice in one day"
    );

    // Reset every area. A gate stored as room object state would come back.
    vm.eval("DAEMON.world.reset_all_areas() return 'reset'").unwrap();
    assert_eq!(
        vm.eval("return tostring(DAEMON.quest.can_accept(_p, 'the_days_ore'))").unwrap(),
        "false",
        "the daily gate was reset along with the world — per-character state \
         does not belong on a room"
    );

    // And it is in the durable tier, so a restart would not clear it either.
    assert!(
        24 * 3600
            > vm.eval("return config('game.cooldown_durable_seconds') or 60")
                .unwrap()
                .parse::<i64>()
                .unwrap(),
        "a daily gate under the durable threshold would be forgotten on restart"
    );
}

/// CHAIN — gated on another quest's flag.
#[test]
fn a_chain_quest_is_gated_on_the_one_before_it() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval(
        "_p = { char_id = 703, name = 'Chained', inventory = {}, equipment = {}, \
                quest_flags = {}, gold = 0, xp = 0, \
                stats = { level = 12, hp = 100 }, send = function() end }",
    )
    .unwrap();
    vm.eval("setmetatable(_p, { __index = require('lib.player') }) return 'ok'").unwrap();

    vm.eval("_ok, _why = DAEMON.quest.can_accept(_p, 'what_is_down_there')").unwrap();
    assert_eq!(vm.eval("return tostring(_ok)").unwrap(), "false");
    assert_eq!(
        vm.eval("return _why").unwrap(),
        "There is something else to do first."
    );

    // Finish the one it depends on, and the gate opens. The flag is the
    // mechanism, and it is the same flag `complete` sets.
    vm.eval("_p:set_quest_flag('quest.done.word_to_the_deep', true) return 'ok'").unwrap();
    assert_eq!(
        vm.eval("return tostring(DAEMON.quest.can_accept(_p, 'what_is_down_there'))").unwrap(),
        "true"
    );

    // The level gate is separate and still applies.
    vm.eval("DAEMON.trait.set_base(_p, 'level', 4) return 'ok'").unwrap();
    vm.eval("_ok2, _why2 = DAEMON.quest.can_accept(_p, 'what_is_down_there')").unwrap();
    assert_eq!(vm.eval("return tostring(_ok2)").unwrap(), "false");
    assert_eq!(vm.eval("return _why2").unwrap(), "You are not ready for that yet.");
}

/// Abandoning throws the counter away, so taking it again starts over.
#[test]
fn abandoning_a_quest_discards_its_progress() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval(
        "_p = { char_id = 704, name = 'Quitter', inventory = {}, equipment = {}, \
                quest_flags = {}, stats = { level = 5 }, send = function() end }",
    )
    .unwrap();
    vm.eval("setmetatable(_p, { __index = require('lib.player') }) return 'ok'").unwrap();

    vm.eval("DAEMON.quest.accept(_p, 'thin_the_crawlers') return 'ok'").unwrap();
    vm.eval("DAEMON.quest.advance(_p, 'thin_the_crawlers', 3) return 'ok'").unwrap();
    assert_eq!(vm.eval("return DAEMON.quest.progress(_p, 'thin_the_crawlers')").unwrap(), "3");

    assert_eq!(
        vm.eval("return tostring(DAEMON.quest.abandon(_p, 'thin_the_crawlers'))").unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval("return tostring(DAEMON.quest.is_active(_p, 'thin_the_crawlers'))").unwrap(),
        "false"
    );

    vm.eval("DAEMON.quest.accept(_p, 'thin_the_crawlers') return 'ok'").unwrap();
    assert_eq!(
        vm.eval("return DAEMON.quest.progress(_p, 'thin_the_crawlers')").unwrap(),
        "0",
        "taking a quest again should start over, not resume"
    );
}

/// A quest reward can apply an effect through the documented `quest:` source
/// scheme, which had no user at all.
#[test]
fn a_reward_can_apply_an_effect_from_a_quest_source() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval(
        "_p = { char_id = 705, name = 'Rewarded', inventory = {}, equipment = {}, \
                quest_flags = {}, gold = 0, xp = 0, \
                stats = { level = 12, hp = 100, intelligence = 10, wisdom = 10, \
                          constitution = 10, strength = 10, dexterity = 10 }, \
                send = function() end }",
    )
    .unwrap();
    vm.eval("setmetatable(_p, { __index = require('lib.player') }) return 'ok'").unwrap();

    vm.eval("DAEMON.quest.reward(_p, DAEMON.quest.get('what_is_down_there')) return 'paid'")
        .unwrap();

    assert_eq!(vm.eval("return _p.gold").unwrap(), "800");
    assert!(
        vm.eval("return #_p.inventory").unwrap().parse::<i64>().unwrap() >= 1,
        "the item reward did not arrive"
    );
    assert_eq!(
        vm.eval(
            "for _, e in ipairs(DAEMON.effect.active(_p)) do \
             if e.inst.def == 'insight' then return e.inst.source end end return 'none'"
        )
        .unwrap(),
        "quest:what_is_down_there",
        "the effect should be sourced to the quest that granted it"
    );
}
