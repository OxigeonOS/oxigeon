//! Body layouts, hit locations, and the resolution pipeline they feed.
//!
//! The property the whole feature rests on: **a creature with no layout behaves
//! exactly as it did before layouts existed.** `Body.of` returns nil, no roll is
//! consumed choosing a location, `ev.hit_slot` is nil, and the per-slot armour
//! guard is skipped. There is no `if layouts_enabled` anywhere — the absence is
//! the compatibility path.
//!
//! Fixture world only; the layouts are written into the test's own game root.

use crate::common::RealVm;

fn one_line(src: &str) -> String {
    src.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ")
}

struct Vm {
    vm: RealVm,
    game: std::path::PathBuf,
}

impl Vm {
    fn new() -> Self {
        let mut vm = RealVm::boot_fixture_with_probe();
        let game = vm.game_root().unwrap().to_path_buf();
        vm.eval("B = require('lib.body') C = require('lib.combat') return 'ready'").unwrap();
        Self { vm, game }
    }

    fn layouts(&mut self, lua: &str) {
        let dir = self.game.join("body");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("fixture.lua"), lua).unwrap();
        self.run("require('body').flush_cache() return 'flushed'");
    }

    fn run(&mut self, src: &str) -> String {
        self.vm.eval(&one_line(src)).unwrap()
    }
}

const HUMANOID: &str = r#"return {
    layouts = {
        humanoid = {
            features = { "hands", "feet" },
            parts = {
                { id = "head",  size = 10, height = 95, slot = "head",
                  vulnerable = { piercing = 0.5 }, vital = true },
                { id = "chest", size = 60, height = 70, slot = "chest" },
                { id = "legs",  size = 30, height = 20, slot = "legs" },
            },
        },
        insect = {
            features = { "bite" },
            parts = { { id = "carapace", size = 100, height = 40 } },
        },
        broken = {
            parts = { { id = "nowhere", size = 0, height = 50 },
                      { id = "toohigh", size = 5, height = 400 } },
        },
    },
}"#;

// ─── Discovery ───────────────────────────────────────────────────────────────

/// Layouts are found across the jail roots, and a flush picks up an edit.
#[test]
fn layouts_are_discovered_from_the_game_root() {
    let mut vm = Vm::new();
    assert_eq!(vm.run("return tostring(require('body').get('humanoid'))"), "nil");

    vm.layouts(HUMANOID);
    assert_eq!(vm.run("return table.concat(require('body').ids(), ',')"), "humanoid,insect");
    assert_eq!(vm.run("return #require('body').get('humanoid').parts"), "3");
}

/// A malformed part is reported and dropped; the other layouts still load.
#[test]
fn a_broken_layout_is_reported_and_does_not_take_the_others_with_it() {
    let mut vm = Vm::new();
    vm.layouts(HUMANOID);

    let problems = vm.run(
        "local out = {} for _, p in ipairs(require('body').problems()) do out[#out+1] = p end \
         table.sort(out) return table.concat(out, '\\n')",
    );
    assert!(problems.contains("no size"), "a sizeless part can never be hit: {problems}");
    assert!(problems.contains("height between 0 and 100"), "{problems}");
    assert!(problems.contains("no usable parts"), "{problems}");

    // …and the good ones are unaffected.
    assert_eq!(vm.run("return tostring(require('body').get('humanoid') ~= nil)"), "true");
    assert_eq!(vm.run("return tostring(require('body').get('broken'))"), "nil");
}

/// A layout attaches by `body`, then by `race`, then by config — and nil is the
/// whole backwards-compatible path.
#[test]
fn a_layout_attaches_by_body_then_race_then_nothing() {
    let mut vm = Vm::new();
    vm.layouts(HUMANOID);

    let out = vm.run(
        "local a = B.of({ body = 'humanoid' }) \
         local b = B.of({ race = 'insect' }) \
         local c = B.of({ body = 'humanoid', race = 'insect' }) \
         local d = B.of({ id = 'plain' }) \
         return tostring(a and a.id) .. '|' .. tostring(b and b.id) .. '|' \
             .. tostring(c and c.id) .. '|' .. tostring(d)",
    );
    assert_eq!(out, "humanoid|insect|humanoid|nil", "`body` overrides `race`, and nil is legal");
}

/// Features are read from the layout and from its parts alike.
#[test]
fn a_feature_may_be_declared_on_the_layout_or_on_a_part() {
    let mut vm = Vm::new();
    vm.layouts(HUMANOID);

    let out = vm.run(
        "local e = { body = 'humanoid' } \
         return tostring(B.has_feature(e, 'hands')) .. '|' \
             .. tostring(B.has_feature(e, 'bite')) .. '|' \
             .. tostring(B.has_feature({ id = 'x' }, 'hands'))",
    );
    assert_eq!(out, "true|false|false");
}

/// An unknown part field rides through untouched, so there is no closed list.
#[test]
fn an_unknown_part_field_survives_onto_the_result() {
    let mut vm = Vm::new();
    vm.layouts(HUMANOID);

    let out = vm.run(
        "local l = require('body').get('humanoid') \
         for _, p in ipairs(l.parts) do if p.id == 'head' then return tostring(p.vital) end end \
         return 'missing'",
    );
    assert_eq!(out, "true", "`vital` is nothing the mudlib knows and it is kept anyway");
}

// ─── Reach ───────────────────────────────────────────────────────────────────

/// The window narrows with height and widens with a longer weapon.
#[test]
fn reach_decides_how_high_a_blow_can_land() {
    let mut vm = Vm::new();

    // Equal heights, no weapon: everything is reachable.
    let out = vm.run("local lo, hi = B.window(180, 180, 0) return lo .. '|' .. math.floor(hi)");
    assert_eq!(out, "0|100");

    // A short attacker against a tall one cannot reach the head.
    let out = vm.run("local _, hi = B.window(100, 300, 0) return math.floor(hi)");
    assert_eq!(out, "38");

    // A long weapon buys reach.
    let out = vm.run("local _, hi = B.window(100, 300, 150) return math.floor(hi)");
    assert_eq!(out, "88");

    // Either height missing disables the filter entirely, which is the ordinary
    // case for a game that defines no `height` trait.
    assert_eq!(vm.run("local _, hi = B.window(0, 300, 0) return math.floor(hi)"), "100");
}

/// When nothing is in reach the **lowest** part is returned, never nothing.
#[test]
fn out_of_reach_hits_the_shins_rather_than_missing_entirely() {
    let mut vm = Vm::new();
    vm.layouts(HUMANOID);

    let out = vm.run(
        "local l = require('body').get('humanoid') \
         local c = B.candidates(l, 0, 5) \
         return #c .. '|' .. c[1].id",
    );
    assert_eq!(out, "1|legs", "a halfling with a dagger hits a giant's shins");
}

/// The pick is weighted by size and covers every part.
#[test]
fn a_location_is_weighted_by_size() {
    let mut vm = Vm::new();
    vm.layouts(HUMANOID);

    // Roll 1 lands in the first part, roll 100 in the last, and the boundary
    // between chest and legs is exactly at 70.
    let out = vm.run(
        "local l = require('body').get('humanoid') \
         local function at(n) return B.pick(l.parts, function() return n end).id end \
         return at(1) .. '|' .. at(10) .. '|' .. at(11) .. '|' .. at(70) .. '|' .. at(71) .. '|' .. at(100)",
    );
    assert_eq!(out, "head|head|chest|chest|legs|legs");
}

// ─── The pipeline ────────────────────────────────────────────────────────────

/// **The compatibility property.** With no layout, a swing consumes exactly the
/// rolls it always did and reports no location.
#[test]
fn no_layout_means_no_location_and_no_extra_roll() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        local rolls = 0
        local real = DAEMON.combat._roll
        DAEMON.combat._roll = function(n) rolls = rolls + 1 return 1 end
        local a = DAEMON.mobs.spawn("fixture_mouse", "fixture.hall")
        local d = DAEMON.mobs.spawn("fixture_mouse", "fixture.store")
        local r = DAEMON.combat.attack_once(a, d)
        DAEMON.combat._roll = real
        return rolls .. "|" .. tostring(r.hit) .. "|" .. tostring(r.location)
            .. "|" .. tostring(r.hit_slot)
        "#,
    );
    // Two rolls: the to-hit, and the weapon's damage spread. No third for a
    // location, because there is no layout to choose one from.
    assert_eq!(out, "2|true|nil|nil");
}

/// With a layout, the blow lands somewhere and says where.
#[test]
fn a_layout_gives_the_blow_a_place_to_land() {
    let mut vm = Vm::new();
    vm.layouts(HUMANOID);

    let out = vm.run(
        r#"
        local a = DAEMON.mobs.spawn("fixture_mouse", "fixture.hall")
        local d = DAEMON.mobs.spawn("fixture_mouse", "fixture.store")
        d.body = "humanoid"
        DAEMON.combat._roll = function(n) return 1 end
        local r = DAEMON.combat.attack_once(a, d)
        return tostring(r.hit_part) .. "|" .. tostring(r.hit_slot)
            .. "|" .. tostring(r.location.vital)
        "#,
    );
    assert_eq!(out, "head|head|true", "and the part's own fields come with it");
}

/// A part's vulnerability is proportional and per damage type.
#[test]
fn a_vulnerable_part_takes_more_of_the_right_kind() {
    let mut vm = Vm::new();

    let out = vm.run(
        "local band = { power = 1.0 } \
         local head = { vulnerable = { piercing = 0.5 } } \
         return string.format('%d|%d', C.damage(10, band, head, 'piercing'), \
                                       C.damage(10, band, head, 'blunt'))",
    );
    assert_eq!(out, "15|10");
}

/// Armour protects the slot it is worn in, and only that one.
#[test]
fn a_helm_does_nothing_for_a_blow_to_the_legs() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        local m = DAEMON.mobs.spawn("fixture_mouse", "fixture.hall")
        m.equipment = {}
        local helm = require('components').build({ id = "probe_helm", short = "a helm",
            components = { "armour" }, slot = "head", defense = 4 })
        require('lib.equipment').refresh_slot(m, "head", helm)
        local before = m:trait("hp")
        m:take_damage(10, { damage_type = "physical", hit_slot = "head" })
        local head = before - m:trait("hp")
        DAEMON.trait.set_cur(m, "hp", before)
        m:take_damage(10, { damage_type = "physical", hit_slot = "legs" })
        local legs = before - m:trait("hp")
        DAEMON.trait.set_cur(m, "hp", before)
        m:take_damage(10, { damage_type = "physical" })
        local nowhere = before - m:trait("hp")
        return head .. "|" .. legs .. "|" .. nowhere
        "#,
    );
    assert_eq!(
        out, "6|10|6",
        "the helm blunts a head blow, does nothing to the legs, and — with no \
         location at all — still applies, which is every call the game makes today"
    );
}

/// Proportional absorb lands in the `mult` phase, before the flat reduction.
#[test]
fn absorb_is_proportional_and_lands_before_the_flat_reduction() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        local m = DAEMON.mobs.spawn("fixture_mouse", "fixture.hall")
        m.equipment = {}
        local plate = require('components').build({ id = "probe_plate", short = "plate",
            components = { "armour" }, slot = "chest", defense = 2,
            absorb = { physical = 0.5 } })
        require('lib.equipment').refresh_slot(m, "chest", plate)
        local before = m:trait("hp")
        m:take_damage(10, { damage_type = "physical" })
        return tostring(before - m:trait("hp"))
        "#,
    );
    assert_eq!(out, "3", "half of ten is five, then two comes off — not (10-2)/2");
}

/// A defender holding no channel trait gets one implicit dodge worth the whole
/// pool, which is what makes the pipeline reduce to the formula it replaced.
#[test]
fn a_defender_with_no_channel_traits_still_dodges() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        local m = DAEMON.mobs.spawn("fixture_mouse", "fixture.store")
        return tostring(DAEMON.trait.has(m, "defense_dodge")) .. "|"
            .. tostring(DAEMON.combat.channels().dodge ~= nil)
            .. "|" .. tostring(DAEMON.combat.channels().parry ~= nil)
            .. "|" .. tostring(DAEMON.combat.channels().block ~= nil)
        "#,
    );
    assert_eq!(out, "false|true|true|true", "the channels are registered; the mouse holds none");

    // And it still gets hit at the rate it always did.
    let out = vm.run(
        r#"
        local a = DAEMON.mobs.spawn("fixture_mouse", "fixture.hall")
        local d = DAEMON.mobs.spawn("fixture_mouse", "fixture.store")
        DAEMON.combat._roll = function(n) return 1 end
        local r = DAEMON.combat.attack_once(a, d)
        return r.threshold .. "|" .. tostring(r.channel)
        "#,
    );
    assert_eq!(out, "60|dodge", "equal dexterity, so the shipped base chance exactly");
}
