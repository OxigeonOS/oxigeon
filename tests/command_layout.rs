//! Command discovery across subdirectories, and the one file that declares many.
//!
//! `cmds/` had 74 files in one flat list, twelve of which were the same eleven
//! lines with one string changed. Splitting it needed two things the loader did
//! not do: descend into a directory (it explicitly discarded `is_dir` entries),
//! and accept a module declaring more than one command.
//!
//! Both failure modes are **silent**. A skipped directory logs nothing; a
//! module the loader does not recognise is dropped without a word. So the whole
//! admin command set could vanish and the only symptom would be a player being
//! told "I don't understand that" — which is why these assertions exist.

mod common;

use common::RealVm;

/// Load the registry once and return the sorted list of canonical command names.
fn names(vm: &mut RealVm) -> Vec<String> {
    vm.eval(
        "local out = {} for name in pairs(require('lib.commands').registry()) do \
           out[#out+1] = name end table.sort(out) return table.concat(out, ',')",
    )
    .unwrap()
    .split(',')
    .map(str::to_string)
    .collect()
}

#[test]
fn commands_in_a_subdirectory_are_discovered() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    let found = names(&mut vm);

    // cmds/admin/ — the biggest group, and the one whose absence a player
    // would never notice.
    for cmd in ["spawn", "goto", "snoop", "trace", "objdump", "teleport", "reload"] {
        assert!(found.iter().any(|n| n == cmd), "missing admin command `{cmd}`");
    }
    // cmds/building/
    for cmd in ["olc", "dig"] {
        assert!(found.iter().any(|n| n == cmd), "missing building command `{cmd}`");
    }
    // …and the top level still works.
    for cmd in ["look", "get", "drop", "help", "say"] {
        assert!(found.iter().any(|n| n == cmd), "missing core command `{cmd}`");
    }
}

#[test]
fn a_subdirectory_command_keeps_its_permission() {
    // Moving a file must not quietly ungate it.
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    assert_eq!(
        vm.eval("return tostring(require('lib.commands').registry()['spawn'].permission)")
            .unwrap(),
        "cmd.spawn"
    );
    assert_eq!(
        vm.eval("return tostring(require('lib.commands').registry()['olc'].permission)")
            .unwrap(),
        "cmd.olc"
    );
}

/// Every gated command names `cmd.<verb>`, and the verb is its own name.
///
/// There was no scheme, and the cost was silent: `setup_roles.lua` granted
/// `cmd.olc` while this command required `olc`, `cmd.verify` while `verify`
/// required `efun.verify`, and `efun.write_file` while `permissions.toml` said
/// `efun.file.write`. Not one grant in the builder role matched anything, so the
/// role was decorative and the only account that could build was account 1, by
/// the `is_admin` bypass. Every part of that was individually invisible.
///
/// Requiring the string to *contain the verb* is the half that matters. A
/// uniform prefix alone would still let `dig` ask for `cmd.olc` — which it did,
/// so `dig` could not be granted separately from `olc`.
#[test]
fn every_gated_command_names_cmd_dot_its_own_verb() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    let bad = vm
        .eval(
            "local out = {} \
             for name, mod in pairs(require('lib.commands').registry()) do \
               local p = mod.permission \
               if p ~= nil then \
                 if type(p) ~= 'string' or not p:match('^cmd%.[%w_]+$') \
                    and not p:match('^cmd%.[%w_]+%.[%w_]+$') then \
                   out[#out+1] = name .. '=' .. tostring(p) \
                 elseif p:match('^cmd%.([%w_]+)') ~= name then \
                   out[#out+1] = name .. '=' .. p .. '(wrong verb)' \
                 end \
               end \
             end \
             table.sort(out) return table.concat(out, ' ')",
        )
        .unwrap();

    assert_eq!(
        bad, "",
        "commands whose permission is not `cmd.<their own verb>`: {bad}"
    );
}

#[test]
fn the_game_layer_is_still_found_alongside_the_mudlib() {
    // `list_dir` merges both roots and the recursion must not have lost that.
    //
    // Against the *fixture* world, which ships one command of its own, rather
    // than against `game/cmds/`: this is a claim about the loader, and it
    // should keep meaning something for somebody who deleted the demo game.
    let mut vm = RealVm::boot_with_fixture_world(0);

    let out = vm.command("help all");
    assert!(
        out.contains("fixturecmd"),
        "a command in the game root should be discovered:\n{out}"
    );
    // …in the same listing as a mudlib command from a subdirectory.
    assert!(out.contains("spawn"), "admin commands should be listed too:\n{out}");

    assert!(
        vm.command("fixturecmd").contains("fixture command ran"),
        "and it should actually dispatch"
    );
    assert!(vm.command("fx").contains("fixture command ran"), "aliases too");
}

#[test]
fn one_file_declares_every_direction() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    let found = names(&mut vm);

    for dir in [
        "north", "south", "east", "west", "northeast", "northwest", "southeast",
        "southwest", "up", "down", "in", "out",
    ] {
        assert!(found.iter().any(|n| n == dir), "missing direction `{dir}`");
    }

    // Every direction that can be authored as an exit can be walked. A stair
    // you can describe and cannot climb is one only an admin with `goto` uses.
    let missing = vm
        .eval(
            "local reg = require('lib.commands').registry() \
             local out = {} \
             for dir in pairs(require('lib.movement').OPPOSITES) do \
               if not reg[dir] then out[#out+1] = dir end end \
             table.sort(out) return table.concat(out, ',')",
        )
        .unwrap();
    assert_eq!(missing, "", "directions with no command: {missing}");
}

#[test]
fn direction_aliases_survive_the_collapse() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    let resolve = |vm: &mut RealVm, alias: &str| {
        vm.eval(&format!(
            "return tostring(require('lib.commands').resolve('{alias}'))"
        ))
        .unwrap()
    };

    assert_eq!(resolve(&mut vm, "n"), "north");
    assert_eq!(resolve(&mut vm, "sw"), "southwest");
    assert_eq!(resolve(&mut vm, "d"), "down");

    // `i` stays `inventory`: it has been for as long as MUDs have had one, and
    // `in` deliberately takes no single-letter alias because of it.
    assert_eq!(resolve(&mut vm, "i"), "inventory");
}

#[test]
fn u_is_up_and_nothing_else_claims_it() {
    // `up.lua` and `use.lua` both wrote `_aliases['u']`. Which one won depended
    // on the order the filesystem listed them in, so `u` was undefined.
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    assert_eq!(
        vm.eval("return tostring(require('lib.commands').resolve('u'))").unwrap(),
        "up"
    );

    let claimants = vm
        .eval(
            "local out = {} \
             for name, mod in pairs(require('lib.commands').registry()) do \
               for _, a in ipairs(mod.aliases or {}) do \
                 if a == 'u' then out[#out+1] = name end end end \
             table.sort(out) return table.concat(out, ',')",
        )
        .unwrap();
    assert_eq!(claimants, "up", "exactly one command may claim `u`");
}

#[test]
fn no_alias_is_claimed_twice_and_none_shadows_a_command() {
    // The general form of the `u` bug, so the next one is caught on the day it
    // lands rather than by somebody noticing a verb doing the wrong thing.
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    let dupes = vm
        .eval(
            "local reg = require('lib.commands').registry() \
             local owner, dupes = {}, {} \
             for name, mod in pairs(reg) do \
               for _, a in ipairs(mod.aliases or {}) do \
                 if owner[a] then dupes[#dupes+1] = a .. '(' .. owner[a] .. '/' .. name .. ')' \
                 else owner[a] = name end end end \
             table.sort(dupes) return table.concat(dupes, ' ')",
        )
        .unwrap();
    assert_eq!(dupes, "", "two commands claim the same alias: {dupes}");

    // An alias equal to another command's canonical name never resolves —
    // `lazy_load` checks the registry first. `quests` aliased `journal` and it
    // was dead the whole time.
    let shadowed = vm
        .eval(
            "local reg = require('lib.commands').registry() \
             local out = {} \
             for name, mod in pairs(reg) do \
               for _, a in ipairs(mod.aliases or {}) do \
                 if reg[a] and a ~= name then out[#out+1] = a .. '(' .. name .. ')' end end end \
             table.sort(out) return table.concat(out, ' ')",
        )
        .unwrap();
    assert_eq!(shadowed, "", "alias shadowed by a real command: {shadowed}");
}

#[test]
fn help_lists_the_categories_a_new_player_needs_first() {
    // CATEGORY_ORDER said "movement"; no command has ever used that category,
    // so navigation fell into the alphabetical overflow and printed after the
    // admin block.
    let mut vm = RealVm::boot_real_mudlib(0);
    let out = vm.command("help");

    let nav = out.find("navigation").or_else(|| out.find("Navigation"));
    let items = out.find("items").or_else(|| out.find("Items"));
    assert!(nav.is_some(), "help should have a navigation section:\n{out}");
    assert!(items.is_some(), "help should have an items section:\n{out}");
    assert!(
        nav < items,
        "navigation should come before items:\n{out}"
    );
    assert!(out.contains("north"), "the directions should be listed:\n{out}");
}

#[test]
fn a_directory_is_a_category_not_a_prefix() {
    // The path is not part of the verb: `cmds/admin/spawn.lua` is `spawn`.
    // Registration keys on `M.name`, and this pins that it stays that way.
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    let found = names(&mut vm);
    assert!(
        !found.iter().any(|n| n.contains('.') || n.contains('/')),
        "a command name should never carry its directory: {found:?}"
    );
}
