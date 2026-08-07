//! Generated area files, area discovery, and the reset path.
//!
//! The round trip the old OLC documented and could not complete. Four things had
//! to be true at once and none of them were:
//!
//! * the files have to land where the loader looks (`write_file` was jailed to
//!   the mudlib; the world loads from `game/`);
//! * they have to be in a shape the loader reads (one file per room under
//!   `rooms/`, against a loader wanting one array);
//! * the loader has to *find* them (`game/init.lua` named every area by hand);
//! * and `areas reset` has to work afterwards (`olc` never registered a source,
//!   so every area it made answered "No registered source").
//!
//! Through the real VM with both roots writable, because every one of those is a
//! claim about the engine rather than about a function.

use crate::common::RealVm;

fn one_line(src: &str) -> String {
    src.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The real mudlib, the fixture world, and a **writable** game root.
///
/// `boot_two_roots` writes a throwaway probe mudlib, which has no daemons; the
/// fixture boot has the real one and a temp game directory, which is what an
/// area has to be generated into.
struct Vm {
    vm: RealVm,
    game: std::path::PathBuf,
}

impl Vm {
    fn new() -> Self {
        let mut vm = RealVm::boot_fixture_with_probe();
        let game = vm.game_root().expect("the fixture boot owns a game root").to_path_buf();
        vm.eval("CG = require('daemons.codegen_d') SC = require('lib.schema') return 'ready'")
            .unwrap();
        Self { vm, game }
    }
    fn run(&mut self, src: &str) -> String {
        self.vm.eval(&one_line(src)).unwrap()
    }
}

/// A generated room file lands in the game root and reads back as what went in.
#[test]
fn a_generated_room_file_lands_in_the_game_root_and_round_trips() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        local rooms = {
            { id = 'crypt.landing', short = 'The Landing',
              description = 'Water has been over this floor and gone again.\nTwice.',
              light = 1, smell = 'Silt.', tags = { 'indoor', 'damp' },
              exits = { down = 'crypt.cistern', up = 'thornhollow.undercroft' },
              items = { tidemark = 'A brown line, dead level all the way round.' } },
            { id = 'crypt.cistern', short = 'The Cistern', light = 0,
              exits = { up = 'crypt.landing' } },
        }
        local ok, err = CG.write_kind('crypt', 'room', rooms)
        if not ok then return 'write failed: ' .. tostring(err) end
        return 'ok'
    "#,
    );
    assert_eq!(out, "ok");

    // Where the loader looks, not where the old codegen wrote.
    assert!(
        vm.game.join("areas/crypt/rooms.lua").exists(),
        "the file did not reach the game root"
    );

    let back = vm.run(
        r#"
        local list = CG.read('crypt', 'rooms')
        if type(list) ~= 'table' then return 'unreadable' end
        if #list ~= 2 then return 'wrong count: ' .. #list end
        local a = list[1]
        if a.id ~= 'crypt.landing' then return 'id lost' end
        if a.light ~= 1 then return 'light lost' end
        if a.smell ~= 'Silt.' then return 'smell lost' end
        if a.tags[2] ~= 'damp' then return 'tags lost' end
        if a.exits.down ~= 'crypt.cistern' then return 'exit lost' end
        if a.items.tidemark == nil then return 'scenery lost' end
        if not a.description:find('\nTwice.', 1, true) then return 'description lost' end
        return 'ok'
    "#,
    );
    assert_eq!(back, "ok");
}

/// **The defect that made `dig` destructive.** A field the old emitter did not
/// know about was silently dropped on the next write, so digging a second exit
/// out of a room deleted its light level, smell, sound and tags.
#[test]
fn a_rewrite_preserves_every_field_including_ones_no_schema_names() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        local original = { {
            id = 'crypt.hall', short = 'The Hall', description = 'Bones.',
            light = 0, smell = 'Dust.', sound = 'A drip.',
            tags = { 'indoor', 'dark' }, exits = { north = 'crypt.stair' },
            puzzle_seed = 17,
        } }
        CG.write_kind('crypt', 'room', original)

        local list = CG.read('crypt', 'rooms')
        list[1].exits.south = 'crypt.crypt'
        CG.write_kind('crypt', 'room', list)

        local again = CG.read('crypt', 'rooms')[1]
        local lost = {}
        for _, f in ipairs({ 'light', 'smell', 'sound', 'puzzle_seed' }) do
            if again[f] == nil then lost[#lost+1] = f end
        end
        if #again.tags ~= 2 then lost[#lost+1] = 'tags' end
        if again.exits.north == nil then lost[#lost+1] = 'exits.north' end
        if again.exits.south == nil then lost[#lost+1] = 'the new exit' end
        return #lost == 0 and 'ok' or ('lost: ' .. table.concat(lost, ', '))
    "#,
    );
    assert_eq!(out, "ok");
}

/// Writing is idempotent, so a file that has not changed produces no diff.
#[test]
fn writing_the_same_data_twice_produces_the_same_bytes() {
    let mut vm = Vm::new();

    vm.run(
        "local r = { { id = 'crypt.hall', short = 'The Hall', \
                       description = 'Line one.\\nLine two.', \
                       exits = { north = 'a', south = 'b', east = 'c' }, \
                       items = { well = 'A well.', bucket = 'A bucket.' } } } \
         CG.write_kind('crypt', 'room', r) return 'ok'",
    );

    let first = std::fs::read_to_string(vm.game.join("areas/crypt/rooms.lua")).unwrap();
    vm.run("CG.write_kind('crypt', 'room', CG.read('crypt', 'rooms')) return 'ok'");
    let second = std::fs::read_to_string(vm.game.join("areas/crypt/rooms.lua")).unwrap();

    // The header carries a timestamp, so compare the body.
    let body = |s: &str| s[s.find("return {").unwrap()..].to_string();
    assert_eq!(body(&first), body(&second), "a rewrite changed the file");

    // Specifically: the description did not grow whitespace. The old emitter
    // indented the closing `]]`, which put that indentation inside the string
    // and added four spaces on every read-and-rewrite, for ever.
    assert_eq!(
        vm.run("return CG.read('crypt', 'rooms')[1].description"),
        "Line one.\nLine two."
    );
}

/// A room title containing a quote used to produce a file that would not
/// compile, and nothing said so until the next reload.
#[test]
fn hostile_content_still_produces_a_file_that_compiles() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        local r = { {
            id = 'crypt.hall',
            short = 'The "Great" Hall',
            description = 'It has a ]] in it.\nAnd a backslash \\ too.',
            items = { ['sign'] = 'It reads: "Beware".' },
        } }
        local ok, err = CG.write_kind('crypt', 'room', r)
        if not ok then return 'write refused: ' .. tostring(err) end
        local back = CG.read('crypt', 'rooms')
        if type(back) ~= 'table' then return 'did not read back' end
        if back[1].short ~= 'The "Great" Hall' then return 'title mangled' end
        return 'ok'
    "#,
    );
    assert_eq!(out, "ok");

    assert_eq!(
        vm.run("return tostring((verify_file('game:areas/crypt/rooms.lua')))"),
        "true",
        "the generated file should compile"
    );
}

/// An item's components are written flat — the same shape `Weapon{...}` takes —
/// and rebuild into the same item.
#[test]
fn an_item_with_components_round_trips_through_the_flat_form() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        local CO = require('components')
        local items = { {
            id = 'drowned_blade', short = 'a drowned short sword',
            slot = 'weapon', weight = 3, value = 120, tags = { 'weapon' },
            components = { 'weapon' },
            damage = { min = 3, max = 7 }, speed = 1.1,
            weapon_type = 'sword', damage_type = 'physical',
            required_level = 4,
        } }
        CG.write_kind('crypt', 'item', items)

        local back = CG.read('crypt', 'items')[1]
        local item = CO.build(back)
        if not item then return 'did not build' end
        if item.weapon.min ~= 3 or item.weapon.max ~= 7 then return 'damage lost' end
        if item.weapon.speed ~= 1.1 then return 'speed lost' end
        if item.requires.level ~= 4 then return 'requirement lost' end
        if item.slot ~= 'weapon' then return 'slot default lost' end
        return 'ok'
    "#,
    );
    assert_eq!(out, "ok");

    // The section comment is what makes the flat form readable — a wall of
    // twelve keys with no indication which four belong to the weapon is what
    // "flat" would otherwise mean.
    let src = std::fs::read_to_string(vm.game.join("areas/crypt/items.lua")).unwrap();
    assert!(src.contains("-- weapon"), "component blocks should be labelled:\n{src}");
    assert!(src.contains("-- requires"), "{src}");
}

/// A generated file reads in schema order, not alphabetically.
///
/// This one is easy to break without noticing, and it did break: the orderer
/// tests the record's breadcrumb to decide "am I at the top level of a datum",
/// and a record inside the file's array has the breadcrumb `1` rather than `""`.
/// Getting that wrong produced a perfectly valid file with every field sorted
/// A-Z — the schema doing nothing at all, silently.
#[test]
fn a_generated_file_reads_in_schema_order() {
    let mut vm = Vm::new();

    vm.run(
        "CG.write_kind('crypt', 'room', { { id = 'crypt.hall', short = 'The Hall', \
           description = 'Bones.', light = 1, tags = { 'indoor' }, \
           exits = { west = 'a', north = 'b' } } }) return 'ok'",
    );
    let src = std::fs::read_to_string(vm.game.join("areas/crypt/rooms.lua")).unwrap();

    let at = |needle: &str| src.find(needle).unwrap_or_else(|| panic!("missing {needle}:\n{src}"));
    assert!(
        at("id ") < at("short ") && at("short ") < at("description ")
            && at("description ") < at("light ") && at("light ") < at("tags ")
            && at("tags ") < at("exits "),
        "fields are not in schema order:\n{src}"
    );

    // Exits in compass order, from the one direction table.
    assert!(
        src.contains(r#"{ north = "b", west = "a" }"#),
        "exits should be north-before-west:\n{src}"
    );
}

/// `_meta.lua` carries the gate, and an area without it is refused.
///
/// This is what keeps a hand-authored area safe: thornhollow's rooms hold inline
/// action functions, and a regeneration would delete them.
#[test]
fn an_unmanaged_area_is_refused_and_told_how_to_opt_in() {
    let mut vm = Vm::new();

    let out = vm.run(
        "local ok, why = CG.is_managed('crypt') return tostring(ok) .. '|' .. tostring(why)",
    );
    assert!(out.starts_with("false|"), "{out}");
    assert!(out.contains("no _meta.lua"), "{out}");

    vm.run("CG.write_meta('crypt', { title = 'The Sunken Crypt', author = 'Wren' }) return 'ok'");
    assert_eq!(vm.run("return tostring((CG.is_managed('crypt')))"), "true");

    // A `_meta.lua` that exists but says nothing about `managed` is still
    // refused, and the message names the command that would change that.
    vm.run(
        "write_file('game:areas/hand/_meta.lua', 'return { name = \"hand\" }') return 'ok'",
    );
    let out = vm.run(
        "local ok, why = CG.is_managed('hand') return tostring(ok) .. '|' .. tostring(why)",
    );
    assert!(out.starts_with("false|"), "{out}");
    assert!(out.contains("olc adopt hand"), "the refusal should say how: {out}");
}

/// Discovery finds an area nobody listed, loads it, and registers its reset.
///
/// The whole round trip: write files → discover → load → walk it → reset → still
/// there. Every one of those steps was broken.
#[test]
fn a_generated_area_is_discovered_loaded_and_can_be_reset() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        CG.write_meta('crypt', { title = 'The Sunken Crypt', author = 'Wren' })
        CG.write_kind('crypt', 'room', {
            { id = 'crypt.entrance', short = 'The Entrance', description = 'A way down.',
              exits = { down = 'crypt.hall' }, tags = { 'indoor' } },
            { id = 'crypt.hall', short = 'The Hall', description = 'Bones.',
              exits = { up = 'crypt.entrance' } },
        })
        CG.write_kind('crypt', 'item', {
            { id = 'crypt_lantern', short = 'a pitch-sealed lantern', weight = 2 },
        })
        CG.write_kind('crypt', 'mob', {
            { id = 'crypt_eel', name = 'eel', short = 'a grey eel',
              spawn_room = 'crypt.hall', count = 1,
              stats = { hp = 10, max_hp_flat = 10, level = 2 } },
        })
        return 'ok'
    "#,
    );
    assert_eq!(out, "ok");

    // Discovery, with nothing having named it.
    assert_eq!(
        vm.run("local a = require('lib.areaload') return table.concat(a.discover(), ',')"),
        "crypt"
    );

    let loaded = vm.run(
        "local a = require('lib.areaload') local ok, err = a.load('crypt') \
         return tostring(ok) .. '|' .. tostring(err)",
    );
    assert_eq!(loaded, "true|nil");

    // The rooms are in the world, with their exits.
    assert_eq!(
        vm.run("return DAEMON.world.get_room('crypt.hall').short"),
        "The Hall"
    );
    assert_eq!(
        vm.run("return DAEMON.world.get_room('crypt.entrance'):get_exit('down')"),
        "crypt.hall"
    );
    assert_eq!(vm.run("return DAEMON.items.get('crypt_lantern').short"), "a pitch-sealed lantern");
    assert_eq!(vm.run("return DAEMON.mobs.get('crypt_eel').name"), "eel");

    // **`areas reset` works without anybody having remembered to register it.**
    // `olc` forgot for its entire existence, so every area it made was
    // un-resettable and gone on the next boot.
    let reset = vm.run(
        "local ok, msg = DAEMON.world.reset_area('crypt') \
         return tostring(ok) .. '|' .. tostring(msg)",
    );
    assert!(reset.starts_with("true|"), "the reset failed: {reset}");
    assert_eq!(
        vm.run("return DAEMON.world.get_room('crypt.hall').short"),
        "The Hall",
        "the area did not survive its own reset"
    );
}

/// `custom.lua` is merged over the generated data, and never written to.
#[test]
fn custom_lua_patches_the_generated_data_and_is_never_regenerated() {
    let mut vm = Vm::new();

    vm.run(
        r#"
        CG.write_meta('crypt', { title = 'Crypt' })
        CG.write_kind('crypt', 'room', {
            { id = 'crypt.hall', short = 'The Hall', description = 'A placeholder.',
              exits = { north = 'crypt.stair' }, items = { bone = 'A bone.' } },
        })
        write_file('game:areas/crypt/custom.lua', [==[
            return {
                rooms = {
                    ['crypt.hall'] = {
                        description = function(room) return 'Computed prose.' end,
                        actions = { pull = { func = function() end, hint = 'pull' } },
                        items = { chain = 'A taut chain.' },
                    },
                },
            }
        ]==])
        local a = require('lib.areaload')
        local ok, err = a.load('crypt')
        if not ok then return 'load failed: ' .. tostring(err) end
        return 'ok'
    "#,
    );

    let room = vm.run(
        "local r = DAEMON.world.get_room('crypt.hall') \
         return type(r.long) .. '|' .. tostring(r.actions and r.actions.pull ~= nil) \
                .. '|' .. tostring(r.items.chain ~= nil) .. '|' .. tostring(r.items.bone ~= nil)",
    );
    assert_eq!(
        room, "function|true|true|true",
        "an lfun description, an action, and a MERGED items map"
    );

    // A `map` field merges key by key; everything else replaces. The schema
    // decides that, not a guess about shape — which would get it wrong the first
    // time somebody patches an empty `exits`.
    let before = std::fs::read_to_string(vm.game.join("areas/crypt/custom.lua")).unwrap();
    vm.run("CG.write_kind('crypt', 'room', CG.read('crypt', 'rooms')) return 'ok'");
    let after = std::fs::read_to_string(vm.game.join("areas/crypt/custom.lua")).unwrap();
    assert_eq!(before, after, "codegen touched custom.lua");
}

/// A patch naming an id nothing declares is a warning, not a silence.
///
/// It is how a room you renamed in OLC gets noticed: the patch carrying its
/// actions now points at nothing, and the room has quietly lost its behaviour.
#[test]
fn a_patch_for_a_missing_id_is_reported() {
    let mut vm = Vm::new();

    let out = vm.run(
        "local p = require('lib.patch') \
         local list = { { id = 'crypt.hall' } } \
         local _, report = p.apply('room', list, { ['crypt.gone'] = { short = 'x' } }) \
         return report.applied .. '|' .. table.concat(report.orphans, ',')",
    );
    assert_eq!(out, "0|crypt.gone");
}



/// `item_d` feeds the tag index, which it never did.
///
/// `DAEMON.tag.find("item", "weapon")` came back empty for every item in the
/// game while `Item.tags` was widely authored and `Item:has_tag` worked — two
/// ways to ask one question, one of which was always wrong.
#[test]
fn item_tags_reach_the_tag_index() {
    let mut vm = RealVm::boot_fixture_with_probe();

    let found = vm
        .eval("return #DAEMON.tag.find('item', 'weapon')")
        .unwrap()
        .parse::<i64>()
        .unwrap();
    assert!(found > 0, "no item is indexed under 'weapon'");
}
