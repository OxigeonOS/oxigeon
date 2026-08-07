//! A broken trait definition is reported, and survivable.
//!
//! `seal()` reports a cycle **as a path** and a missing dependency **by name**,
//! because "there is a cycle somewhere in your thirty traits" is not something
//! anybody can act on. And one bad trait must not disable the other thirty —
//! the same guarantee a broken area file has.
//!
//! These used to be five declarations in `game/traits/broken_example.lua`, a
//! file shipped deliberately broken and loaded only from a test. Two problems
//! with that: broken code does not belong in a content directory, and
//! `make_test_lua()` puts `game/` on `package.path`, so it sat one `require`
//! away from every Lua unit test.
//!
//! The counter-argument in that file's header was that a test hand-writing the
//! traits "would be testing its own fixture rather than the game's". That holds
//! for an *area*, whose shape is complicated. Five trait declarations are not
//! complicated, and nothing below depends on this game — which is why this is a
//! mudlib test rather than a content one.

use crate::common::RealVm;

fn one_line(src: &str) -> String {
    src.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ")
}

/// The deliberately broken set. Four defects and one control:
///
/// * `broken_dangling` depends on a trait that does not exist. `seal` must mark
///   it failed, name what is missing, and leave the trait answering with its
///   `default`.
/// * `broken_cycle_a` → `b` → `c` → `a` is a three-trait cycle, which must be
///   reported as a path so every link is named and the one to break is obvious.
/// * `broken_bystander` is **not** broken: it depends on a real trait and must
///   keep computing while the four above are failed.
///
/// No `--` comments in here: the probe collapses this to one line, so a comment
/// would swallow everything after it.
const BROKEN: &str = r#"
DAEMON.trait.define_all({
    { id = "broken_dangling", label = "Dangling", kind = "derived",
      group = "broken", depends = { "no_such_trait" }, default = 3,
      formula = function(t) return t.no_such_trait * 2 end },

    { id = "broken_cycle_a", label = "Cycle A", kind = "derived",
      group = "broken", depends = { "broken_cycle_b" }, default = 1,
      formula = function(t) return t.broken_cycle_b + 1 end },
    { id = "broken_cycle_b", label = "Cycle B", kind = "derived",
      group = "broken", depends = { "broken_cycle_c" }, default = 1,
      formula = function(t) return t.broken_cycle_c + 1 end },
    { id = "broken_cycle_c", label = "Cycle C", kind = "derived",
      group = "broken", depends = { "broken_cycle_a" }, default = 1,
      formula = function(t) return t.broken_cycle_a + 1 end },

    { id = "broken_bystander", label = "Bystander", kind = "derived",
      group = "broken", depends = { "wisdom" }, round = "floor",
      formula = function(t) return t.wisdom * 2 end },
})
return 'defined'
"#;

#[test]
fn a_broken_trait_definition_is_reported_and_survivable() {
    let mut vm = RealVm::boot_fixture_with_probe();

    assert_eq!(vm.eval(&one_line(BROKEN)).unwrap(), "defined");
    assert_eq!(
        vm.eval("return tostring(DAEMON.trait.seal())").unwrap(),
        "false",
        "seal should report that something is broken"
    );

    // A missing dependency is named, not merely counted.
    let dangling = vm.eval("return DAEMON.trait.errors().broken_dangling").unwrap();
    assert!(
        dangling.contains("no_such_trait"),
        "a missing dependency should be named: {dangling}"
    );

    // The cycle is a *path*, naming every link.
    let cycle = vm.eval("return DAEMON.trait.errors().broken_cycle_a").unwrap();
    assert!(cycle.contains("->"), "a cycle should be reported as a path: {cycle}");
    for link in ["broken_cycle_a", "broken_cycle_b", "broken_cycle_c"] {
        assert!(cycle.contains(link), "the path should name '{link}': {cycle}");
    }

    // The graph still works. `max_hp` is `50 + constitution*5 + (level-1)*10`,
    // so a seeded character with constitution 10 at level 1 is 100.
    vm.eval("_c = { stats = {} } DAEMON.trait.seed(_c, 'character') return 'seeded'").unwrap();
    assert_eq!(
        vm.eval("return DAEMON.trait.value(_c, 'max_hp')").unwrap(),
        "100",
        "one bad trait disabled the other thirty"
    );

    // A failed trait answers with its default rather than raising.
    assert_eq!(vm.eval("return DAEMON.trait.value(_c, 'broken_dangling')").unwrap(), "3");

    // And the bystander keeps computing, which is what makes this "one bad
    // trait" rather than "one bad file".
    assert_eq!(
        vm.eval("return DAEMON.trait.value(_c, 'broken_bystander')").unwrap(),
        "20",
        "wisdom 10 * 2; a healthy trait beside a broken one must still compute"
    );
}
