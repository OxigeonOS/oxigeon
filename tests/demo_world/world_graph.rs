//! The world is walkable.
//!
//! Three of these were real and none of them was visible from any other test:
//!
//!   * the Wizard's Workshop had no exit to anywhere, and it is the **start
//!     room** — a new character could not reach a single other area on foot;
//!   * the West Gate had no exit into Greywater Marsh, though the marsh linked
//!     back to it, so the whole area was unreachable;
//!   * `up`, `down`, `in` and `out` were in `movement.OPPOSITES` and used by
//!     area files from the beginning, and **had no commands** — a stair you
//!     can describe and cannot climb.
//!
//! Every one of those was invisible to a test suite that moved with `goto`.
//! This is the test that walks instead.


use crate::common::RealVm;

/// One line of Lua, because the probe input path is line-oriented.
const AUDIT: &str = "local OPP = require('lib.movement').OPPOSITES \
    local g = DAEMON.world.exit_graph() \
    local dangling, oneway = {}, {} \
    local ids = {} for id in pairs(g) do ids[#ids+1] = id end table.sort(ids) \
    for _, id in ipairs(ids) do \
      local dirs = {} for d in pairs(g[id]) do dirs[#dirs+1] = d end table.sort(dirs) \
      for _, dir in ipairs(dirs) do \
        local t = g[id][dir] \
        if not g[t] then dangling[#dangling+1] = id..' '..dir..' -> '..t \
        else local back = OPP[dir] \
          if back and g[t][back] ~= id then \
            oneway[#oneway+1] = id..' '..dir..' -> '..t..' (no '..back..' back)' end \
        end end end \
    return 'D=' .. table.concat(dangling, ' | ') .. ' ;O=' .. table.concat(oneway, ' | ')";

/// No exit points at a room that does not exist.
///
/// A dangling exit is not caught by anything else: `get_room` returns nil, the
/// move is refused, and the player is told "that exit leads nowhere" — which
/// reads as intentional.
#[test]
fn no_exit_leads_nowhere() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    let report = vm.eval(AUDIT).unwrap();
    let dangling = report
        .split(";O=")
        .next()
        .unwrap()
        .trim_start_matches("D=")
        .trim();
    assert!(dangling.is_empty(), "exits pointing at nothing:\n  {dangling}");
}

/// Every exit with a known opposite has a matching return.
///
/// One-way exits are legitimate — a trapdoor, a teleport — but they should be
/// *chosen*. An accidental one makes an area reachable and not leavable, or
/// leavable and not reachable, and the only way to find out is to walk it.
/// When a genuinely one-way exit is wanted, use a direction with no opposite
/// or take it out of the graph.
#[test]
fn every_exit_has_a_way_back() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    let report = vm.eval(AUDIT).unwrap();
    let oneway = report.split(";O=").nth(1).unwrap_or("").trim();
    assert!(
        oneway.is_empty(),
        "one-way exits — deliberate or not, say which:\n  {}",
        oneway.replace(" | ", "\n  ")
    );
}

/// Every area is reachable on foot from the start room.
///
/// The assertion the workshop failed: it was a sealed pocket containing the
/// place every new character begins.
#[test]
fn every_area_is_reachable_from_the_start_room() {
    // The probe harness does not set a start room — it never logs anybody in —
    // so the example world's own entrance is passed explicitly.
    //
    // This used to read `start_room` out of `config/server.toml`, on the
    // argument that moving the start room should re-point the test rather than
    // silently exempt it. That was right while the config described the world
    // under test. It no longer does: `server.toml` points at `game/`, the game
    // being developed, so reading it sent this flood fill at a room
    // `game.example/` has never contained and the whole graph came back
    // unreachable.
    let start = crate::common::EXAMPLE_START_ROOM;

    let mut vm = RealVm::boot_real_mudlib_with_probe_opts(crate::common::TestCtx {
        start_room: Some(start.to_string()),
        ..Default::default()
    });

    assert_eq!(
        vm.eval("return config('game.start_room')").unwrap(),
        start,
        "the start room did not reach the VM"
    );

    // Flood fill from the start room over the static graph.
    let reached = vm
        .eval(&format!(
            "local g = DAEMON.world.exit_graph() \
             local seen, queue, head = {{ ['{start}'] = true }}, {{ '{start}' }}, 1 \
             while head <= #queue do local id = queue[head] head = head + 1 \
               for _, t in pairs(g[id] or {{}}) do \
                 if g[t] and not seen[t] then seen[t] = true queue[#queue+1] = t end end end \
             local areas = {{}} \
             for id in pairs(seen) do areas[id:match('^([^.]+)') or '?'] = true end \
             local out = {{}} for a in pairs(areas) do out[#out+1] = a end \
             table.sort(out) return table.concat(out, ',')"
        ))
        .unwrap();

    for area in ["wizard_workshop", "thornhollow", "greywater_marsh", "collapsed_mine"] {
        assert!(
            reached.contains(area),
            "'{area}' cannot be walked to from {start} — reached: {reached}"
        );
    }
}

/// Every direction an area file uses has a command behind it.
///
/// `up`, `down`, `in` and `out` were used by rooms and had no verb, so the
/// stair down from the square had never been walkable by a player. A test that
/// moved with `goto` could not see it.
#[test]
fn every_direction_used_has_a_command() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    let used = vm
        .eval(
            "local g = DAEMON.world.exit_graph() local set = {} \
             for _, exits in pairs(g) do for dir in pairs(exits) do set[dir] = true end end \
             local out = {} for d in pairs(set) do out[#out+1] = d end \
             table.sort(out) return table.concat(out, ',')",
        )
        .unwrap();

    let registry = vm
        .eval(
            "local t = {} for name, mod in pairs(require('lib.commands').registry()) do \
             t[#t+1] = name end table.sort(t) return table.concat(t, ',')",
        )
        .unwrap();

    for dir in used.split(',').filter(|d| !d.is_empty()) {
        assert!(
            registry.split(',').any(|c| c == dir),
            "rooms use '{dir}' as an exit and no command walks it"
        );
    }

    // And the four that were missing are present by name, so this cannot pass
    // by nobody happening to use them.
    for dir in ["up", "down", "in", "out"] {
        assert!(
            registry.split(',').any(|c| c == dir),
            "'{dir}' has no command"
        );
    }
}

/// The opening walk of the demo world, as the guide describes it. If a
/// direction in `demo-world/` is wrong, this is what says so.
#[test]
fn the_guides_opening_route_works() {
    let mut vm = RealVm::boot_real_mudlib(0);

    let steps: &[(&str, &str)] = &[
        ("look", "Entrance to the Workshop"),
        ("north", "The Alchemical Laboratory"),
        ("south", "Entrance to the Workshop"),
        ("east", "The Undercroft"),
        ("up", "The Undercroft Stair"),
        ("up", "Thornhollow Square"),
        ("north", "Bellow & Son"),
        ("south", "Thornhollow Square"),
        ("east", "The Market Arcade"),
        ("west", "Thornhollow Square"),
        ("west", "The West Gate"),
        ("west", "The Head of the Causeway"),
        ("east", "The West Gate"),
        ("east", "Thornhollow Square"),
        ("north", "Bellow & Son"),
        ("down", "The Mine Adit"),
        ("up", "Bellow & Son"),
    ];

    for (command, expected) in steps {
        let out = vm.command(command);
        assert!(
            out.contains(expected),
            "`{command}` should have arrived at {expected:?}, got:\n{out}"
        );
    }
}
