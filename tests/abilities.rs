//! Abilities: cost, cooldown, target, outcomes, timing — as data.
//!
//! `spell_d` was 177 lines arranging five things the mudlib already had into
//! "casting", with every spell inside it a hand-written function. `ability_d` is
//! that arrangement, so a designer writes a data bag instead. What has to stay
//! true while that is so:
//!
//! * **nothing is spent until the target resolves.** A mistyped name costing
//!   mana is the failure `spell_d`'s ordering existed to prevent, and it is the
//!   easiest thing to lose when the flow grows steps;
//! * **an ability's damage goes through `take_damage`**, so armour, resists and
//!   the whole `damage_taken` pipeline meet it exactly as they meet a sword. An
//!   ability that bypassed it would be the one thing nothing could be designed
//!   against;
//! * **an interrupt costs you.** Cost at the start, cooldown at completion. A
//!   cast you can begin for free and abort for free is a free oracle.
//!
//! Fixture world only — nothing here knows this repository ships a game.

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
        vm.eval(
            "A = DAEMON.ability AB = require('lib.abilities') \
             _p = { char_id = 901, id = 'p901', name = 'Probe', stats = {}, \
                    equipment = {}, _sent = {} } \
             function _p:send(t) self._sent[#self._sent+1] = tostring(t) end \
             function _p:message_room(t) end \
             setmetatable(_p, { __index = require('lib.mobile') }) \
             DAEMON.trait.seed(_p, 'character') \
             DAEMON.trait.set_cur(_p, 'mp', _p:trait('max_mp')) \
             DAEMON.trait.set_cur(_p, 'hp', _p:trait('max_hp')) return 'ready'",
        )
        .unwrap();
        Self(vm)
    }

    fn run(&mut self, src: &str) -> String {
        self.0.eval(&one_line(src)).unwrap()
    }
}

// ─── The load-bearing invariant ──────────────────────────────────────────────

/// **Nothing is spent when the target does not resolve.**
///
/// Inherited from `spell_d`, whose comment says it outright: *resolve the target
/// before spending anything, so a mistyped name does not cost mana*. The flow
/// grew six steps around it and this is what keeps it true.
#[test]
fn a_mistyped_target_costs_nothing() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        local before = _p:trait("mp")
        local ok, why = A.use(_p, "fixture_strike", { target = "nosuchthing" })
        local after = _p:trait("mp")
        return tostring(ok) .. "|" .. tostring(why) .. "|" .. tostring(before == after)
            .. "|" .. tostring(DAEMON.cooldown.remaining(_p, "technique.fixture_strike"))
        "#,
    );

    assert_eq!(
        out, "false|There is no nosuchthing here.|true|0",
        "the refusal names the target, no mana moved, and no cooldown was marked"
    );
}

/// A gate refuses with its own reason, and a `why` on the spec overrides it.
#[test]
fn a_requirement_refuses_in_its_own_words() {
    let mut vm = Vm::new();

    // Not known at all: no `open`, no rank trait value, no grant.
    let out = vm.run(r#"local ok, why = A.use(_p, "fixture_taught") return tostring(why)"#);
    assert_eq!(out, "You do not know any such thing.");

    // Known, but below `min_rank`.
    let out = vm.run(
        r#"DAEMON.trait.set_base(_p, "fixture_skill", 1)
           local ok, why = A.use(_p, "fixture_taught") return tostring(why)"#,
    );
    assert_eq!(out, "That is beyond you for now.", "presence and rank are separate refusals");

    let out = vm.run(
        r#"DAEMON.trait.set_base(_p, "fixture_skill", 2)
           local ok = A.use(_p, "fixture_taught") return tostring(ok)"#,
    );
    assert_eq!(out, "true");

    // A `why` on the requirement wins over the predicate's default.
    let out = vm.run(
        r#"
        A.define({ id = "probe_gated", category = "technique", open = true, target = "none",
                   requires = { { kind = "trait", id = "level", min = 99,
                                  why = "You are nowhere near ready." } } })
        local ok, why = A.use(_p, "probe_gated") return tostring(why)
        "#,
    );
    assert_eq!(out, "You are nowhere near ready.");
}

/// Damage goes through `take_damage`, so an effect that reduces damage reduces
/// an ability's damage — with no special case anywhere.
#[test]
fn declarative_damage_meets_the_damage_pipeline() {
    let mut vm = Vm::new();

    let plain = vm.run(
        r#"
        local m = DAEMON.mobs.spawn("fixture_mouse", "fixture.hall")
        local before = m:trait("hp")
        A.use(_p, "fixture_strike", { target = m })
        return tostring(before - m:trait("hp"))
        "#,
    );
    assert_eq!(plain, "7", "the authored 7 lands unmitigated");

    let warded = vm.run(
        r#"
        DAEMON.cooldown.clear(_p, "technique.fixture_strike")
        local m = DAEMON.mobs.spawn("fixture_mouse", "fixture.cellar")
        DAEMON.effect.apply(m, "fixture_ward", { source = "probe" })
        local before = m:trait("hp")
        A.use(_p, "fixture_strike", { target = m })
        return tostring(before - m:trait("hp"))
        "#,
    );
    assert_eq!(
        warded, "4",
        "the ward's flat 3 came off, because the ability went through the same \
         pipeline a sword does"
    );
}

/// The cooldown tier is chosen by duration, and the key is `<category>.<id>`.
#[test]
fn a_cooldown_lands_in_the_tier_its_duration_earns() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        A.use(_p, "fixture_strike", { target = DAEMON.mobs.spawn("fixture_mouse", "fixture.hall") })
        A.use(_p, "fixture_slow")
        local fast = DAEMON.cache.get("cooldowns_fast", 901, "technique.fixture_strike")
        local slow = DAEMON.cache.get("cooldowns", 901, "technique.fixture_slow")
        local wrong_a = DAEMON.cache.get("cooldowns", 901, "technique.fixture_strike")
        local wrong_b = DAEMON.cache.get("cooldowns_fast", 901, "technique.fixture_slow")
        return tostring(fast ~= nil) .. "|" .. tostring(slow ~= nil)
            .. "|" .. tostring(wrong_a) .. "|" .. tostring(wrong_b)
        "#,
    );
    assert_eq!(
        out, "true|true|nil|nil",
        "4s is a game mechanic and lives in memory; 90s is a promise and is written"
    );

    // And it actually gates.
    let out = vm.run(
        r#"local ok, why = A.use(_p, "fixture_slow") return tostring(ok) .. "|" .. tostring(why)"#,
    );
    assert!(out.starts_with("false|Not yet."), "{out}");
}

/// A cost that is not a gauge is refused at define time.
///
/// The mirror of `effect_d` refusing a modifier aimed at a gauge, inverted: a
/// cost against an attribute is a modifier pretending to be a payment, which is
/// the mistake the whole effect design exists to avoid.
#[test]
fn a_cost_must_name_a_gauge() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        local ok = A.define({ id = "probe_bad_cost", target = "none",
                              cost = { strength = 2 } })
        return tostring(ok) .. "|" .. tostring(A.get("probe_bad_cost"))
        "#,
    );
    assert_eq!(out, "false|nil");
}

/// A signed gauge delta, applied after the outcome.
///
/// Not a cost, and the difference is the sign. `cost` is what you must have to
/// begin and can only subtract; `adjust` is what doing the thing did to you, and
/// a footing resource needs it to go both ways — a heavy swing spends balance
/// and a recovery step restores it.
#[test]
fn an_adjust_moves_a_gauge_in_either_direction() {
    let mut vm = Vm::new();

    // A footing pool rests at its neutral point rather than at zero.
    let out = vm.run(
        r#"DAEMON.trait.set_cur(_p, "balance", 10)
           A.use(_p, "fixture_heavy")
           return tostring(_p:trait("balance"))"#,
    );
    assert_eq!(out, "5", "a heavy action spends footing");

    let out = vm.run(
        r#"local mp = _p:trait("mp")
           A.use(_p, "fixture_recover")
           return tostring(_p:trait("balance")) .. "|" .. tostring(mp - _p:trait("mp"))"#,
    );
    assert_eq!(out, "8|1", "and a recovery restores it, while another gauge falls");
}

/// It clamps to the gauge's bounds rather than running past them.
#[test]
fn an_adjust_clamps_to_the_gauge() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"DAEMON.trait.set_cur(_p, "balance", 2)
           A.use(_p, "fixture_heavy")
           return tostring(_p:trait("balance"))"#,
    );
    assert_eq!(out, "0", "spending more than you have floors at the minimum");

    let out = vm.run(
        r#"DAEMON.trait.set_cur(_p, "balance", 20)
           A.use(_p, "fixture_recover")
           return tostring(_p:trait("balance"))"#,
    );
    assert_eq!(out, "20", "and restoring past the ceiling stops there");
}

/// An adjust may only name a gauge — the mirror of the rule for `cost`.
///
/// `trait.adjust` moves a *current* value and only a gauge has one. Moving an
/// attribute from here would be a permanent change wearing an action's clothes;
/// that is what an effect is for.
#[test]
fn an_adjust_must_name_a_gauge() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"local ok = A.define({ id = "probe_bad_adjust", target = "none",
                                 adjust = { strength = 2 } })
           return tostring(ok) .. "|" .. tostring(A.get("probe_bad_adjust"))"#,
    );
    assert_eq!(out, "false|nil");
}

/// It sits with the other declarative outcomes, so `run` sees the result of it.
///
/// The fixed order is roll → announce → remove → damage → heal → apply →
/// **adjust** → result → run → engage. `run` is the escape hatch and it runs
/// last on purpose: an ability that wants to react to what it just did should
/// be looking at final state, not at a half-applied one.
#[test]
fn an_adjust_lands_with_the_outcomes_and_before_run() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        A.define({ id = "probe_order", category = "technique", open = true,
                   target = "none", adjust = { balance = -4 },
                   run = function(ctx) ctx.user._seen = ctx.user:trait("balance") end })
        DAEMON.trait.set_cur(_p, "balance", 10)
        A.use(_p, "probe_order")
        return tostring(_p._seen) .. "|" .. tostring(_p:trait("balance"))
        "#,
    );
    assert_eq!(out, "6|6", "`run` sees the adjust already applied");
}

// ─── Ownership ───────────────────────────────────────────────────────────────

/// Grants reconcile by source, idempotently — `set_source_effects`' contract,
/// word for word.
#[test]
fn abilities_granted_by_a_source_come_and_go_with_it() {
    let mut vm = Vm::new();

    assert_eq!(vm.run(r#"return tostring(A.knows(_p, "fixture_granted"))"#), "false");

    let out = vm.run(
        r#"
        local a, r = A.set_source_abilities(_p, "equip:weapon", { "fixture_granted" })
        return a .. "|" .. r .. "|" .. tostring(A.knows(_p, "fixture_granted"))
        "#,
    );
    assert_eq!(out, "1|0|true");

    // Idempotent: saying the same thing again changes nothing. This is what
    // makes it safe to call on every login and every slot change.
    let out = vm.run(
        r#"
        local a, r = A.set_source_abilities(_p, "equip:weapon", { "fixture_granted" })
        return a .. "|" .. r .. "|" .. tostring(A.knows(_p, "fixture_granted"))
        "#,
    );
    assert_eq!(out, "0|0|true");

    // A second source keeps it known when the first lets go.
    let out = vm.run(
        r#"
        A.set_source_abilities(_p, "equip:offhand", { "fixture_granted" })
        A.set_source_abilities(_p, "equip:weapon", {})
        return tostring(A.knows(_p, "fixture_granted"))
        "#,
    );
    assert_eq!(out, "true", "one source releasing must not revoke another's grant");

    let out = vm.run(
        r#"
        A.set_source_abilities(_p, "equip:offhand", {})
        return tostring(A.knows(_p, "fixture_granted"))
        "#,
    );
    assert_eq!(out, "false");
}

/// Rank folds by `math.max`, in both directions.
///
/// A sword granting Cleave at rank 2 must not *reduce* a swordmaster already at
/// 5, and picking it up must not drop its floor for somebody at 0. It is the
/// only rule that is right both ways.
#[test]
fn rank_is_the_highest_of_the_trait_and_every_grant() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        DAEMON.trait.set_base(_p, "fixture_skill", 5)
        A.set_source_abilities(_p, "equip:weapon", { { id = "fixture_taught", rank = 2 } })
        local low = A.rank(_p, "fixture_taught")
        A.set_source_abilities(_p, "equip:weapon", { { id = "fixture_taught", rank = 7 } })
        local high = A.rank(_p, "fixture_taught")
        return low .. "|" .. high
        "#,
    );
    assert_eq!(out, "5|7", "the trait wins when it is higher, the grant when it is");
}

/// `known` reports what you have *and* whether the gates pass, which are
/// different questions. A listing needs both; `spell_d` filters on the second.
#[test]
fn known_separates_having_it_from_being_able_to_use_it() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        DAEMON.trait.set_base(_p, "fixture_skill", 1)
        local found
        for _, e in ipairs(A.known(_p, "technique")) do
            if e.id == "fixture_taught" then found = e end
        end
        return tostring(found ~= nil) .. "|" .. tostring(found.usable) .. "|" .. tostring(found.why)
        "#,
    );
    assert_eq!(
        out, "true|false|That is beyond you for now.",
        "you have it, you cannot use it, and it says which"
    );
}

// ─── Casts that span time ────────────────────────────────────────────────────

/// One working at a time. Not a limitation — something that should run alongside
/// another ability is an effect, not a cast.
#[test]
fn only_one_working_at_a_time() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        A.use(_p, "fixture_chant")
        local ok, why = A.use(_p, "fixture_chant")
        return tostring(ok) .. "|" .. tostring(why) .. "|" .. tostring(A.casting(_p) ~= nil)
        "#,
    );
    assert_eq!(out, "false|You are already busy.|true");
}

/// **The cost/cooldown split.** Cost is spent at the start; the ability's own
/// cooldown is marked at completion; nothing is refunded on an interrupt.
#[test]
fn an_interrupt_costs_you_and_does_not_mark_the_cooldown() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        local before = _p:trait("mp")
        A.use(_p, "fixture_chant")
        local during = _p:trait("mp")
        A.on_damaged(_p, 10, {})
        local after = _p:trait("mp")
        return (before - during) .. "|" .. (before - after) .. "|"
            .. tostring(A.casting(_p) ~= nil) .. "|"
            .. tostring(DAEMON.cooldown.remaining(_p, "technique.fixture_chant"))
            .. "|" .. tostring(DAEMON.effect.has(_p, "fixture_mark"))
        "#,
    );
    assert_eq!(
        out, "7|7|false|0|false",
        "spent up front, still spent after, the cast is gone, no cooldown was \
         marked and the outcome never happened"
    );
}

/// A blow under the threshold does not break concentration.
#[test]
fn a_glancing_blow_does_not_break_a_cast() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        A.define({ id = "probe_stoic", category = "technique", open = true,
                   target = "none", cast_time = 5,
                   interrupt = { on_damage = true, threshold = 5 } })
        A.use(_p, "probe_stoic")
        A.on_damaged(_p, 3, {})
        local survived = A.casting(_p) ~= nil
        A.on_damaged(_p, 9, {})
        return tostring(survived) .. "|" .. tostring(A.casting(_p) ~= nil)
        "#,
    );
    assert_eq!(out, "true|false");
}

/// Moving ends a cast, and **never refuses the move**.
#[test]
fn walking_out_ends_a_working_without_stopping_you() {
    let mut vm = RealVm::boot_with_fixture_world(0);
    vm.command("pagesize 0");

    vm.lua(
        "local p = get_player(SESSION) DAEMON.ability.use(p, 'fixture_chant') return 'cast'",
    );
    assert_eq!(
        vm.lua("local p = get_player(SESSION) return tostring(DAEMON.ability.casting(p) ~= nil)"),
        "true"
    );

    let out = vm.command("north");
    assert!(out.contains("Store Room"), "the move must still happen:\n{out}");
    assert_eq!(
        vm.lua("local p = get_player(SESSION) return tostring(DAEMON.ability.casting(p) ~= nil)"),
        "false",
        "and the working is over"
    );
}

/// A cast that runs its time resolves its outcomes.
#[test]
fn a_cast_that_is_left_alone_completes() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        A.use(_p, "fixture_chant")
        local record = A.casting(_p)
        A._complete(_p, record)
        return tostring(A.casting(_p) ~= nil) .. "|"
            .. tostring(DAEMON.effect.has(_p, "fixture_mark")) .. "|"
            .. tostring(DAEMON.cooldown.remaining(_p, "technique.fixture_chant"))
        "#,
    );
    assert_eq!(
        out, "false|true|0",
        "the effect landed; this ability declares no cooldown so there is none to mark"
    );
}

/// A channel **is** an effect: it is listed, it ticks, and it ends knowing why.
#[test]
fn a_channel_is_an_effect_instance() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        A.use(_p, "fixture_channel")
        local found
        for _, a in ipairs(DAEMON.effect.active(_p)) do
            if a.def.id == "channel_fixture_channel" then found = a end
        end
        return tostring(found ~= nil) .. "|" .. tostring(A.casting(_p) ~= nil)
        "#,
    );
    assert_eq!(out, "true|true", "the channel is visible in `effects`, like anything else");

    // Interrupting it takes the effect with it, because the record and the
    // instance are the same thing said two ways.
    let out = vm.run(
        r#"
        A.cancel(_p, "probe")
        return tostring(A.casting(_p) ~= nil) .. "|"
            .. tostring(DAEMON.effect.has(_p, "channel_fixture_channel"))
        "#,
    );
    assert_eq!(out, "false|false");
}

// ─── Creatures ───────────────────────────────────────────────────────────────

/// A creature's cooldowns are memory-only **by construction**.
///
/// A mob instance id is `mob:<seq>` from a sequence that restarts with the
/// process, so a durable one would come back after a reboot attached to a
/// different creature. Not a threshold decision — there is no duration at which
/// it would be right.
#[test]
fn a_creatures_cooldowns_are_never_written() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        local m = DAEMON.mobs.spawn("fixture_mouse", "fixture.hall")
        DAEMON.cooldown.mark(m, "technique.fixture_slow", 900)
        local obj = DAEMON.cache.get("cooldowns_obj", m.id, "technique.fixture_slow")
        local durable = DAEMON.cache.get("cooldowns", m.id, "technique.fixture_slow")
        local left = DAEMON.cooldown.remaining(m, "technique.fixture_slow")
        DAEMON.mobs.despawn(m)
        local after = DAEMON.cache.get("cooldowns_obj", m.id, "technique.fixture_slow")
        return tostring(obj ~= nil) .. "|" .. tostring(durable) .. "|"
            .. tostring(left > 0) .. "|" .. tostring(after)
        "#,
    );
    assert_eq!(
        out, "true|nil|true|nil",
        "900 seconds would normally be durable; a creature's never is, and \
         despawning drops the scope rather than leaking it forever"
    );
}

/// A character id still works everywhere it used to. The widening is additive.
#[test]
fn a_bare_char_id_still_works() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        DAEMON.cooldown.mark(901, "probe.gate", 30)
        return tostring(DAEMON.cooldown.remaining(901, "probe.gate") > 0) .. "|"
            .. tostring(DAEMON.cooldown.remaining(_p, "probe.gate") > 0) .. "|"
            .. tostring(DAEMON.cooldown.ready(901, "probe.nothing"))
        "#,
    );
    assert_eq!(out, "true|true|true", "an id and the entity that carries it are the same scope");
}

// ─── The pure half ───────────────────────────────────────────────────────────

/// Amount resolution: a number, a range, a scaled range, several scalars, and a
/// function. This is the whole of what a designer can write for a number.
#[test]
fn an_amount_resolves_in_every_shape_it_may_be_written() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        local ctx = { user = _p, rank = 4, power = 9 }
        local fixed = function(n) return 1 end
        local function n(v) return string.format("%d", math.floor(AB.roll(v, ctx, fixed))) end
        return table.concat({
            n(6),
            n({ min = 4, max = 9 }),
            n({ min = 10, max = 10, scale = { trait = "rank", per = 2 } }),
            n({ min = 100, max = 100, scale = { trait = "rank", pct = 5 } }),
            n({ min = 10, max = 10, scale = { { trait = "rank", per = 2 },
                                              { trait = "power", per = 1 } } }),
            n(function(c) return c.power * 2 end),
        }, "|")
        "#,
    );
    assert_eq!(
        out, "6|4|18|120|27|18",
        "a bare number; the low end of a range; +2 per rank; +5% of base per \
         rank; two scalars adding; and an arbitrary function"
    );
}

/// **A bug we shipped.** `$target` printed a table address.
///
/// `ctx.target` is the target *entity*, and `target` was in a whitelist that ran
/// `tostring` over whatever it found — so `game/abilities/spells.lua`'s
/// `"You draw a line of fire at $target."` rendered `at table: 0x7f…` in a
/// player's face. Nothing asserted a successful cast's message body, which is
/// exactly how it survived being written, reviewed and tested.
#[test]
fn a_target_token_renders_a_name_and_never_an_address() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        local m = DAEMON.mobs.spawn("fixture_mouse", "fixture.hall")
        return AB.render("You strike $target.", { target = m })
        "#,
    );
    assert_eq!(out, "You strike mouse.");
    assert!(!out.contains("table: 0x"), "an entity must never print as an address: {out}");

    // The whole authored line from the shipped spell, end to end.
    let out = vm.run(
        r#"
        local m = DAEMON.mobs.spawn("fixture_mouse", "fixture.store")
        return AB.render("{red}You draw a line of fire at $target.{/}", { target = m })
        "#,
    );
    assert_eq!(out, "{red}You draw a line of fire at mouse.{/}");

    // And the general form: a table nothing can name survives as its token
    // rather than leaking an address. `$ability` is an ordinary key in the ctx
    // every outcome is rendered against.
    let out = vm.run(
        r#"return AB.render("$ability costs $dealt.", { ability = { cost = {} }, dealt = 3 })"#,
    );
    assert_eq!(out, "$ability costs 3.");
}

/// An unknown token in a message is left alone rather than erased.
///
/// "You strike $victim" is a typo somebody can see and fix. "You strike " is a
/// bug they will stare at.
#[test]
fn an_unknown_message_token_survives() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"return AB.render("$name hits $target for $dealt, $victim watches",
                            { name = "Wren", target = "the mouse", dealt = 4 })"#,
    );
    assert_eq!(out, "Wren hits the mouse for 4, $victim watches");
}

/// The cooldown key is `<category>.<id>` unless `shared` names one, and then two
/// abilities gate each other with no new mechanism.
#[test]
fn a_shared_cooldown_is_just_a_shared_key() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        return AB.cooldown_key({ category = "spell", id = "emberlance" }) .. "|"
            .. AB.cooldown_key({ category = "technique", id = "cleave",
                                 cooldown = { shared = "technique.heavy" } })
        "#,
    );
    assert_eq!(out, "spell.emberlance|technique.heavy");

    let out = vm.run(
        r#"
        A.define({ id = "probe_a", category = "technique", open = true, target = "none",
                   cooldown = { seconds = 20, shared = "probe.heavy" } })
        A.define({ id = "probe_b", category = "technique", open = true, target = "none",
                   cooldown = { seconds = 20, shared = "probe.heavy" } })
        A.use(_p, "probe_a")
        local ok, why = A.use(_p, "probe_b")
        return tostring(ok) .. "|" .. tostring(why):sub(1, 7)
        "#,
    );
    assert_eq!(out, "false|Not yet", "using one gates the other");
}

// ─── Attacks ─────────────────────────────────────────────────────────────────

/// `attack` routes through the resolution pipeline, so it can miss — and when it
/// lands it meets armour exactly as a sword's blow does.
#[test]
fn an_attack_goes_through_the_resolution_pipeline() {
    let mut vm = Vm::new();

    // Pinned to always land.
    let hit = vm.run(
        r#"
        DAEMON.combat._roll = function(n) return 1 end
        local m = DAEMON.mobs.spawn("fixture_mouse", "fixture.hall")
        local before = m:trait("hp")
        A.use(_p, "fixture_lance", { target = m })
        return tostring(before - m:trait("hp"))
        "#,
    );
    assert_eq!(hit, "9");

    // The ward's flat reduction applies, which is the proof it went *through*
    // `take_damage` rather than around it.
    let warded = vm.run(
        r#"
        local m = DAEMON.mobs.spawn("fixture_mouse", "fixture.cellar")
        DAEMON.effect.apply(m, "fixture_ward", { source = "probe" })
        local before = m:trait("hp")
        A.use(_p, "fixture_lance", { target = m })
        return tostring(before - m:trait("hp"))
        "#,
    );
    assert_eq!(warded, "6", "the ward's three came off an ability's damage too");
}

/// An attack can miss, and says so in the ability's own words.
#[test]
fn an_attack_can_miss() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        DAEMON.combat._roll = function(n) return 100 end
        local m = DAEMON.mobs.spawn("fixture_mouse", "fixture.hall")
        local before = m:trait("hp")
        _p._sent = {}
        A.use(_p, "fixture_lance", { target = m })
        local said = table.concat(_p._sent, " ")
        return tostring(before == m:trait("hp")) .. "|" .. tostring(said:find("avoids", 1, true) ~= nil)
        "#,
    );
    assert_eq!(out, "true|true", "no damage, and the authored miss line");
}

/// `attack` and `damage` together is refused at define time.
///
/// The mirror of the gauge rule for `cost`: an ability ambiguous about whether
/// it can miss is worse discovered in a fight than at registration.
#[test]
fn an_ability_may_not_declare_both_attack_and_damage() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"local ok = A.define({ id = "probe_both", category = "technique", open = true,
                                 target = "creature",
                                 damage = { min = 1, max = 1 },
                                 attack = { damage = { min = 1, max = 1 } } })
           return tostring(ok) .. "|" .. tostring(A.get("probe_both"))"#,
    );
    assert_eq!(out, "false|nil");
}

/// The plain `damage` path is untouched and still always lands.
#[test]
fn plain_damage_is_unchanged_and_never_misses() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        DAEMON.combat._roll = function(n) return 100 end
        local m = DAEMON.mobs.spawn("fixture_mouse", "fixture.hall")
        local before = m:trait("hp")
        A.use(_p, "fixture_strike", { target = m })
        return tostring(before - m:trait("hp"))
        "#,
    );
    assert_eq!(out, "7", "a rolled 100 would miss an attack; plain damage does not roll");
}
