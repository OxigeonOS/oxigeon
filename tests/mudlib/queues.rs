//! Action queues and roundtime.
//!
//! The requirement this whole daemon exists to keep is the first test in the
//! file: **roundtime never gates a command.** `look`, `say` and `who` work while
//! you are recovering — not because of an exemption list, but because nothing in
//! command dispatch reads a track, so they never enter the code path at all.
//!
//! Everything else follows from three sentences:
//!
//! * roundtime is **recovery, not occupation** — `ability_d`'s `cast_time`
//!   already owns "you are busy", so the two need no arbitration;
//! * roundtime lives on a **track**, and only actions on that track consult it;
//! * the queue holds **intent**; the cast record holds the thing in flight.
//!
//! Fixture world only.

use crate::common::RealVm;

fn one_line(src: &str) -> String {
    src.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ")
}

struct Vm(RealVm);

impl Vm {
    fn new() -> Self {
        let mut vm = RealVm::boot_fixture_with_probe();
        vm.eval(&one_line(
            "Q = DAEMON.queue A = DAEMON.ability
             _p = { char_id = 801, id = 'p801', name = 'Probe', stats = {},
                    equipment = {}, room_id = 'fixture.hall', _sent = {} }
             function _p:send(t) self._sent[#self._sent+1] = tostring(t) end
             function _p:message_room(t) end
             setmetatable(_p, { __index = require('lib.mobile') })
             DAEMON.trait.seed(_p, 'character')
             DAEMON.trait.set_cur(_p, 'mp', _p:trait('max_mp'))
             DAEMON.trait.set_cur(_p, 'hp', _p:trait('max_hp'))
             return 'ready'",
        ))
        .unwrap();
        Self(vm)
    }

    fn run(&mut self, src: &str) -> String {
        self.0.eval(&one_line(src)).unwrap()
    }
}

/// **The requirement.** Roundtime gates a track, never a command.
#[test]
fn roundtime_never_gates_an_ordinary_command() {
    let mut vm = RealVm::boot_with_fixture_world(0);
    vm.command("pagesize 0");

    // Put the player deep in combat roundtime.
    vm.lua(
        "local p = get_player(SESSION) \
         DAEMON.cooldown.mark(p, 'rt.combat', 30) \
         return tostring(DAEMON.queue.in_roundtime(p, 'combat'))",
    );
    assert_eq!(
        vm.lua("local p = get_player(SESSION) return tostring(DAEMON.queue.in_roundtime(p, 'combat'))"),
        "true"
    );

    // Everything that is not an action on that track keeps working.
    assert!(vm.command("look").contains("A Plain Hall"), "look must work in roundtime");
    assert!(vm.command("say hello").contains("hello"), "say must work in roundtime");
    assert!(!vm.command("who").is_empty(), "who must work in roundtime");
    assert!(vm.command("inventory").len() > 0, "inventory must work in roundtime");
    assert!(vm.command("score").contains("Health"), "score must work in roundtime");

    // Movement too — a channel that pinned you in place would be a trap.
    assert!(vm.command("north").contains("Store Room"), "you can still walk");
}

/// An ability that has left the track is never gated by it.
#[test]
fn an_ability_may_leave_the_track_entirely() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        A.define({ id = "probe_shout", category = "technique", open = true,
                   target = "none", track = "none", messages = { self = "You shout." } })
        DAEMON.cooldown.mark(_p, "rt.combat", 30)
        local ok, why, status = A.use(_p, "probe_shout")
        return tostring(ok) .. "|" .. tostring(why) .. "|" .. tostring(status)
        "#,
    );
    assert_eq!(out, "true|nil|done", "a trackless ability runs while the track recovers");
}

/// Roundtime is an ordinary `cooldown_d` entry, in the memory tier.
#[test]
fn a_roundtime_is_a_cooldown_under_a_known_key() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        Q.mark(_p, "combat", 4)
        local fast = DAEMON.cache.get("cooldowns_fast", 801, "rt.combat")
        local durable = DAEMON.cache.get("cooldowns", 801, "rt.combat")
        local listed = false
        for _, e in ipairs(DAEMON.cooldown.list(_p)) do
            if e.what == "rt.combat" then listed = true end
        end
        return tostring(fast ~= nil) .. "|" .. tostring(durable) .. "|" .. tostring(listed)
        "#,
    );
    assert_eq!(
        out, "true|nil|true",
        "memory tier, and visible in `cooldown` so a player can see why they cannot swing"
    );
}

/// Two tracks keep separate roundtimes — the test of whether this is generic.
#[test]
fn two_tracks_keep_separate_roundtimes() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        Q.define_track("crafting", { round_trait = "craft_round_length", round_seconds = 6,
                                     resolve = function() return true end })
        Q.mark(_p, "combat", 5)
        return tostring(Q.in_roundtime(_p, "combat")) .. "|"
            .. tostring(Q.in_roundtime(_p, "crafting"))
        "#,
    );
    assert_eq!(out, "true|false", "a track registered by a game gates only itself");

    // And it uses its own round trait, so a crafting round is not a combat one.
    let out = vm.run(
        r#"return string.format("%.1f|%.1f",
             Q.round_length(_p, "combat"), Q.round_length(_p, "crafting"))"#,
    );
    assert_eq!(out, "3.0|6.0");
}

/// An absent round trait falls back **and says so**, rather than silently
/// producing a roundtime of zero.
///
/// The absence is arranged rather than assumed: the fixture world defines a
/// `round_length` now, because a world with no clock makes every test of pacing
/// pass by measuring a constant. So this registers a track naming a trait
/// nothing defines, which is the condition the fallback is actually for.
#[test]
fn an_absent_round_length_falls_back_and_says_so() {
    let mut vm = Vm::new();

    vm.run(
        r#"Q.define_track("dreaming", { round_trait = "no_such_trait",
             round_seconds = 7, empty = "idle",
             resolve = function() return false end })
           return "registered""#,
    );

    let out = vm.run(r#"return tostring(DAEMON.trait.has(_p, "no_such_trait"))"#);
    assert_eq!(out, "false", "this test needs a trait nothing defines");

    let out = vm.run(r#"return string.format("%.1f", Q.round_length(_p, "dreaming"))"#);
    assert_eq!(out, "7.0", "so it falls back to the track's configured round");

    // `journal.recent` hands back raw JSON lines, not tables.
    let warned = vm.run(
        "local found = false \
         for _, line in ipairs(DAEMON.journal.recent(200, 'warn') or {}) do \
             if tostring(line):find('no_such_trait', 1, true) then found = true end \
         end return tostring(found)",
    );
    assert_eq!(warned, "true", "and it warns, because a silent zero would be a wrong answer");
}

/// `{ rounds = n }` reaches roundtime through the trait, and equipment moves it.
#[test]
fn rounds_resolve_against_the_characters_round_length() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        DAEMON.trait.define({ id = "round_length", label = "Round", kind = "attribute",
                              group = "derived", default = 4, min = 1, sets = false })
        DAEMON.trait.seal()
        DAEMON.trait.set_base(_p, "round_length", 4)
        Q.mark(_p, "combat", { rounds = 0.75 })
        local a = math.ceil(Q.roundtime(_p, "combat"))
        DAEMON.cooldown.clear(_p, "rt.combat")
        DAEMON.trait.set_base(_p, "round_length", 8)
        Q.mark(_p, "combat", { rounds = 0.75 })
        local b = math.ceil(Q.roundtime(_p, "combat"))
        return a .. "|" .. b
        "#,
    );
    assert_eq!(out, "3|6", "three quarters of a four-second round, then of an eight-second one");
}

/// An ability used in roundtime is **queued, not refused** — and nothing is
/// spent by queueing it.
#[test]
fn an_ability_used_in_roundtime_is_queued_not_refused() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        local m = DAEMON.mobs.spawn("fixture_mouse", "fixture.hall")
        DAEMON.cooldown.mark(_p, "rt.combat", 10)
        local before = _p:trait("mp")
        local ok, why, status = A.use(_p, "fixture_strike", { target = m })
        return tostring(ok) .. "|" .. tostring(status) .. "|" .. #Q.list(_p, "combat")
            .. "|" .. tostring(before == _p:trait("mp"))
        "#,
    );
    assert_eq!(out, "true|queued|1|true", "queued, and not a point of mana spent yet");
}

/// **The invariant, under roundtime.** A mistyped target still refuses
/// immediately rather than queueing something doomed.
#[test]
fn a_mistyped_target_is_never_queued() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        DAEMON.cooldown.mark(_p, "rt.combat", 10)
        local before = _p:trait("mp")
        local ok, why = A.use(_p, "fixture_strike", { target = "nosuchthing" })
        return tostring(ok) .. "|" .. tostring(why) .. "|" .. #Q.list(_p, "combat")
            .. "|" .. tostring(before == _p:trait("mp"))
        "#,
    );
    assert_eq!(
        out, "false|There is no nosuchthing here.|0|true",
        "the enqueue sits below target resolution, so a typo still costs nothing"
    );
}

/// **A cooldown queues too**, and this test used to assert the opposite.
///
/// The old rule was that only roundtime enqueued: a cooldown said "not this,
/// for a while" and roundtime said "not yet, but soon and certainly". That is a
/// true distinction and not one the player is in a position to make — from the
/// seat, "Not yet. (1s)" and "You will strike next" are the same situation, and
/// two behaviours out of one intent reads as the game being arbitrary.
///
/// What still refuses is being *unable* rather than waiting: see
/// `a_mistyped_target_is_never_queued` above and
/// `something_you_cannot_afford_is_still_refused` below.
#[test]
fn a_cooldown_queues_rather_than_refusing() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        local m = DAEMON.mobs.spawn("fixture_mouse", "fixture.hall")
        A.use(_p, "fixture_strike", { target = m })
        DAEMON.cooldown.clear(_p, "rt.combat")
        local ok, why, tag = A.use(_p, "fixture_strike", { target = m })
        return tostring(ok) .. "|" .. tostring(tag) .. "|" .. #Q.list(_p, "combat")
        "#,
    );
    assert_eq!(
        out, "true|queued|1",
        "off roundtime but on cooldown should still queue: {out}"
    );
}

/// The queue is bounded, and refuses the newest rather than dropping a promise.
#[test]
fn a_full_queue_refuses_the_newest() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        for i = 1, 4 do Q.enqueue(_p, "combat", { kind = "ability", id = "fixture_strike" }) end
        local ok, why = Q.enqueue(_p, "combat", { kind = "ability", id = "fixture_slow" })
        local ids = {}
        for _, e in ipairs(Q.list(_p, "combat")) do ids[#ids+1] = e.id end
        return tostring(ok) .. "|" .. #ids .. "|" .. table.concat(ids, ",")
        "#,
    );
    assert_eq!(out, "false|3|fixture_strike,fixture_strike,fixture_strike");
}

/// The queue advances only when the roundtime expires.
#[test]
fn the_queue_advances_only_when_the_roundtime_expires() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        local m = DAEMON.mobs.spawn("fixture_mouse", "fixture.hall")
        DAEMON.cooldown.mark(_p, "rt.combat", 30)
        A.use(_p, "fixture_strike", { target = m })
        local queued = #Q.list(_p, "combat")
        local blocked = Q.advance(_p, "combat")
        DAEMON.cooldown.clear(_p, "rt.combat")
        local ran = Q.advance(_p, "combat")
        return queued .. "|" .. tostring(blocked) .. "|" .. tostring(ran)
            .. "|" .. #Q.list(_p, "combat")
        "#,
    );
    assert_eq!(out, "1|false|true|0");
}

/// A cast occupies the character, and the queue waits for it.
#[test]
fn a_cast_occupies_the_character_and_the_queue_waits() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        A.use(_p, "fixture_chant")
        Q.enqueue(_p, "combat", { kind = "ability", id = "fixture_slow" })
        local while_casting = Q.advance(_p, "combat")
        local record = A.casting(_p)
        A._complete(_p, record)
        local after = Q.advance(_p, "combat")
        return tostring(while_casting) .. "|" .. tostring(after)
        "#,
    );
    assert_eq!(out, "false|true", "occupation and recovery are different things");
}

/// An interrupted cast owes no roundtime and keeps what was queued behind it.
#[test]
fn an_interrupted_cast_marks_no_roundtime_and_keeps_the_queue() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        A.define({ id = "probe_slow_cast", category = "technique", open = true,
                   target = "none", cast_time = 5, roundtime = 4,
                   messages = { self = "Done." } })
        A.use(_p, "probe_slow_cast")
        Q.enqueue(_p, "combat", { kind = "ability", id = "fixture_slow" })
        A.on_damaged(_p, 10, {})
        return tostring(A.casting(_p) ~= nil) .. "|"
            .. math.ceil(Q.roundtime(_p, "combat")) .. "|" .. #Q.list(_p, "combat")
        "#,
    );
    assert_eq!(out, "false|0|1", "no recovery owed for a working that never landed");
}

/// A completed action owes its roundtime.
#[test]
fn a_completed_action_owes_its_roundtime() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        A.define({ id = "probe_heavy_swing", category = "technique", open = true,
                   target = "none", roundtime = { rounds = 1 },
                   messages = { self = "Swung." } })
        local ok = A.use(_p, "probe_heavy_swing")
        return tostring(ok) .. "|" .. math.ceil(Q.roundtime(_p, "combat"))
        "#,
    );
    assert_eq!(out, "true|3", "one round, and the fixture's fallback round is three seconds");
}

/// A stale entry is dropped rather than replayed a minute later.
#[test]
fn a_stale_entry_is_dropped_at_dequeue() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        Q.enqueue(_p, "combat", { kind = "ability", id = "fixture_slow" })
        for _, e in ipairs(Q.list(_p, "combat")) do e.at = e.at - 120 end
        local ran = Q.advance(_p, "combat")
        return tostring(ran) .. "|" .. #Q.list(_p, "combat")
        "#,
    );
    assert_eq!(out, "false|0", "dropped, not run");
}

/// History records what completed, newest first, with no entity references.
#[test]
fn history_records_what_completed() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        Q.enqueue(_p, "combat", { kind = "ability", id = "fixture_slow" })
        Q.advance(_p, "combat")
        local h = Q.history(_p, "combat")
        return #h .. "|" .. tostring(h[1] and h[1].id) .. "|"
            .. tostring(h[1] and h[1].target)
        "#,
    );
    assert_eq!(out, "1|fixture_slow|nil", "and the target did not come along for the ride");
}

/// A creature's queue is dropped when it despawns; a player's on disconnect.
#[test]
fn a_queue_does_not_outlive_what_it_belonged_to() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        local m = DAEMON.mobs.spawn("fixture_mouse", "fixture.hall")
        Q.enqueue(m, "combat", { kind = "attack" })
        local before = #Q.list(m, "combat")
        DAEMON.mobs.despawn(m)
        local gone = DAEMON.cache.get("queue", "obj:" .. m.id, "combat")
        Q.enqueue(_p, "combat", { kind = "ability", id = "fixture_slow" })
        Q.cleanup(801)
        return before .. "|" .. tostring(gone) .. "|" .. #Q.list(_p, "combat")
        "#,
    );
    assert_eq!(out, "1|nil|0");
}

// ═══════════════════════════════════════════════════════════════════════════
//  Waiting enqueues; being unable refuses
// ═══════════════════════════════════════════════════════════════════════════

/// **A cooldown queues rather than refusing.**
///
/// The rule was that only roundtime enqueued: a cooldown said "not this, for a
/// while" and roundtime said "not yet, but soon and certainly". That is a true
/// distinction and not one the player is in a position to make — from the seat,
/// "Not yet. (1s)" and "You will strike next" are the same situation, and
/// getting two different behaviours out of one intent reads as arbitrary.
#[test]
fn an_ability_on_cooldown_is_queued_rather_than_refused() {
    let mut vm = Vm::new();

    // Off cooldown and free: it just happens.
    let first = vm.run(        "_r = DAEMON.mobs.spawn('fixture_mouse', 'fixture.hall') \
         local ok, err, tag = DAEMON.ability.use(_p, 'fixture_strike', { target = _r }) \
         return tostring(ok) .. '|' .. tostring(tag)",
    );
    assert_eq!(first, "true|done", "the first use should resolve, not queue");

    // Still on its four-second cooldown: queued, not refused.
    let second = vm.run(        "local ok, err, tag = DAEMON.ability.use(_p, 'fixture_strike', { target = _r }) \
         return tostring(ok) .. '|' .. tostring(tag) .. '|' .. tostring(err)",
    );
    assert_eq!(
        second, "true|queued|nil",
        "an ability on cooldown should queue"
    );
    assert_eq!(
        vm.run("return tostring(#DAEMON.queue.list(_p, 'combat'))"),
        "1"
    );
}

/// **A queued entry that is still cooling is put back, not dropped.**
///
/// `advance` pops the head before resolving, so a resolver returning false
/// loses the entry. Letting a cooldown enqueue without this would mean the
/// player queues an ability and nothing ever happens — the worst of both.
#[test]
fn a_queued_entry_survives_a_cooldown_that_has_not_cleared() {
    let mut vm = Vm::new();

    vm.run(        "_r = DAEMON.mobs.spawn('fixture_mouse', 'fixture.hall') \
         DAEMON.ability.use(_p, 'fixture_strike', { target = _r }) \
         DAEMON.ability.use(_p, 'fixture_strike', { target = _r }) \
         return 'queued'",
    );
    assert_eq!(vm.run("return tostring(#DAEMON.queue.list(_p, 'combat'))"), "1");

    // Free of roundtime but still cooling: the tick must not consume it.
    vm.run("DAEMON.cooldown.clear(_p, 'rt.combat') return 'free'");
    vm.run("DAEMON.queue.advance(_p, 'combat') return 'ticked'");
    assert_eq!(
        vm.run("return tostring(#DAEMON.queue.list(_p, 'combat'))"),
        "1",
        "the entry was popped and dropped — the player's intent vanished"
    );

    // Once the cooldown clears it runs, and leaves the queue.
    vm.run("DAEMON.cooldown.clear(_p, 'technique.fixture_strike') return 'ready'");
    vm.run("DAEMON.queue.advance(_p, 'combat') return 'ticked'");
    assert_eq!(
        vm.run("return tostring(#DAEMON.queue.list(_p, 'combat'))"),
        "0",
        "it should have run once it could"
    );
}

/// Being *unable* still refuses: a cost you cannot pay is not a wait.
#[test]
fn something_you_cannot_afford_is_still_refused() {
    let mut vm = Vm::new();

    let out = vm.run(        "_r = DAEMON.mobs.spawn('fixture_mouse', 'fixture.hall') \
         DAEMON.trait.set_cur(_p, 'mp', 0) \
         local ok, err, tag = DAEMON.ability.use(_p, 'fixture_strike', { target = _r }) \
         return tostring(ok) .. '|' .. tostring(tag)",
    );
    assert!(out.starts_with("false|"), "an unaffordable cost should refuse: {out}");
    assert_eq!(vm.run("return tostring(#DAEMON.queue.list(_p, 'combat'))"), "0");
}
