//! What OLC says about the areas *this game* ships.
//!
//! Here rather than in `tests/verify_lint.rs` because every assertion names
//! Thornhollow or the marsh: somebody who deletes `game/` to build their own
//! world should not inherit a failing suite over content they removed.
//!
//! Two things are worth pinning against real content rather than a fixture. The
//! linter has to be quiet about an area that works, and every shipped area has
//! to be editable in the game — which means OLC-managed, with everything OLC
//! cannot author moved into a `custom.lua` beside it.

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

/// **Every shipped area is OLC-managed**, so a builder can edit any of them
/// from inside the game.
///
/// This used to assert the exact opposite, and the reason it did was real: each
/// of these areas holds something OLC cannot author — the square's inline room
/// actions, the marsh's weather-keyed lfun descriptions, the mine's two exits
/// with a `check` — and regenerating a file wholesale would have deleted them
/// silently. The gate being *closed* was the only thing standing in the way.
///
/// Closing the gate was never the goal, though; not losing the behaviour was.
/// So each area was split instead: the data in the generated files, everything
/// that is a function in a hand-written `custom.lua` that OLC never touches. The
/// gate is open now because there is nothing left behind it to eat.
#[test]
fn every_shipped_area_is_olc_managed() {
    let mut vm = probe();

    for area in ["thornhollow", "wizard_workshop", "greywater_marsh", "collapsed_mine"] {
        let out = vm
            .eval(&format!(
                "local ok, why = require('daemons.codegen_d').is_managed('{area}')                  return tostring(ok) .. '|' .. tostring(why)"
            ))
            .unwrap();
        assert!(
            out.starts_with("true|"),
            "'{area}' is not OLC-managed, so it cannot be edited in the game: {out}"
        );
    }
}

/// …and each one kept a `custom.lua`, which is the other half of that bargain.
///
/// A managed area with no `custom.lua` is not automatically wrong — an area that
/// is genuinely all data needs none. But all four of these had behaviour that
/// could not be expressed as data, so a missing file here means it was dropped
/// on the way rather than moved.
#[test]
fn every_shipped_area_kept_its_hand_written_half() {
    let mut vm = probe();

    for area in ["thornhollow", "wizard_workshop", "greywater_marsh", "collapsed_mine"] {
        let out = vm
            .eval(&format!(
                "local c = require('daemons.codegen_d').read('{area}', 'custom')                  return type(c)"
            ))
            .unwrap();
        assert_eq!(out, "table", "'{area}' lost its custom.lua");
    }
}

/// No shipped area is still assembled by an `init.lua`.
///
/// `areaload.inspect` prefers `init.lua` over `rooms.lua` unconditionally, so a
/// generated `rooms.lua` beside a surviving `init.lua` would never be read —
/// every `olc save` writing to a file the loader ignores, reporting success.
/// Thornhollow was in exactly that shape and is the reason `adopt_d` now refuses
/// it out loud.
#[test]
fn no_shipped_area_is_assembled_by_an_init_lua() {
    let mut vm = probe();

    for area in ["thornhollow", "wizard_workshop", "greywater_marsh", "collapsed_mine"] {
        let entry = vm
            .eval(&format!(
                "return tostring(require('lib.areaload').inspect('{area}').entry)"
            ))
            .unwrap();
        assert_eq!(
            entry, "rooms",
            "'{area}' is still entered through {entry}.lua, which OLC cannot write"
        );
    }
}


// `adopting_the_marsh_reports_its_lfun_descriptions` lived here. It asserted
// that a dry run named the marsh's weather-keyed lfun descriptions as fields
// that would move to `custom.lua` — which was worth pinning while the marsh was
// hand-authored and the adoption was hypothetical. The marsh is OLC-managed now
// and its descriptions *are* in `custom.lua`, so adoption refuses it, and what
// the test was really about is asserted directly by
// `marsh::descriptions_read_the_weather_without_being_told`. What `adopt`
// classifies as lossy is a mudlib claim and is tested against fixtures in
// `tests/mudlib/olc_adopt.rs`.


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

/// The three reagent vials survived becoming data.
///
/// They were a `for` loop over a colour table, so they existed only when the
/// file ran: `olc list items` could not see them and `verify` could not check
/// them. As declared records naming a prototype they are the same three items
/// and are now visible to both.
#[test]
fn the_reagent_vials_are_data_now_and_are_unchanged() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    for colour in ["red", "blue", "green"] {
        let out = vm.eval(&format!(
            "local i = DAEMON.items.get('potion_{colour}') \
             return tostring(i.short) .. '|' .. tostring(i.weight) .. '|' \
                 .. tostring(i.value) .. '|' .. table.concat(i.tags or {{}}, '+')"
        )).unwrap();
        assert_eq!(
            out,
            format!("a vial of {colour} liquid|1|12|reagent"),
            "weight and tags are inherited, the description is the record's own"
        );
    }
}
