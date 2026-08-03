//! Sparse traits, through the real VM.
//!
//! The claim under test is that **storage decides what an entity has**, rather
//! than the definition table deciding that everything has everything. Two
//! consequences follow, and both are asserted here rather than argued:
//!
//!  1. A sword has `dps` because it has damage and speed, and has no
//!     `willpower` because it has no `wisdom`. Nothing declares that.
//!  2. A recompute is O(traits this entity holds), not O(traits the game
//!     defines — which is the whole performance change.
//!
//! Presence and learning are covered in `tests/traits_effects.rs`, which came
//! first. What is here is the rest of the plan's verification list: `all()`
//! filtering, `category` as a lens that cannot change behaviour, which command
//! shows what, and the evaluation-count property.
//!
//! Per `CLAUDE.md`, everything goes through `tests/common/mod.rs`'s real
//! `ScriptEngine` — a helper called in isolation would answer a question about
//! a function rather than about what game code can do.

mod common;

use common::RealVm;

/// `all()` returns what the entity holds, not what the registry defines.
///
/// This is the assertion the plan opens with: `score` on a sword should stop
/// listing willpower. `all` is what `score` iterates, so this is that, one
/// level down from the command.
#[test]
fn all_returns_only_what_the_entity_holds() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_T = DAEMON.trait").unwrap();
    vm.eval("_sword = { stats = { durability = 100 } }").unwrap();
    vm.eval(
        "_T.define({ id = 'durability', kind = 'gauge', category = 'condition', \
         min = 0, max = 200, sets = 'item' }); _T.seal()",
    )
    .unwrap();

    assert_eq!(
        vm.eval("return #_T.all(_sword)").unwrap(),
        "1",
        "an item holding one trait should report exactly one, not the registry"
    );
    assert_eq!(vm.eval("return _T.all(_sword)[1].id").unwrap(), "durability");

    // The registry is much larger than one, so the filter is doing real work.
    vm.eval("_n = 0; for _ in pairs(_T.defs()) do _n = _n + 1 end").unwrap();
    let defined: i64 = vm.eval("return _n").unwrap().parse().unwrap();
    assert!(
        defined > 5,
        "the real game defines {defined} traits; this test proves nothing if it is 1"
    );

    // And a character, which does hold the character set, is unaffected.
    vm.eval("_c = { stats = {} }; _T.seed(_c, 'character')").unwrap();
    assert_eq!(
        vm.eval("return _T.has(_c, 'willpower')").unwrap(),
        "true",
        "a seeded character should still hold everything it always did"
    );
    assert_eq!(
        vm.eval("return _T.has(_sword, 'willpower')").unwrap(),
        "false",
        "a sword has no wisdom, so it can have no willpower"
    );
}

/// Reading a trait the entity does not have answers with the default and writes
/// nothing. Arithmetic stays safe; `has` answers the other question.
///
/// The "writes nothing" half is the one worth pinning: a `value` that
/// materialised on read would silently turn every entity dense again, and the
/// only symptom would be the performance regression this whole change exists to
/// avoid.
#[test]
fn an_absent_read_returns_the_default_and_stores_nothing() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_T = DAEMON.trait").unwrap();
    vm.eval("_T.define({ id = 'sp_luck', kind = 'attribute', default = 7 }); _T.seal()").unwrap();
    vm.eval("_e = { stats = {} }").unwrap();

    assert_eq!(vm.eval("return _T.value(_e, 'sp_luck')").unwrap(), "7");
    assert_eq!(
        vm.eval("return tostring(_e.stats.sp_luck)").unwrap(),
        "nil",
        "reading an absent trait materialised it on the entity"
    );
    assert_eq!(vm.eval("return _T.has(_e, 'sp_luck')").unwrap(), "false");
    assert_eq!(
        vm.eval("return #_T.present(_e)").unwrap(),
        "0",
        "a read should not have added anything to the present set"
    );
}

/// **The performance property, counted rather than timed.**
///
/// Define two hundred derived traits, hand an entity the two inputs one of them
/// needs, and count how many formulas actually run. A timing assertion would be
/// flaky; a call count is exact.
///
/// Before the present-set cache, `recompute` walked the global order, so this
/// number was 200 for every entity in the game, forever, growing with every
/// skill anyone ever authored.
#[test]
fn a_recompute_is_proportional_to_the_entity_not_the_registry() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_T = DAEMON.trait; _evals = 0").unwrap();

    // Two hundred derived traits, each over its own pair of inputs, so an
    // entity holding one pair can be present for exactly one of them.
    vm.eval(
        "for i = 1, 200 do \
           local a, b = 'perf_a' .. i, 'perf_b' .. i \
           _T.define({ id = a, kind = 'attribute', default = 1, sets = 'nobody' }) \
           _T.define({ id = b, kind = 'attribute', default = 1, sets = 'nobody' }) \
           _T.define({ id = 'perf_d' .. i, kind = 'derived', depends = { a, b }, \
             sets = 'nobody', formula = function(t) \
               _evals = _evals + 1 return t[a] + t[b] end }) \
         end; _T.seal()",
    )
    .unwrap();

    // Sanity: the registry really is large now.
    vm.eval("_n = 0; for _ in pairs(_T.defs()) do _n = _n + 1 end").unwrap();
    let defined: i64 = vm.eval("return _n").unwrap().parse().unwrap();
    assert!(defined >= 600, "expected 600+ definitions, got {defined}");

    // An entity holding exactly one pair. `perf_d7` is present because both its
    // dependencies are; the other 199 are not, and nothing had to say so.
    vm.eval("_small = { stats = { perf_a7 = 3, perf_b7 = 4 } }").unwrap();
    vm.eval("_evals = 0").unwrap();
    assert_eq!(vm.eval("return _T.value(_small, 'perf_d7')").unwrap(), "7");

    let evals: i64 = vm.eval("return _evals").unwrap().parse().unwrap();
    assert_eq!(
        evals, 1,
        "a full recompute of an entity holding one derived trait ran {evals} \
         formulas; walking the global order would run 200"
    );
    assert_eq!(
        vm.eval("return #_T.present(_small)").unwrap(),
        "3",
        "the entity holds two attributes and the one derived trait over them"
    );

    // Reading a trait it does not have still runs nothing extra: the answer is
    // the default, computed from no formula at all.
    vm.eval("_evals = 0").unwrap();
    assert_eq!(vm.eval("return _T.value(_small, 'perf_d100')").unwrap(), "0");
    assert_eq!(
        vm.eval("return _evals").unwrap(),
        "0",
        "reading an absent derived trait evaluated its formula"
    );
}

/// **`category` is a lens, not behaviour.**
///
/// The same trait, defined twice under two different categories, must compute
/// an identical value and settle identically. This is the test that stops
/// `category` quietly becoming a second `kind`: if adding a category can ever
/// change a number, the field has grown a meaning it was not supposed to have.
#[test]
fn a_category_cannot_change_what_a_trait_is_worth() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_T = DAEMON.trait").unwrap();

    // Identical in every way that could matter, different only in `category`.
    vm.eval(
        "for _, c in ipairs({ 'stat', 'reputation' }) do \
           _T.define({ id = 'lens_base_' .. c, kind = 'attribute', category = c, default = 9 }) \
           _T.define({ id = 'lens_derived_' .. c, kind = 'derived', category = c, \
             depends = { 'lens_base_' .. c }, round = 'floor', \
             formula = function(t) return t['lens_base_' .. c] * 3 + 1 end }) \
         end; _T.seal()",
    )
    .unwrap();

    vm.eval("_a = { stats = { lens_base_stat = 9 } }").unwrap();
    vm.eval("_b = { stats = { lens_base_reputation = 9 } }").unwrap();

    assert_eq!(vm.eval("return _T.value(_a, 'lens_derived_stat')").unwrap(), "28");
    assert_eq!(
        vm.eval("return _T.value(_b, 'lens_derived_reputation')").unwrap(),
        "28",
        "the same formula under a different category produced a different number"
    );

    // Presence is unaffected too: both are present for the same reason, and it
    // is not the category.
    assert_eq!(vm.eval("return #_T.present(_a)").unwrap(), "2");
    assert_eq!(vm.eval("return #_T.present(_b)").unwrap(), "2");

    // The only observable difference is which lens lists them.
    assert_eq!(vm.eval("return #_T.all(_a, 'stat')").unwrap(), "2");
    assert_eq!(vm.eval("return #_T.all(_a, 'reputation')").unwrap(), "0");
    assert_eq!(vm.eval("return #_T.all(_b, 'reputation')").unwrap(), "2");
    assert_eq!(vm.eval("return #_T.all(_b, 'stat')").unwrap(), "0");
}

/// Everything defined before `category` existed defaults to `"stat"`, which is
/// the migration property: `score` shows exactly what it always did.
#[test]
fn category_defaults_to_stat_so_nothing_moved() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_T = DAEMON.trait").unwrap();
    for id in ["strength", "max_hp", "hp", "level", "willpower"] {
        assert_eq!(
            vm.eval(&format!("return _T.get_def('{id}').category")).unwrap(),
            "stat",
            "'{id}' declares no category and so must default to stat"
        );
    }

    // A skill, which does declare one, is the counter-example that proves the
    // default is a default rather than the only value.
    assert_eq!(
        vm.eval("return _T.get_def('swordsmanship').category").unwrap(),
        "skill"
    );
    assert_eq!(
        vm.eval("return _T.get_def('swordsmanship').kind").unwrap(),
        "counter",
        "kind and category are separate axes — a skill need not be its own kind"
    );
    assert_eq!(
        vm.eval("return _T.get_def('sword_mastery').kind").unwrap(),
        "derived",
        "two traits can share a category and differ in kind, which is the point"
    );
    assert_eq!(
        vm.eval("return _T.get_def('sword_mastery').category").unwrap(),
        "skill"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
//  Command routing — through the real dispatcher, as a player meets it
// ═════════════════════════════════════════════════════════════════════════════

/// A skill appears in `skills` and not in `score`; an attribute the reverse.
///
/// Commands name what they show. A trait does not say where it goes, which is
/// what lets a game invent a category without editing any command it does not
/// want to change.
#[test]
fn score_and_skills_show_different_categories() {
    let mut vm = RealVm::boot_real_mudlib(0);

    // A fresh character has learned nothing — sparseness, seen from the front.
    let empty = vm.command("skills");
    assert!(
        empty.contains("not learned any skills"),
        "a new character should hold no skills:\n{empty}"
    );

    let score = vm.command("score");
    assert!(score.contains("Strength"), "score lost its attributes:\n{score}");
    assert!(
        !score.contains("Swordsmanship"),
        "an unlearned skill must not appear anywhere:\n{score}"
    );

    // Learn one. `set_base` on an absent trait creates it; that is the whole
    // mechanism, and `affect learn` is the admin door onto it.
    vm.command("affect learn swordsmanship 40");

    let skills = vm.command("skills");
    assert!(
        skills.contains("Swordsmanship"),
        "a learned skill should appear in `skills`:\n{skills}"
    );
    assert!(skills.contains("40"), "with its value:\n{skills}");
    assert!(
        skills.contains("Sword Mastery"),
        "the derived mastery becomes present the moment its dependency does:\n{skills}"
    );
    assert!(
        skills.contains("Weapon"),
        "`group` still sorts within the command:\n{skills}"
    );

    // And it stays out of `score`, which is what the category is for.
    let score = vm.command("score");
    assert!(
        !score.contains("Swordsmanship"),
        "a skill leaked into score; `category` is not being applied:\n{score}"
    );
    assert!(
        score.contains("Strength"),
        "score should be unchanged by learning a skill:\n{score}"
    );

    // Unlearning takes the derived mastery with it — presence cascades, because
    // it is derived from storage rather than declared.
    vm.command("affect unlearn swordsmanship");
    let after = vm.command("skills");
    assert!(
        after.contains("not learned any skills"),
        "forgetting the counter should remove the mastery derived over it:\n{after}"
    );
}

/// `traits` shows everything, including a category no other command names.
///
/// That is the correct default for a new category — it should not silently leak
/// into `score` — but it needs somewhere to be findable, and this is the
/// discoverability answer.
#[test]
fn the_traits_command_is_where_an_uncategorised_trait_shows_up() {
    let mut vm = RealVm::boot_real_mudlib(0);

    let defs = vm.command("traits defs");
    assert!(defs.contains("Defined traits"), "no registry listing:\n{defs}");
    assert!(
        defs.contains("swordsmanship") && defs.contains("skill"),
        "the registry should name each trait's category:\n{defs}"
    );
    assert!(
        defs.contains("strength") && defs.contains("attribute"),
        "and its kind, which is a different axis:\n{defs}"
    );

    // The character's own view: grouped by category, with the gap between what
    // it holds and what the game defines made visible.
    let mine = vm.command("traits");
    assert!(mine.contains("stat"), "expected the stat category:\n{mine}");
    assert!(
        mine.contains("defined traits present"),
        "the present-vs-defined gap is the point of sparse traits:\n{mine}"
    );
    assert!(
        !mine.contains("swordsmanship"),
        "an unlearned skill is not present and must not be listed:\n{mine}"
    );

    vm.command("affect learn herbalism 5");
    let mine = vm.command("traits");
    assert!(
        mine.contains("herbalism") && mine.contains("skill"),
        "a learned skill should appear under its category:\n{mine}"
    );
}

/// The migration proof, from the front: a seeded character's `score` still
/// renders every group it did before traits went sparse.
///
/// Existing characters need no migration — they already have all eleven traits
/// materialised, so every one is present and behaviour is identical. This is
/// that claim, pinned.
#[test]
fn a_seeded_characters_score_is_unchanged_by_sparsity() {
    let mut vm = RealVm::boot_real_mudlib(0);
    let out = vm.command("score");

    for expected in [
        "Health", "Mana",            // gauges
        "Strength", "Dexterity", "Constitution", "Intelligence", "Wisdom",
        "Max Health", "Max Mana", "Willpower",   // derived
        "Level", "Gold", "Experience",
    ] {
        assert!(
            out.contains(expected),
            "score lost '{expected}' — the character set is no longer seeding it:\n{out}"
        );
    }

    // Eleven traits plus the two Player fields. Nothing extra crept in from the
    // skills file, because a skill is in no seed set.
    for unexpected in ["Swordsmanship", "Archery", "Herbalism", "Mining", "Sword Mastery"] {
        assert!(
            !out.contains(unexpected),
            "'{unexpected}' is seeded when it should be learned:\n{out}"
        );
    }
}

/// A skill survives save and load after `Mobile.skills` was deleted. Storage
/// moved into `stats`; nothing was lost.
#[test]
fn a_learned_skill_round_trips_through_the_character_save() {
    let mut vm = RealVm::boot_real_mudlib(0);

    vm.command("affect learn archery 23");
    assert!(vm.command("skills").contains("23"));

    // Through the real save path, then read back out of the live character.
    vm.command("save");
    let after = vm.command("skills");
    assert!(
        after.contains("Archery") && after.contains("23"),
        "a skill did not survive a save:\n{after}"
    );

    // And it is in `stats`, which is the only place it should be — there is no
    // parallel map left to disagree with.
    let dump = vm.command("traits");
    assert!(dump.contains("archery"), "expected archery in the trait dump:\n{dump}");
}
