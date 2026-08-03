//! The Collapsed Mine — dark rooms, a locked door, a lever puzzle and a boss.
//!
//! `Room.light_level` had been a field since rooms existed and **nothing read
//! it**: every room was equally visible and the field documented an intention.
//! The rest of this file is object state used the way object state is for — a
//! door, a lever, a worked-out seam — against the marsh's daily gate, which is
//! per-character and deliberately does *not* live there.

mod common;

use common::RealVm;

fn go_to(vm: &mut RealVm, room: &str) {
    let out = vm.command(&format!("goto {room}"));
    assert!(!out.contains("Unknown"), "could not reach {room}:\n{out}");
}

/// A pitch-dark room needs a light, and a light is per instance.
#[test]
fn you_cannot_see_in_the_dark_without_a_lantern() {
    let mut vm = RealVm::boot_real_mudlib(0);
    go_to(&mut vm, "collapsed_mine.first_level");

    let out = vm.command("look");
    assert!(out.contains("pitch dark"), "the mine should be dark:\n{out}");
    assert!(
        !out.contains("propped every eight feet"),
        "the description leaked in the dark:\n{out}"
    );
    // Exits are still felt for. A dark room is a situation, not a wall.
    assert!(out.contains("feel your way"), "no exits in the dark:\n{out}");

    // Even a named look is refused — "look at the lever" in a dark room should
    // not work either.
    assert!(vm.command("look arrows").contains("pitch dark"));

    // An unlit lantern is not a light.
    vm.command("spawn hooded_lantern");
    assert!(
        vm.command("look").contains("pitch dark"),
        "an unlit lantern should not light anything"
    );

    let out = vm.command("use lantern");
    assert!(out.contains("Warm yellow light"), "the lantern did not light:\n{out}");

    let out = vm.command("look");
    assert!(
        out.contains("propped every eight feet"),
        "a lit lantern should let you see:\n{out}"
    );

    // And it can be shut again.
    vm.command("use lantern");
    assert!(vm.command("look").contains("pitch dark"));
}

/// Two lanterns must be able to disagree about whether they are burning, which
/// is why `lit` is per-instance object state and not a field on the template.
#[test]
fn two_lanterns_can_disagree_about_being_lit() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_a = DAEMON.items.spawn('hooded_lantern', nil)").unwrap();
    vm.eval("_b = DAEMON.items.spawn('hooded_lantern', nil)").unwrap();
    vm.eval("set_object_state(_a.id, 'lit', true) return 'lit'").unwrap();

    vm.eval("_L = require('lib.light')").unwrap();
    assert_eq!(
        vm.eval("return tostring(_L.is_lit(_a, DAEMON.items.resolve(_a)))").unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval("return tostring(_L.is_lit(_b, DAEMON.items.resolve(_b)))").unwrap(),
        "false",
        "lighting one lantern lit every lantern in the game"
    );
}

/// Weather and light are the same scale, so a fogbound marsh and a mine ask the
/// same question of the same code.
#[test]
fn a_carried_light_beats_a_dark_room_whatever_made_it_dark() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_L = require('lib.light')").unwrap();
    vm.eval("_room = DAEMON.world.get_room('collapsed_mine.first_level')").unwrap();
    vm.eval(
        "_p = { char_id = 750, inventory = {}, equipment = {}, send = function() end }",
    )
    .unwrap();

    assert_eq!(vm.eval("return tostring(_L.can_see(_p, _room))").unwrap(), "false");

    vm.eval("_l = DAEMON.items.spawn('hooded_lantern', nil) \
             table.insert(_p.inventory, _l) return 'carried'")
        .unwrap();
    assert_eq!(
        vm.eval("return tostring(_L.can_see(_p, _room))").unwrap(),
        "false",
        "carrying an unlit lantern is not carrying a light"
    );

    vm.eval("set_object_state(_l.id, 'lit', true) return 'lit'").unwrap();
    assert_eq!(vm.eval("return tostring(_L.can_see(_p, _room))").unwrap(), "true");

    // Equipped counts as well as carried: insisting it be in the `light` slot
    // is a rule players discover by dying in the dark.
    vm.eval("_p.inventory = {} _p.equipment.light = _l return 'worn'").unwrap();
    assert_eq!(vm.eval("return tostring(_L.can_see(_p, _room))").unwrap(), "true");
}

/// The locked grille: two routes in, and object state remembers which.
#[test]
fn the_grille_opens_with_a_key_or_with_skill() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("spawn hooded_lantern");
    vm.command("use lantern");
    go_to(&mut vm, "collapsed_mine.second_level");

    assert!(vm.command("look").contains("grille is down"), "expected it shut");

    let out = vm.command("open");
    assert!(out.contains("size of a thumb"), "expected a locked refusal:\n{out}");

    // The lockpick route needs dexterity as well as the pick.
    vm.command("spawn iron_lockpick");
    let out = vm.command("open");
    assert!(
        out.contains("pick comes out again"),
        "dexterity 10 should not be enough:\n{out}"
    );

    vm.command("affect learn dexterity 15");
    let out = vm.command("open");
    assert!(out.contains("Three wards"), "the pick should work at 15:\n{out}");
    assert!(vm.command("look").contains("grille is up"));

    // The exit's `check` was refusing before and allows now.
    let out = vm.command("west");
    assert!(!out.contains("grille is down"), "the exit is still barred:\n{out}");
}

/// The lever puzzle: an order, a wrong-order reset, and a timed reset on the
/// ticker replacing itself by id.
#[test]
fn the_levers_have_an_order_and_a_timeout() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("spawn hooded_lantern");
    vm.command("use lantern");
    go_to(&mut vm, "collapsed_mine.pump_house");

    assert!(vm.command("look").contains("All three levers stand upright"));

    // Wrong first lever is a no-op rather than a reset — there is nothing to
    // reset yet, and saying "it resets" would be a lie.
    let out = vm.command("pull middle");
    assert!(out.contains("comes straight back"), "{out}");

    vm.command("pull left");
    assert!(vm.command("look").contains("1 of the levers"), "step one did not stick");

    // Wrong second lever undoes it, loudly.
    let out = vm.command("pull right");
    assert!(out.contains("lets go"), "a wrong lever should undo the sequence:\n{out}");
    assert!(vm.command("look").contains("All three levers stand upright"));

    // Right order.
    vm.command("pull left");
    vm.command("pull middle");
    let out = vm.command("pull right");
    assert!(out.contains("pump takes"), "the sequence did not finish:\n{out}");
    assert!(vm.command("look").contains("engine is working"));

    // The timed reset is disarmed once it is running.
    assert!(
        !vm.command("tasks").contains("mine.levers.reset"),
        "the reset timer should be cancelled once the pump is on"
    );

    // And the shaft is open, which is what the puzzle was for.
    go_to(&mut vm, "collapsed_mine.deep_workings");
    assert!(vm.command("look").contains("water has gone down"));
}

/// The one timer, re-armed rather than stacked. Pulling the first lever twice
/// must not leave two resets pending.
#[test]
fn the_lever_timer_replaces_itself_rather_than_stacking() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("set_object_state('collapsed_mine.pump_house', 'lever_step', 0)").unwrap();
    vm.eval("set_object_state('collapsed_mine.pump_house', 'pump_running', false)").unwrap();

    let pending = |vm: &mut RealVm| -> i64 {
        vm.eval(
            "local n = 0 for _, t in ipairs(DAEMON.ticker.list()) do \
             local id = type(t) == 'table' and t.id or t \
             if id == 'mine.levers.reset' then n = n + 1 end end return n",
        )
        .unwrap()
        .parse()
        .unwrap()
    };
    assert_eq!(pending(&mut vm), 0);

    // Arm it twice through the ticker directly, which is what two `pull left`s
    // in a row do.
    vm.eval("DAEMON.ticker.after(60, 'mine.levers.reset', function() end)").unwrap();
    vm.eval("DAEMON.ticker.after(60, 'mine.levers.reset', function() end)").unwrap();
    assert_eq!(
        pending(&mut vm),
        1,
        "arming a timer by the same id twice should replace, not stack — this \
         is the shape the ticker_d leak had"
    );
}

/// An area reset clears the puzzle. That is what a reset is *for*, and it is
/// the contrast with the marsh's daily gate, which must survive one.
#[test]
fn an_area_reset_clears_the_puzzle_but_not_a_daily_gate() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("spawn hooded_lantern");
    vm.command("use lantern");

    // Solve the puzzle and work the seam.
    go_to(&mut vm, "collapsed_mine.pump_house");
    vm.command("pull left");
    vm.command("pull middle");
    vm.command("pull right");
    assert!(vm.command("look").contains("engine is working"));

    go_to(&mut vm, "collapsed_mine.first_level");
    assert!(vm.command("mine").contains("comes away with it"));
    assert!(vm.command("mine").contains("worked out"));

    // And take the marsh's daily.
    go_to(&mut vm, "greywater_marsh.herb_beds");
    vm.command("gather");
    assert!(vm.command("gather").contains("picked over"));

    vm.command("areas reset collapsed_mine");
    vm.command("areas reset greywater_marsh");

    // The seam is back — shared world state, and refilling it is correct.
    go_to(&mut vm, "collapsed_mine.first_level");
    assert!(
        vm.command("mine").contains("comes away with it"),
        "an area reset should refill a shared seam"
    );

    // The pump has stopped, because that is world state too.
    go_to(&mut vm, "collapsed_mine.pump_house");
    assert!(
        vm.command("look").contains("levers stand upright"),
        "an area reset should clear the puzzle"
    );

    // The daily gate has not, because it is per character and lives in a
    // cooldown rather than on a room.
    go_to(&mut vm, "greywater_marsh.herb_beds");
    assert!(
        vm.command("gather").contains("picked over"),
        "the daily gate was reset with the area — that is the bug"
    );
}

/// The boss drops a **corpse**, which is the third kind of container: not
/// carried, not fixed, and it goes away.
#[test]
fn the_boss_leaves_a_corpse_with_its_loot_in_it() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    let in_room = |vm: &mut RealVm| -> i64 {
        vm.eval("return #DAEMON.items.in_room('collapsed_mine.the_sump')")
            .unwrap()
            .parse()
            .unwrap()
    };
    assert_eq!(in_room(&mut vm), 0);

    // Listening before the kill, because the death is the only time it fires.
    vm.eval(
        "_heard = false DAEMON.event.on('area.collapsed_mine.delver_slain', 't', \
         function() _heard = true end) return 'listening'",
    )
    .unwrap();

    vm.eval(
        "for _, m in ipairs(DAEMON.mobs.in_room('collapsed_mine.the_sump')) do \
         if m.template_id == 'the_delver' then _boss = m end end return 'found'",
    )
    .unwrap();
    assert_eq!(vm.eval("return tostring(_boss ~= nil)").unwrap(), "true");

    vm.eval("_boss:take_damage(9999) return 'dead'").unwrap();

    assert_eq!(in_room(&mut vm), 1, "expected exactly one corpse on the floor");
    vm.eval("_corpse = DAEMON.items.in_room('collapsed_mine.the_sump')[1]").unwrap();
    assert_eq!(vm.eval("return _corpse.template").unwrap(), "delver_corpse");

    // The loot is *in* it, not scattered. Six items on a floor is a wall of
    // text; a corpse is one line and a decision.
    assert_eq!(
        vm.eval("return #DAEMON.items.contents(_corpse.id)").unwrap(),
        "4",
        "the boss's loot should be inside the corpse"
    );

    // Unlimited capacity, so a boss that drops eleven things does not silently
    // lose the eleventh.
    assert_eq!(
        vm.eval("return DAEMON.items.resolve(_corpse).container.capacity").unwrap(),
        "0"
    );

    // And the area-wide event fired, which is `signals.md`'s worked example: a
    // game reacting to a boss dying without combat knowing it can be reacted to.
    assert_eq!(
        vm.eval("return tostring(_heard)").unwrap(),
        "true",
        "the boss's death should announce itself to the area"
    );

    // The corpse rots on its own timer, armed by instance id so a second boss's
    // corpse does not replace this one's.
    assert_eq!(
        vm.eval(
            "for _, t in ipairs(DAEMON.ticker.list()) do \
             local id = type(t) == 'table' and t.id or t \
             if id == 'corpse.rot.' .. _corpse.id then return 'armed' end end return 'missing'"
        )
        .unwrap(),
        "armed"
    );
}

/// The boss's curse survives dying, which is what makes it a curse.
#[test]
fn the_delvers_regard_survives_death() {
    let mut vm = RealVm::boot_real_mudlib(0);

    vm.command("affect apply delvers_regard 1200");
    assert!(vm.command("affect list").contains("delvers_regard"));

    vm.command("affect damage 9999");
    assert!(
        vm.command("affect list").contains("delvers_regard"),
        "a curse you can remove by dying is not a curse"
    );
}
