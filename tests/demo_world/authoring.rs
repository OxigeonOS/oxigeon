//! What OLC says about the areas *this game* ships.
//!
//! Here rather than in `tests/verify_lint.rs` because every assertion names
//! Thornhollow or the marsh: somebody who deletes `game/` to build their own
//! world should not inherit a failing suite over content they removed.
//!
//! Two things are worth pinning against real content rather than a fixture. The
//! linter has to be quiet about an area that works, and hand-authored areas have
//! to be *refused* by OLC — a regeneration would delete the very things that
//! make them worth shipping.

use crate::common::RealVm;

fn one_line(src: &str) -> String {
    src.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn probe() -> RealVm {
    RealVm::boot_real_mudlib_with_probe()
}

/// None of the shipped areas is OLC-managed, and that is deliberate.
///
/// Every one of them holds something OLC cannot author — thornhollow's square
/// has two inline room actions, greywater_marsh's descriptions are lfuns keyed
/// on the weather — so regenerating any of them would delete it silently. The
/// gate is what stops that, and this asserts the gate is closed.
#[test]
fn the_shipped_areas_are_not_olc_managed() {
    let mut vm = probe();

    for area in ["thornhollow", "wizard_workshop", "greywater_marsh", "collapsed_mine"] {
        let out = vm
            .eval(&format!(
                "local ok, why = require('daemons.codegen_d').is_managed('{area}') \
                 return tostring(ok) .. '|' .. tostring(why)"
            ))
            .unwrap();
        assert!(
            out.starts_with("false|"),
            "'{area}' is OLC-managed — a regeneration would eat its hand-written \
             behaviour: {out}"
        );
    }
}

/// `olc adopt thornhollow` reports the room actions it would move.
///
/// The demo that makes `custom.lua` make sense, and the regression that would
/// catch an adoption which quietly dropped them instead.
#[test]
fn adopting_thornhollow_reports_its_room_actions() {
    let mut vm = probe();

    let out = vm
        .eval(&one_line(
            "return table.concat(DAEMON.adopt.run(nil, 'thornhollow', false), '\\n')",
        ))
        .unwrap();

    assert!(out.contains("not OLC-managed"), "{out}");
    assert!(out.contains("Moves to custom.lua"), "{out}");
    assert!(
        out.contains("actions"),
        "square.lua's room actions should be reported as moving:\n{out}"
    );
    assert!(out.contains("Nothing has been written"), "a dry run must not write:\n{out}");
}

/// `olc adopt greywater_marsh` reports its lfun descriptions.
///
/// A different shape of the same problem: not a named hand-written field but an
/// ordinary one whose *value* is a function. Both have to be caught, and by
/// different halves of `schema.lossy`.
#[test]
fn adopting_the_marsh_reports_its_lfun_descriptions() {
    let mut vm = probe();

    let out = vm
        .eval(&one_line(
            "return table.concat(DAEMON.adopt.run(nil, 'greywater_marsh', false), '\\n')",
        ))
        .unwrap();

    assert!(out.contains("Moves to custom.lua"), "{out}");
    assert!(
        out.contains("description") && out.contains("function"),
        "the weather-keyed descriptions should be reported:\n{out}"
    );
}

/// The shipped content passes its own linter.
///
/// Not "zero findings" — these areas are hand-authored, so the lossy section is
/// expected and is the point of the previous two tests. What must be zero is
/// **errors**: an exit into nothing, a duplicate id, a `custom.lua` patch naming
/// a room that does not exist. Those are content bugs, and a content change that
/// introduces one should fail here rather than at somebody's login.
#[test]
fn no_shipped_area_has_a_lint_error() {
    let mut vm = probe();

    let broken = vm
        .eval(&one_line(
            "local out = {}
             for _, area in ipairs(require('lib.areaload').discover()) do
                 local r = DAEMON.verify.area(area)
                 for _, f in ipairs(r.findings) do
                     if f.level == 'error' then
                         out[#out+1] = area .. ' ' .. f.where .. ' ' .. tostring(f.what)
                                       .. ': ' .. f.message
                     end
                 end
             end
             table.sort(out) return table.concat(out, ' // ')",
        ))
        .unwrap();

    assert_eq!(broken, "", "the shipped content has lint errors: {broken}");
}

/// Every exit in the game leads somewhere that exists.
///
/// The check most worth having against real content: it is the easiest thing to
/// get wrong while editing and the symptom is a player walking into nothing.
#[test]
fn every_shipped_exit_leads_somewhere() {
    let mut vm = probe();

    let dangling = vm
        .eval(&one_line(
            "local out = {}
             for id, exits in pairs(DAEMON.world.exit_graph()) do
                 for direction, target in pairs(exits) do
                     if not DAEMON.world.get_room(target) then
                         out[#out+1] = id .. ' ' .. direction .. ' -> ' .. tostring(target)
                     end
                 end
             end
             table.sort(out) return table.concat(out, ', ')",
        ))
        .unwrap();

    assert_eq!(dangling, "", "exits leading nowhere: {dangling}");
}
