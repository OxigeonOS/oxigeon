//! Trait categories and skills, against the set `game/traits/` declares.
//!
//! The mechanics of sparsity are mudlib business and stay in
//! `tests/trait_sparsity.rs`. These assert the *shipped* trait registry —
//! `swordsmanship`, `herbalism`, how big the registry is — which is content.

use crate::common::RealVm;
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

