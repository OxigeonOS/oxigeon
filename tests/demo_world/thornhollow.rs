//! Thornhollow — the town, and the mob fields that had no reader.
//!
//! `Mobile.dialogue` and `Mobile:get_dialogue` had existed since the class did
//! and had **no callers**. `faction` was on every guard and nothing compared
//! two of them. `echoes`, `patrol`, `stationary` and `unique` were the same
//! story. This is the content that reads them, and the assertions that they are
//! read.
//!
//! Also the multi-file area: three room files, one `_meta`, one area in
//! `areas`, one reset.


use crate::common::RealVm;

fn go_to(vm: &mut RealVm, room: &str) {
    let out = vm.command(&format!("goto {room}"));
    assert!(!out.contains("Unknown"), "could not reach {room}:\n{out}");
}

/// One area, three files, one identity.
#[test]
fn the_town_is_one_area_across_three_files() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    for room in [
        "thornhollow.square",        // square.lua
        "thornhollow.market",        // market.lua
        "thornhollow.undercroft",    // undercroft.lua
    ] {
        assert_eq!(
            vm.eval(&format!("return tostring(DAEMON.world.get_room('{room}') ~= nil)")).unwrap(),
            "true",
            "'{room}' is missing — ROOM_D.merge did not join the files"
        );
    }

    // `_meta` from the first source that has one, and exactly one area entry.
    assert_eq!(
        vm.eval("return DAEMON.world.all_area_meta().thornhollow.title").unwrap(),
        "Thornhollow"
    );
    assert_eq!(
        vm.eval("return DAEMON.world.all_area_meta().thornhollow.status").unwrap(),
        "live"
    );

    // Ten rooms across the three files — four, three and three — under one
    // area name. The count is asserted so splitting a file cannot silently
    // drop one.
    assert_eq!(
        vm.eval("return #DAEMON.world.get_area_rooms('thornhollow')").unwrap(),
        "10"
    );
}

/// Room tags feed the reverse index, and the index is what a weather daemon
/// will ask instead of walking the world on every tick.
#[test]
fn rooms_are_tagged_and_the_index_answers_backwards() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    // Forward: does this room have the tag.
    assert_eq!(
        vm.eval("return tostring(DAEMON.world.get_room('thornhollow.square'):has_tag('outdoor'))")
            .unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval("return tostring(DAEMON.world.get_room('thornhollow.undercroft'):has_tag('outdoor'))")
            .unwrap(),
        "false"
    );

    // Backward: which rooms have it. Every area contributes, which is the
    // point — a weather daemon wants the outdoor rooms in the *world*, not in
    // one file. Membership rather than an exact list, so adding an area is not
    // a test edit.
    vm.eval("_out = {} for _, id in ipairs(DAEMON.tag.find('room', 'outdoor')) do \
             _out[id] = true end return 'ok'")
        .unwrap();
    for room in [
        "thornhollow.square",
        "thornhollow.west_gate",
        "greywater_marsh.causeway_head",
    ] {
        assert_eq!(
            vm.eval(&format!("return tostring(_out['{room}'] == true)")).unwrap(),
            "true",
            "'{room}' should be indexed as outdoor"
        );
    }
    assert_eq!(
        vm.eval("return tostring(_out['thornhollow.undercroft'])").unwrap(),
        "nil",
        "an indoor room must not be in the outdoor index"
    );

    // Sorted, so two reads agree — a weather tick that visited rooms in
    // `pairs` order would make a bug that only shows on the third room
    // impossible to reproduce.
    assert_eq!(
        vm.eval(
            "local l = DAEMON.tag.find('room', 'outdoor') \
             for i = 2, #l do if l[i] < l[i-1] then return 'unsorted' end end return 'sorted'"
        )
        .unwrap(),
        "sorted"
    );

    // An intersection, starting from the smaller set.
    assert_eq!(
        vm.eval("return table.concat(DAEMON.tag.find_all('room', { 'underground', 'safe' }), ',')")
            .unwrap(),
        "thornhollow.undercroft"
    );

    // Mobs are indexed too, by instance.
    let merchants: i64 = vm
        .eval("return #DAEMON.tag.find('mob', 'merchant')")
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(merchants, 3, "three shopkeepers, one per shop");

    // And a despawn takes its entry with it — the same lesson as object state.
    vm.eval("_m = DAEMON.mobs.in_room('thornhollow.smithy')[1]").unwrap();
    vm.eval("_id = _m.id DAEMON.mobs.despawn(_m) return 'gone'").unwrap();
    assert_eq!(
        vm.eval("return #DAEMON.tag.tags_of('mob', _id)").unwrap(),
        "0",
        "a despawned mob left its tags in the index"
    );
}

/// `talk` and `ask` — the first callers `Mobile:get_dialogue` has ever had.
#[test]
fn npcs_answer_when_spoken_to() {
    let mut vm = RealVm::boot_real_mudlib(0);
    go_to(&mut vm, "thornhollow.smithy");

    let out = vm.command("talk smith");
    assert!(
        out.contains("Buy something"),
        "the smith did not give her greeting:\n{out}"
    );

    let out = vm.command("ask smith about ore");
    assert!(out.contains("mine"), "no answer about ore:\n{out}");

    // `talk X about Y` is the same as `ask X about Y`, because people type both.
    let out = vm.command("talk smith about mine");
    assert!(out.contains("Collapsed"), "{out}");

    // A topic with no answer says so, rather than falling back to the greeting.
    let out = vm.command("ask smith about the weather");
    assert!(
        out.contains("nothing to say"),
        "an unknown topic should be admitted, not papered over:\n{out}"
    );

    // Somebody who is not here.
    let out = vm.command("talk hobb");
    assert!(out.contains("not here"), "{out}");

    // An lfun answer, which reads the world rather than a table.
    let unarmed = vm.command("ask smith about sword");
    assert!(unarmed.contains("Unarmed"), "expected the unarmed branch:\n{unarmed}");
    vm.command("spawn apprentice_dagger");
    vm.command("wield dagger");
    let armed = vm.command("ask smith about sword");
    assert!(
        armed.contains("That'll do"),
        "the lfun did not see the wielded weapon:\n{armed}"
    );
}

/// `emote` — and the apostrophe case, which is the one people notice.
#[test]
fn emote_says_what_you_are_doing() {
    let mut vm = RealVm::boot_real_mudlib(0);

    let out = vm.command("emote grins.");
    assert!(out.contains("grins."), "{out}");
    assert!(
        out.contains("benchuser grins."),
        "an emote is prefixed with the name and a space:\n{out}"
    );

    // The punctuation alias, which is how everyone actually types it — and
    // which did not dispatch at all until `parse` learned to split a leading
    // punctuation character off the rest.
    let out = vm.command(":grins.");
    assert!(out.contains("benchuser grins."), "the `:` alias did not dispatch:\n{out}");

    let out = vm.command(":'s hand shakes.");
    assert!(
        out.contains("benchuser's hand shakes."),
        "a leading apostrophe should attach to the name, not sit a space away:\n{out}"
    );

    // `say` gets the same fix, and it is the older of the two — `'hello` has
    // been the spelling since MUDs had a `say` command.
    let out = vm.command("'hello there");
    assert!(out.contains("hello there"), "the `'` alias did not dispatch:\n{out}");

    assert!(vm.command("emote").contains("Emote what?"));
}

/// Faction: attacking one guard brings the other, because they share one. That
/// field was on every mob template and nothing had ever compared two of them.
#[test]
fn a_guards_faction_brings_help() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    let guards: i64 = vm
        .eval("return #DAEMON.mobs.in_room('thornhollow.west_gate')")
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(guards, 2, "expected two guards on the gate");

    vm.eval(
        "_p = { char_id = 500, name = 'Fool', inventory = {}, equipment = {}, \
                stats = { level = 3, strength = 10, dexterity = 10, \
                          constitution = 10, hp = 50 }, \
                is_alive = function() return true end, send = function() end }",
    )
    .unwrap();
    vm.eval("setmetatable(_p, { __index = require('lib.mobile') }) return 'ok'").unwrap();
    vm.eval("DAEMON.world.place_character(500, 'thornhollow.west_gate')").unwrap();
    // `character.get` is how aggro_d finds the attacker, so the assist path
    // needs this character to be findable by id.
    vm.eval("DAEMON.character._cache = DAEMON.character._cache or {} return 'ok'").unwrap();

    vm.eval("_g1 = DAEMON.mobs.in_room('thornhollow.west_gate')[1]").unwrap();
    vm.eval("_g2 = DAEMON.mobs.in_room('thornhollow.west_gate')[2]").unwrap();
    assert_eq!(
        vm.eval("return tostring(_g1.faction == _g2.faction)").unwrap(),
        "true",
        "the two guards should share a faction"
    );

    // The assist rule itself, without needing the character cache: two mobs of
    // one faction, one already fighting, and `would_attack` says the other has
    // no quarrel with its own side.
    assert_eq!(
        vm.eval("return tostring(DAEMON.aggro.would_attack(_g1, _g2))").unwrap(),
        "false",
        "a creature should not attack its own faction"
    );

    // And an aggressive creature does not care about someone far above it.
    //
    // Through `set_base` rather than by writing `stats.level` directly: a raw
    // write does not bump the entity, so the memoized value would still be the
    // old one and this would assert against a number nothing can see.
    vm.eval("_g1.aggressive = true DAEMON.trait.set_base(_p, 'level', 40) return 'ok'").unwrap();
    assert_eq!(
        vm.eval("return tostring(DAEMON.aggro.would_attack(_g1, _p))").unwrap(),
        "false",
        "a level-6 guard should ignore a level-40 character"
    );
    vm.eval("DAEMON.trait.set_base(_p, 'level', 3) return 'ok'").unwrap();
    assert_eq!(
        vm.eval("return tostring(DAEMON.aggro.would_attack(_g1, _p))").unwrap(),
        "true"
    );
}

/// `unique` means one, however many times `populate` is called.
#[test]
fn a_unique_creature_stays_unique() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    let count_watchmen = |vm: &mut RealVm| -> i64 {
        vm.eval(
            "local n = 0 for _, id in ipairs(DAEMON.mobs.all()) do end \
             for _, m in ipairs(DAEMON.mobs.in_room('thornhollow.square')) do \
             if m.template_id == 'night_watchman' then n = n + 1 end end return n",
        )
        .unwrap()
        .parse()
        .unwrap()
    };

    assert_eq!(count_watchmen(&mut vm), 1);
    vm.eval("DAEMON.mobs.populate() DAEMON.mobs.populate() return 'ok'").unwrap();
    assert_eq!(
        count_watchmen(&mut vm),
        1,
        "populate is idempotent, and `unique` is what keeps it so for this one"
    );
}

/// Echoes are weighted and may be lfuns. `roll_echo` had one caller and no test
/// that a weighted list picks from the whole list.
#[test]
fn echoes_are_weighted_and_may_be_functions() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval(
        "for _, m in ipairs(DAEMON.mobs.in_room('thornhollow.tavern')) do          if m.template_id == 'tavern_drunk' then _d = m end end return 'found'",
    )
    .unwrap();
    assert_eq!(
        vm.eval("return tostring(#_d.echoes)").unwrap(),
        "3",
        "expected the drunk's three echoes"
    );

    // Every line is reachable. 200 rolls against weights 5/3/1 makes missing
    // the rare one vanishingly unlikely, and the PRNG is seeded now — so this
    // is a real sample rather than the same one every boot.
    vm.eval(
        "_seen = {} for i = 1, 200 do local e = _d:roll_echo() if e then _seen[e] = true end end \
         return 'rolled'",
    )
    .unwrap();
    let distinct: i64 = vm
        .eval("local n = 0 for _ in pairs(_seen) do n = n + 1 end return n")
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(distinct, 3, "a weighted roll should still reach every entry");

    // The apprentice's third echo is an lfun, resolved like any other property.
    // By template rather than by position: `in_room` is sorted by instance id,
    // so which mob is first depends on spawn order across every area.
    vm.eval(
        "for _, m in ipairs(DAEMON.mobs.in_room('thornhollow.smithy')) do \
         if m.template_id == 'forge_apprentice' then _a = m end end return 'found'",
    )
    .unwrap();
    assert_eq!(vm.eval("return tostring(_a ~= nil)").unwrap(), "true");
    vm.eval(
        "_kinds = {} for _, e in ipairs(_a.echoes) do _kinds[type(e.text)] = true end return 'ok'",
    )
    .unwrap();
    assert_eq!(
        vm.eval("return tostring(_kinds['function'] == true and _kinds['string'] == true)").unwrap(),
        "true",
        "the apprentice should have both a plain and an lfun echo"
    );
    assert_eq!(
        vm.eval("return type(_a:roll_echo())").unwrap(),
        "string",
        "roll_echo must resolve an lfun rather than returning the function"
    );
}

/// A room action beats a system command with the same name. `drink` at the well
/// is the well, not a potion — which is the dispatch precedence made visible.
#[test]
fn a_room_action_shadows_a_system_command() {
    let mut vm = RealVm::boot_real_mudlib(0);

    // In the workshop, `drink` is the system command.
    let out = vm.command("drink nothing");
    assert!(
        !out.contains("bucket"),
        "the well action leaked outside its room:\n{out}"
    );

    go_to(&mut vm, "thornhollow.square");
    let out = vm.command("drink");
    assert!(
        out.contains("bucket"),
        "the room action did not take precedence:\n{out}"
    );

    // And the cooldown gates it — per character, in `cooldown_d`, so an area
    // reset cannot turn "once every five minutes" into "whenever".
    let out = vm.command("drink");
    assert!(out.contains("had your fill"), "the cooldown did not hold:\n{out}");
}

/// The vault is a fixed container in a room: it stays where it is and holds
/// what you left in it.
#[test]
fn the_town_vault_is_a_container_in_a_room() {
    let mut vm = RealVm::boot_real_mudlib(0);
    go_to(&mut vm, "thornhollow.undercroft");

    let look = vm.command("look");
    assert!(look.contains("strongbox"), "the vault is not in the room:\n{look}");

    vm.command("spawn brass_key");
    let out = vm.command("put key in strongbox");
    assert!(out.contains("You put"), "{out}");

    let out = vm.command("examine strongbox");
    assert!(out.contains("brass key"), "the vault did not keep it:\n{out}");

    // Leaving and coming back finds it still there — a room's contents are not
    // a player's inventory.
    go_to(&mut vm, "thornhollow.square");
    go_to(&mut vm, "thornhollow.undercroft");
    assert!(vm.command("examine strongbox").contains("brass key"));

    let out = vm.command("get key from strongbox");
    assert!(out.contains("You take"), "{out}");
}

/// The crypt's flagstone: a room action gated by a trait rather than an item.
#[test]
fn the_crypt_flagstone_needs_strength() {
    let mut vm = RealVm::boot_real_mudlib(0);

    // The crypt is unlit. A room action still works in the dark — you can feel
    // for a flagstone — but seeing what it revealed needs a light.
    vm.command("spawn hooded_lantern");
    vm.command("use lantern");
    go_to(&mut vm, "thornhollow.crypt");

    let out = vm.command("pry");
    assert!(
        out.contains("does not move"),
        "a strength-10 character should not lift it:\n{out}"
    );

    vm.command("affect learn strength 16");
    let out = vm.command("pry");
    assert!(out.contains("comes up"), "strength 16 should lift it:\n{out}");
    assert!(vm.command("look").contains("brass key"), "nothing was revealed");

    // Object state remembers, so it does not come up twice.
    let out = vm.command("pry");
    assert!(out.contains("already up"), "{out}");
}
