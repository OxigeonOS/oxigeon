//! The `olc` verb: what a builder can actually do without leaving the game.
//!
//! Before this, OLC could enter an area and dig rooms. It could not make an
//! item, a creature or a tag; it could not set a field; and there was no way to
//! see what was settable. Every one of those was blocked on the same missing
//! thing — nothing had written down what an authorable object *is*.
//!
//! These go through the real dispatcher on a playing session, because half of
//! what is being tested is dispatch itself: that the cursor does not follow
//! movement, that `on` is a keyword rather than a guess, that a refusal names
//! the file the value belongs in instead.

use crate::common::RealVm;

/// A VM with a writable game root, logged in, with an OLC-managed area to build.
fn building() -> RealVm {
    let mut vm = RealVm::boot_with_fixture_world(0);
    vm.command("pagesize 0");
    vm.command("olc new area crypt The Sunken Crypt");
    vm
}

/// Creating an area writes the gate, moves you into it, and selects the room.
#[test]
fn a_new_area_is_created_entered_and_selected() {
    let mut vm = RealVm::boot_with_fixture_world(0);
    vm.command("pagesize 0");

    let out = vm.command("olc new area crypt The Sunken Crypt");
    assert!(out.contains("Created area 'crypt'"), "{out}");
    assert!(out.contains("crypt.entrance"), "{out}");

    let where_ = vm.command("olc where");
    assert!(where_.contains("crypt"), "{where_}");
    assert!(where_.contains("crypt.entrance"), "{where_}");

    // The gate is on disk, so the next `olc crypt` is allowed.
    assert_eq!(vm.lua("return tostring((DAEMON.codegen.is_managed('crypt')))"), "true");
}

/// An area OLC does not manage is refused, and told how to opt in.
///
/// This is what keeps a hand-authored area safe: a room may carry an inline
/// action function, and a regeneration would delete it.
///
/// The area is **this test's own**. It used to borrow thornhollow, which made a
/// mudlib test depend on shipped content — and worse, the assertion could not
/// tell "refused because unmanaged" from "refused because there is no such
/// area", so the moment the fixture world replaced that content it would have
/// gone on passing for entirely the wrong reason. Hence the second half.
#[test]
fn an_unmanaged_area_is_refused_and_names_adopt() {
    let mut vm = RealVm::boot_with_fixture_world(0);
    vm.command("pagesize 0");

    // One room, no `_meta.lua` — which is exactly what "not OLC-managed" means.
    let wrote = vm.lua(
        "return tostring((write_file('game:areas/handmade/rooms.lua', \
         [[return { { id = \"handmade.hall\", short = \"A Hall\", \
         description = \"Written by hand.\" } }]])))",
    );
    assert_eq!(wrote, "true", "the unmanaged fixture area was not written");

    let out = vm.command("olc handmade");
    assert!(
        out.contains("olc adopt handmade") || out.contains("_meta.lua"),
        "the refusal should say how to proceed: {out}"
    );

    // …and an area that simply is not there is refused *differently*. Without
    // this, the assertion above passes for any area at all.
    let missing = vm.command("olc no_such_area_exists");
    assert!(
        !missing.contains("olc adopt no_such_area_exists"),
        "a missing area was offered adoption: {missing}"
    );
}

/// `dig` makes a room, both exits, and moves the cursor.
#[test]
fn dig_creates_a_room_and_the_passage_back() {
    let mut vm = building();

    let out = vm.command("dig n hall");
    assert!(out.contains("crypt.hall"), "{out}");
    assert!(out.contains("north"), "{out}");
    assert!(out.contains("south"), "the way back should be made: {out}");

    // Both exits are live, immediately.
    assert_eq!(
        vm.lua("return DAEMON.world.get_room('crypt.entrance'):get_exit('north')"),
        "crypt.hall"
    );
    assert_eq!(
        vm.lua("return DAEMON.world.get_room('crypt.hall'):get_exit('south')"),
        "crypt.entrance"
    );

    // …and the cursor followed, because you just made that room.
    assert!(vm.command("olc where").contains("crypt.hall"), "the cursor did not follow dig");
}

/// `in` and `out` dig both ways.
///
/// `dig` had a private `REVERSE` table with no entry for either, so digging one
/// made a one-way passage and reported success.
#[test]
fn every_direction_including_in_and_out_digs_both_ways() {
    let mut vm = building();

    vm.command("dig in vault");
    assert_eq!(
        vm.lua("return DAEMON.world.get_room('crypt.vault'):get_exit('out')"),
        "crypt.entrance",
        "`in` produced a one-way passage"
    );
}

/// The cursor does **not** follow movement.
///
/// Walking next door to see what an exit looks like from the other side and
/// walking back must not make it fifty-fifty which room the next `set` writes to.
#[test]
fn walking_does_not_move_the_cursor() {
    let mut vm = building();
    vm.command("dig n hall");
    vm.command("olc edit crypt.entrance");

    vm.command("north");
    let out = vm.command("olc where");
    // The labels carry colour tags, so match the values on their own lines
    // rather than the rendered text.
    let cursor_line = out.lines().find(|l| l.contains("Cursor")).unwrap_or("");
    let standing_line = out.lines().find(|l| l.contains("Standing")).unwrap_or("");
    assert!(
        cursor_line.contains("crypt.entrance"),
        "the cursor moved with me:\n{out}"
    );
    assert!(standing_line.contains("crypt.hall"), "{out}");
    // …and it says so, because that is the one state where `set` writes
    // somewhere you are not standing.
    assert!(out.contains("not where you are standing"), "{out}");

    vm.command("olc set short The Entrance Hall");
    assert_eq!(
        vm.lua("return DAEMON.world.get_room('crypt.entrance').short"),
        "The Entrance Hall",
        "the write went to the wrong room"
    );
}

/// `set` writes the draft and the live object at once.
#[test]
fn set_changes_the_world_as_you_type() {
    let mut vm = building();

    let out = vm.command("olc set short The Sunken Entrance");
    assert!(out.contains("The Sunken Entrance"), "{out}");
    assert!(out.contains("(was"), "it should say what it replaced: {out}");

    assert_eq!(
        vm.lua("return DAEMON.world.get_room('crypt.entrance').short"),
        "The Sunken Entrance"
    );
    // Nothing has been written yet — that is what `save` is for.
    assert!(vm.command("olc diff").contains("crypt.entrance"), "should be unsaved");
}

/// `set on <target>` writes elsewhere without moving the cursor.
///
/// `on` is a reserved word rather than a guess. The alternative — deciding by
/// whether the next token happens to resolve as a field — is DWIM on a command
/// that writes files, and the day somebody names an item `damage` it silently
/// writes to the wrong object.
#[test]
fn the_on_keyword_writes_elsewhere_without_moving_the_cursor() {
    let mut vm = building();
    vm.command("dig n hall");

    // Cursor is on crypt.hall after the dig.
    vm.command("olc set on crypt.entrance short The Way Down");

    assert_eq!(
        vm.lua("return DAEMON.world.get_room('crypt.entrance').short"),
        "The Way Down"
    );
    assert!(
        vm.command("olc where").contains("crypt.hall"),
        "the one-shot form moved the cursor"
    );
}

/// Types are enforced at input, with the error saying what is accepted.
#[test]
fn values_are_coerced_and_refused_by_type() {
    let mut vm = building();

    // Integers, with bounds.
    vm.command("olc set light 0");
    assert_eq!(vm.lua("return tostring(DAEMON.world.get_room('crypt.entrance').light_level)"), "0");

    let out = vm.command("olc set light 12");
    assert!(out.contains("maximum"), "{out}");

    let out = vm.command("olc set light dim");
    assert!(out.contains("not a number"), "{out}");

    // A field that does not exist is refused rather than quietly stored.
    let out = vm.command("olc set sparkle yes");
    assert!(out.contains("No field 'sparkle'"), "{out}");
    assert!(out.contains("olc fields"), "it should say where to look: {out}");
}

/// Booleans take eight spellings and nothing is truthy-by-default.
#[test]
fn a_boolean_is_refused_rather_than_guessed() {
    let mut vm = building();
    vm.command("olc new mob picker");

    vm.command("olc set aggressive yes");
    assert_eq!(vm.lua("return tostring(DAEMON.mobs.get('picker').aggressive)"), "true");

    let out = vm.command("olc set aggressive maybe");
    assert!(out.contains("true false yes no on off 1 0"), "{out}");
    assert_eq!(
        vm.lua("return tostring(DAEMON.mobs.get('picker').aggressive)"),
        "true",
        "a refused set must not change anything"
    );
}

/// A map key has to be a keyword, because codegen writes it bare and `examine`
/// and `talk` match on it.
#[test]
fn a_map_field_is_set_one_key_at_a_time() {
    let mut vm = building();

    vm.command("olc set items.well A stone well, worn smooth at the lip.");
    assert_eq!(
        vm.lua("return DAEMON.world.get_room('crypt.entrance').items.well"),
        "A stone well, worn smooth at the lip."
    );

    // A space cannot reach a key: the field and the value are split on
    // whitespace before the key is looked at. What *can* is a character a bare
    // table key may not hold, which is what codegen writes it as.
    let out = vm.command("olc set items.tide-mark A brown line.");
    assert!(out.contains("keyword"), "{out}");
}

/// Lists take `add`/`remove`, never an index and never a comma split.
#[test]
fn tags_are_added_one_at_a_time_and_reach_the_index() {
    let mut vm = building();

    vm.command("olc tag indoor damp");
    assert_eq!(
        vm.lua(
            "local t = DAEMON.world.get_room('crypt.entrance').tags \
             table.sort(t) return table.concat(t, ',')"
        ),
        "damp,indoor"
    );

    // The reverse index is fed, which is what `weather_d` reads.
    assert_eq!(
        vm.lua(
            "for _, id in ipairs(DAEMON.tag.find('room', 'damp')) do \
               if id == 'crypt.entrance' then return 'indexed' end end return 'missing'"
        ),
        "indexed"
    );

    // Adding twice is a no-op rather than a duplicate.
    let out = vm.command("olc tag damp");
    assert!(out.contains("already has"), "{out}");

    vm.command("olc untag indoor");
    assert_eq!(
        vm.lua("return table.concat(DAEMON.world.get_room('crypt.entrance').tags, ',')"),
        "damp"
    );
}

/// An item is created from a base, and its component fields are settable.
#[test]
fn an_item_can_be_made_from_a_component_base() {
    let mut vm = building();

    let out = vm.command("olc new item bone_saw from weapon");
    assert!(out.contains("bone_saw"), "{out}");
    assert!(out.contains("weapon"), "{out}");

    vm.command("olc set short a corroded bone saw");
    vm.command("olc set damage 3-7");
    vm.command("olc set speed 0.8");
    vm.command("olc set weapon_type saw");

    assert_eq!(
        vm.lua(
            "local i = DAEMON.items.get('bone_saw') \
             return i.short .. '|' .. i.weapon.min .. '-' .. i.weapon.max \
                    .. '|' .. i.weapon.speed .. '|' .. i.weapon.weapon_type"
        ),
        "a corroded bone saw|3-7|0.8|saw"
    );

    // The archetype's default reached it, so `Weapon{...}` and OLC agree.
    assert_eq!(vm.lua("return DAEMON.items.get('bone_saw').slot"), "weapon");
}

/// An lfun field is refused, and the refusal says where the value goes.
///
/// Refusing without saying where leaves a builder stuck, and stuck is when
/// people start hand-editing the files OLC regenerates.
#[test]
fn an_lfun_field_is_refused_and_names_custom_lua() {
    let mut vm = building();
    vm.command("olc new item bone_saw from weapon");

    let out = vm.command("olc set hit_message You saw into {target}.");
    assert!(out.contains("custom.lua"), "the refusal should say where it goes:\n{out}");
    assert!(out.contains("lfun"), "{out}");
}

/// Components can be added to a plain item after the fact.
#[test]
fn a_component_can_be_added_and_removed() {
    let mut vm = building();
    vm.command("olc new item satchel");

    assert!(vm.command("olc comp list").contains("(none)"));

    let out = vm.command("olc comp add container");
    assert!(out.contains("container"), "{out}");
    vm.command("olc set capacity 20");
    assert_eq!(vm.lua("return tostring(DAEMON.items.get('satchel').container.capacity)"), "20");

    // Both spellings reach the same component, and the declared one is what is
    // written: `armor.lua` declares `M.component = "armour"`, and a file saying
    // `components = { "armor" }` would not load.
    //
    // Asked of the *built item* rather than of `data.components`: the latter is
    // authoring data and does not survive construction, so checking it would be
    // checking the draft rather than the world.
    vm.command("olc comp add armor");
    assert_eq!(
        vm.lua(
            "local out = {} \
             for _, c in ipairs(require('components').on(DAEMON.items.get('satchel'))) do \
               out[#out+1] = c.component end \
             table.sort(out) return table.concat(out, ',')"
        ),
        "armour,container",
        "both components should be built onto the item"
    );
    assert!(
        vm.command("olc comp list").contains("armour"),
        "the declared spelling should be what is stored"
    );

    vm.command("olc comp remove armour");
    assert_eq!(
        vm.lua(
            "local out = {} \
             for _, c in ipairs(require('components').on(DAEMON.items.get('satchel'))) do \
               out[#out+1] = c.component end \
             return table.concat(out, ',')"
        ),
        "container"
    );
}

/// A creature is created and its stats set.
#[test]
fn a_mob_can_be_made_and_given_stats() {
    let mut vm = building();
    vm.command("dig d ossuary");

    vm.command("olc new mob bone_picker");
    vm.command("olc set name bone picker");
    vm.command("olc set short a stooped bone picker");
    vm.command("olc set stats.level 4");
    vm.command("olc set damage 2-6");
    vm.command("olc set spawn_room crypt.ossuary");
    vm.command("olc set dialogue.bones They come up after rain.");

    assert_eq!(
        vm.lua(
            "local m = DAEMON.mobs.get('bone_picker') \
             return m.name .. '|' .. m.stats.level .. '|' .. m.damage.min .. '-' .. m.damage.max \
                    .. '|' .. m.spawn_room .. '|' .. m.dialogue.bones"
        ),
        "bone picker|4|2-6|crypt.ossuary|They come up after rain."
    );
}

/// A forward reference is accepted, because building in the right order is not
/// how anybody builds.
#[test]
fn a_reference_to_something_not_yet_made_is_accepted() {
    let mut vm = building();
    vm.command("olc new mob picker");

    let out = vm.command("olc set spawn_room crypt.not_dug_yet");
    assert!(!out.contains("{red}"), "a forward reference should be accepted:\n{out}");
    assert_eq!(vm.lua("return DAEMON.mobs.get('picker').spawn_room"), "crypt.not_dug_yet");
}

/// `fields` and `show` are the two halves of "what can I set".
#[test]
fn fields_lists_what_could_be_set_and_show_lists_what_is() {
    let mut vm = building();

    let fields = vm.command("olc fields room");
    assert!(fields.contains("short"), "{fields}");
    assert!(fields.contains("description"), "{fields}");
    assert!(fields.contains("editable"), "the legend should be shown: {fields}");
    // `id` is not editable and says so.
    assert!(fields.contains("id"), "{fields}");

    let show = vm.command("olc show");
    assert!(show.contains("crypt.entrance"), "{show}");
    assert!(show.contains("short"), "{show}");

    // And the per-field help.
    let help = vm.command("olc help light");
    assert!(help.contains("pitch dark"), "{help}");
}

/// `save` writes the files, and `diff` goes quiet afterwards.
#[test]
fn save_writes_the_area_and_clears_the_diff() {
    let mut vm = building();
    let game = vm.game_root().unwrap().to_path_buf();

    vm.command("dig n hall");
    vm.command("olc set short The Hall");
    vm.command("olc new item lantern");
    vm.command("olc set on item:lantern short a tin lantern");

    let out = vm.command("olc save");
    assert!(out.contains("rooms.lua"), "{out}");
    assert!(out.contains("items.lua"), "{out}");
    assert!(out.contains("custom.lua untouched"), "{out}");

    assert!(game.join("areas/crypt/rooms.lua").exists());
    assert!(game.join("areas/crypt/items.lua").exists());
    // No mobs were made, so no `mobs.lua` holding `{}` for somebody to wonder at.
    assert!(!game.join("areas/crypt/mobs.lua").exists());

    assert!(vm.command("olc diff").contains("Nothing unsaved"));

    // And what was written loads back.
    assert_eq!(
        vm.lua("return CG_TEST_ROOMS or (function() \
                  local l = DAEMON.codegen.read('crypt', 'rooms') \
                  for _, r in ipairs(l) do if r.id == 'crypt.hall' then return r.short end end \
                  return 'missing' end)()"),
        "The Hall"
    );
}

/// Leaving with unsaved work is refused rather than silently discarding it.
#[test]
fn leaving_with_unsaved_changes_is_refused() {
    let mut vm = building();
    vm.command("olc set short Something");

    let out = vm.command("olc done");
    assert!(out.contains("unsaved"), "{out}");
    assert!(vm.command("olc where").contains("crypt"), "it let me leave anyway");

    vm.command("olc revert");
    assert!(vm.command("olc done").contains("Left build mode"));
}

/// An unknown subcommand prints the usage, as `role` does.
#[test]
fn an_unknown_subcommand_prints_the_usage() {
    let mut vm = building();

    let out = vm.command("olc frobnicate the widget");
    assert!(out.contains("Unknown option 'frobnicate'"), "{out}");
    assert!(out.contains("olc set"), "the usage should follow: {out}");
}

/// A bare word is an area name, and it is checked **last** — so adding a
/// subcommand can never be shadowed by somebody's area, and adding an area can
/// never shadow a subcommand.
#[test]
fn a_bare_word_is_an_area_and_subcommands_win() {
    let mut vm = building();

    // `save` is a subcommand, not an attempt to enter an area called "save".
    assert!(!vm.command("olc save").contains("not OLC-managed"));
    // An unknown bare word is read as an area.
    let out = vm.command("olc nowhere_at_all");
    assert!(out.contains("nowhere_at_all"), "{out}");
    assert!(out.contains("_meta"), "it should be an area-not-found: {out}");
}

/// **A keyed `record_array` is set one entry at a time**, like a map.
///
/// `loot_table`, `echoes` and `spawn_table` are all `record_array`, and none of
/// them could be set in OLC at all: `schema.set` refused the type outright and
/// pointed at `<field>.<key>`, a syntax that did not work. So a builder could
/// make a room with a spawner and never fill in what it spawned.
///
/// A record declares which of its fields is the address, with `key = true`.
/// Declared rather than inferred — "the first field, if it looks like an id"
/// reads the wrong one the first time somebody writes a record in a different
/// order, and reads it silently.
#[test]
fn a_keyed_record_array_is_set_one_entry_at_a_time() {
    let mut vm = building();

    // The whole-field form is refused, and the refusal names the real syntax.
    let refused = vm.command("olc set spawn_table fixture_mouse 5");
    assert!(
        refused.contains("spawn_table.<template>"),
        "the refusal should name the key field: {refused}"
    );

    let first = vm.command("olc set spawn_table.fixture_mouse 5");
    assert!(first.contains("= 5"), "the new value should be reported: {first}");

    // Setting the same key again **updates** rather than appending — otherwise
    // a builder correcting a weight leaves two entries that disagree.
    let second = vm.command("olc set spawn_table.fixture_mouse 8");
    assert!(second.contains("= 8"), "{second}");
    assert!(
        second.contains("was 5"),
        "the previous value has to be readable, which needs a path-aware read \
         rather than `draft[name]`: {second}"
    );

    // One entry, carrying the second value — not two that disagree.
    let shown = vm.command("olc show");
    let line = shown.lines().find(|l| l.contains("spawn_table")).unwrap_or("");
    assert!(line.contains("fixture_mouse 8"), "{shown}");
    assert_eq!(
        line.matches("fixture_mouse").count(),
        1,
        "setting the same key twice appended instead of updating: {line}"
    );

    // …and it survives the round trip through the generated file.
    vm.command("olc set spawn_max 3");
    vm.command("olc save");
    let written = vm.lua("return tostring(read_file('game:areas/crypt/rooms.lua'))");
    assert!(written.contains("fixture_mouse"), "{written}");
    assert!(written.contains("weight"), "the record form should be emitted: {written}");
}

/// An unkeyed `record_array` says why it cannot be addressed, rather than
/// offering a syntax that does not work.
#[test]
fn an_unkeyed_record_array_says_where_to_write_it_instead() {
    let mut vm = building();
    vm.command("olc new mob crypt_thing");

    // `echoes` has a `bare` field and no key: its natural address is a whole
    // sentence, which is not an address. `olc new mob` leaves the cursor on the
    // creature, so this is a mob field.
    let out = vm.command("olc set echoes something");
    assert!(
        out.contains("no key") && out.contains("area file"),
        "the refusal should say why and where: {out}"
    );
}
