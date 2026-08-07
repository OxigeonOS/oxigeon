//! What this game actually ships on disk, as opposed to what the loader can do.
//!
//! These two moved out of `tests/mudlib/codegen.rs` when the suite was split.
//! Discovery and the `items.lua`-appends-`gear.lua` convention are mudlib
//! mechanisms and are tested there against a fixture; what is asserted *here* is
//! that Thornhollow, the marsh, the mine and the workshop are all still on disk
//! and still load. Delete `game/` and this file goes with it.

use crate::common::RealVm;

/// Every area the game ships is discovered, and none of them is lost.
///
/// The list `game/init.lua` used to hold, asserted against what discovery finds
/// — so deleting an area from disk fails here rather than at somebody's login.
#[test]
fn the_shipped_areas_are_all_discovered() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    let found = vm
        .eval("local a = require('lib.areaload') return table.concat(a.discover(), ',')")
        .unwrap();
    for area in ["collapsed_mine", "greywater_marsh", "thornhollow", "wizard_workshop"] {
        assert!(found.contains(area), "'{area}' was not discovered: {found}");
    }

    // …and each one actually loaded, which discovery alone does not prove.
    for room in [
        "thornhollow.square",
        "wizard_workshop.entrance",
        "collapsed_mine.adit",
        "greywater_marsh.causeway_head",
    ] {
        assert_eq!(
            vm.eval(&format!(
                "return tostring(DAEMON.world.get_room('{room}') ~= nil)"
            ))
            .unwrap(),
            "true",
            "'{room}' is missing — its area did not load"
        );
    }
}

/// The workshop's equipment is in its `items.lua`, where OLC can reach it.
///
/// It used to be in a `gear.lua` beside it, appended by `items.lua` — the
/// loader has five entry names and anything else in the directory has to be
/// pulled in by one of them, which is one convention instead of one exception.
/// That worked, and it also meant ten items OLC could not list, lint or edit,
/// because `olc save` regenerates `items.lua` and would not have written them
/// back. Making the area OLC-managed folded them in and deleted the file.
#[test]
fn the_workshops_equipment_is_in_its_generated_items_file() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    for id in ["apprentice_dagger", "warded_cloak", "oak_buckler", "leather_backpack"] {
        assert_eq!(
            vm.eval(&format!("return tostring(DAEMON.items.get('{id}') ~= nil)")).unwrap(),
            "true",
            "'{id}' was lost when gear.lua was folded into items.lua"
        );
    }

    // …and it is in the file OLC owns, not merely in the registry.
    let ids = vm
        .eval(
            "local out = {} \
             for _, d in ipairs(require('daemons.codegen_d').read('wizard_workshop', 'items')) do \
                 out[#out + 1] = d.id \
             end \
             return table.concat(out, ',')",
        )
        .unwrap();
    assert!(
        ids.contains("apprentice_dagger"),
        "the gear is registered but not in the generated file, so the next \
         `olc save` would drop it: {ids}"
    );
}

/// **Every generated file is already in the shape OLC would write.**
///
/// The conversion to OLC-managed was done by hand, in the shape `codegen_d`
/// emits, rather than by running `olc adopt` — adopt classifies a whole `map`
/// field as lossy the moment one key holds a function, so it would have dropped
/// the mine's plain exits along with its two checked ones, and the laboratory's
/// workbench along with its cauldron.
///
/// Hand-writing buys that back and costs this: the files have to be canonical or
/// the first `olc save` a builder does produces a large, confusing diff that has
/// nothing to do with what they changed. Field order, key order within a map,
/// indentation and line endings all have to match.
///
/// Re-generating each file from its own contents and comparing is the check.
/// It caught three things: an `exits` map written in the order it reads rather
/// than `movement.ORDER`; a file left with CRLF endings by a Windows editor
/// against a repo that mandates LF; and one that only shows on one runtime.
///
/// ─── The cross-runtime one ──────────────────────────────────────────────────
///
/// `speed = 1.0` in the mine's items was canonical under Lua 5.5 and not under
/// LuaJIT. Nothing is wrong with either: 5.3+ has an integer subtype, so `1.0`
/// is a float and `serialize.number` keeps the point to stop it changing type on
/// the way back; LuaJIT has no such subtype, so the same value is written `1`.
///
/// The value is identical either way — but the *file* is not, which means an
/// `olc save` produces a different diff depending on which Lua the server was
/// built against. So an **integral float in authored content is a hazard**, and
/// the fix is to author it as an integer where nothing needs the distinction.
/// This test runs on both runtimes, which is what makes it able to say so.
#[test]
fn every_generated_area_file_is_already_canonical() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    let out = vm
        .eval(
            "local CG = require('daemons.codegen_d') local bad = {} \
             local NL = string.char(10) \
             local function body(src) return (src:gsub('^.-' .. NL .. 'return ', 'return ')) end \
             for _, area in ipairs(require('lib.areaload').discover()) do \
                 for _, spec in ipairs(CG.GENERATED) do \
                     local list = CG.read(area, spec.file) \
                     if type(list) == 'table' then \
                         local disk = read_file(CG.path(area, spec.file)) \
                         local regen = CG.generate(area, spec.kind, list) \
                         if not regen or body(regen) ~= body(disk) then \
                             bad[#bad + 1] = area .. '/' .. spec.file \
                         end \
                     end \
                 end \
             end \
             return #bad == 0 and 'canonical' or table.concat(bad, ',')",
        )
        .unwrap();

    assert_eq!(
        out, "canonical",
        "these files are not what `olc save` would write, so the next save will \
         produce a diff nobody asked for"
    );
}
