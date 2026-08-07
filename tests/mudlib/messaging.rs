//! One authored line, delivered to a room, read three ways.
//!
//! `lib/render.lua` is tested in the bare VM — this is the half that needs
//! rooms, sessions and live Player objects: who is present, who is excluded, and
//! the cost rule that a broadcast renders once per *role set* rather than once
//! per viewer.
//!
//! Fixture world only.

use crate::common::RealVm;

fn one_line(src: &str) -> String {
    src.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ")
}

struct Vm(RealVm);

impl Vm {
    /// Three probe readers standing in `fixture.hall`, each capturing what it
    /// is sent. Registered with `world` so `get_characters` finds them.
    fn new() -> Self {
        let mut vm = RealVm::boot_fixture_with_probe();
        vm.eval(&one_line(
            "MSG = require('lib.messaging') R = require('lib.render')
             local function reader(id, name, gender)
                 local e = { char_id = id, name = name, gender = gender, room_id = 'fixture.hall', _sent = {} }
                 function e:send(t) self._sent[#self._sent+1] = tostring(t) end
                 return e
             end
             wren = reader(901, 'Wren', 'female')
             bram = reader(902, 'Bram', 'male')
             ash  = reader(903, 'Ash', nil)
             READERS = { wren, bram, ash }
             _real_get = DAEMON.character.get
             DAEMON.character.get = function(cid)
                 for _, r in ipairs(READERS) do if r.char_id == cid then return r end end
                 return _real_get(cid)
             end
             local room = DAEMON.world.get_room('fixture.hall')
             for _, r in ipairs(READERS) do room:add_character(r.char_id) end
             return 'ready'",
        ))
        .unwrap();
        Self(vm)
    }

    fn run(&mut self, src: &str) -> String {
        self.0.eval(&one_line(src)).unwrap()
    }

    /// The last line each reader was sent, joined.
    fn last(&mut self) -> String {
        self.run(
            "local out = {} for _, r in ipairs(READERS) do
                 out[#out+1] = r._sent[#r._sent] or '(nothing)' end
             return table.concat(out, ' | ')",
        )
    }

    fn clear(&mut self) {
        self.run("for _, r in ipairs(READERS) do r._sent = {} end return 'ok'");
    }
}

/// **The headline.** One authored line reaches a room as three sentences.
#[test]
fn one_authored_line_reaches_a_room_as_three_sentences() {
    let mut vm = Vm::new();

    let n = vm.run(
        "return MSG.broadcast('fixture.hall', '$Actor $actor.v(hit) $target.',
             { actor = wren, target = bram })",
    );
    assert_eq!(n, "3", "everybody in the room should be reached");

    assert_eq!(
        vm.last(),
        "You hit Bram. | Wren hits you. | Wren hits Bram.",
        "the actor, the target, and somebody who is neither"
    );
}

/// Ash has no gender and is a player, so singular they — with the verb agreeing.
#[test]
fn a_player_with_no_gender_reads_as_they_with_the_verb_to_match() {
    let mut vm = Vm::new();

    // `.v` agrees with how the role renders: "you swing", "Ash swings".
    // READERS is wren, bram, ash — and ash is the actor.
    vm.run(
        "MSG.broadcast('fixture.hall',
             '$Actor $actor.v(swing) at $target, and $actor $actor.v(be) done.',
             { actor = ash, target = bram }) return 'sent'",
    );
    assert_eq!(
        vm.last(),
        "Ash swings at Bram, and Ash is done. | \
         Ash swings at you, and Ash is done. | \
         You swing at Bram, and you are done.",
        "a name is third-person singular whatever its owner's pronouns are"
    );

    // `.vthey` agrees with the pronoun, which for they/them is plural.
    vm.clear();
    vm.run(
        "MSG.broadcast('fixture.hall', '$Actor.They $actor.vthey(be) bleeding.',
             { actor = ash }) return 'sent'",
    );
    let out = vm.last();
    assert_eq!(out, "They are bleeding. | They are bleeding. | You are bleeding.");
    // The two that read as broken instantly.
    assert!(!out.contains("they swings") && !out.contains("They is"), "{out}");
}

/// A broadcast renders once per distinct role set, not once per viewer.
#[test]
fn a_broadcast_renders_once_per_role_set_not_once_per_viewer() {
    let mut vm = Vm::new();

    // Seven more onlookers, so ten readers share one sentence between eight.
    let out = vm.run(
        "local room = DAEMON.world.get_room('fixture.hall')
         for i = 1, 7 do
             local e = { char_id = 910 + i, name = 'Extra' .. i, _sent = {} }
             function e:send(t) self._sent[#self._sent+1] = tostring(t) end
             READERS[#READERS+1] = e
             room:add_character(e.char_id)
         end
         local calls = 0 local real = R.render
         R.render = function(...) calls = calls + 1 return real(...) end
         local n = MSG.broadcast('fixture.hall', '$Actor $actor.v(hit) $target.',
                                 { actor = wren, target = bram })
         R.render = real
         return n .. '|' .. calls",
    );
    assert_eq!(
        out, "10|3",
        "ten readers, two participants — three distinct sentences and three renders"
    );
}

/// Exclusion, by entity and by char_id.
#[test]
fn a_broadcast_excludes_who_it_is_told_to() {
    let mut vm = Vm::new();

    vm.run(
        "MSG.broadcast('fixture.hall', '$Actor $actor.v(shout).', { actor = wren },
             { exclude = wren }) return 'sent'",
    );
    assert_eq!(vm.last(), "(nothing) | Wren shouts. | Wren shouts.");

    vm.clear();
    vm.run(
        "MSG.broadcast('fixture.hall', '$Actor $actor.v(shout).', { actor = wren },
             { exclude = { wren, 902 } }) return 'sent'",
    );
    assert_eq!(vm.last(), "(nothing) | (nothing) | Wren shouts.");
}

/// `announce` finds the room from whoever is acting, so a caller need not.
#[test]
fn announce_takes_the_room_from_the_actor() {
    let mut vm = Vm::new();

    let n = vm.run(
        "return MSG.announce('$Actor $actor.v(arrive).', { actor = wren })",
    );
    assert_eq!(n, "3");
    assert_eq!(vm.last(), "You arrive. | Wren arrives. | Wren arrives.");

    // An actor who is nowhere reaches nobody, and does not raise.
    let n = vm.run("return MSG.announce('$Actor $actor.v(arrive).', { actor = { name = 'Ghost' } })");
    assert_eq!(n, "0");
}

/// A target with no `send` is skipped rather than raising — a creature is a
/// participant in the sentence, not a reader of it.
#[test]
fn an_entity_with_no_send_is_skipped_rather_than_raising() {
    let mut vm = Vm::new();

    let out = vm.run(
        "local m = DAEMON.mobs.spawn('fixture_mouse', 'fixture.hall')
         local ok, n = pcall(MSG.broadcast, 'fixture.hall', '$Actor $actor.v(hit) $target.',
                             { actor = wren, target = m })
         return tostring(ok) .. '|' .. tostring(n)",
    );
    assert_eq!(out, "true|3", "the three players are reached; the mouse is not a reader");
    assert!(vm.last().contains("You hit mouse."), "{}", vm.last());

    // `tell` to a non-reader is a no-op that reports it did nothing.
    let out = vm.run(
        "local m = DAEMON.mobs.spawn('fixture_mouse', 'fixture.store')
         return tostring(MSG.tell(m, 'You feel odd.', {}))",
    );
    assert_eq!(out, "false");
}

/// `tell` renders for its one reader, so `$actor` says "you" to the right person.
#[test]
fn tell_renders_for_the_one_person_reading_it() {
    let mut vm = Vm::new();

    vm.run("MSG.tell(wren, '$Actor $actor.v(feel) cold.', { actor = wren }) return 'sent'");
    assert_eq!(vm.last(), "You feel cold. | (nothing) | (nothing)");

    vm.clear();
    vm.run("MSG.tell(bram, '$Actor $actor.v(feel) cold.', { actor = wren }) return 'sent'");
    assert_eq!(vm.last(), "(nothing) | Wren feels cold. | (nothing)");
}

/// An empty or absent template sends nothing at all, rather than a blank line.
#[test]
fn an_absent_template_sends_nothing() {
    let mut vm = Vm::new();
    let out = vm.run(
        "local a = MSG.broadcast('fixture.hall', nil, { actor = wren })
         local b = MSG.broadcast('fixture.hall', '', { actor = wren })
         local c = MSG.tell(wren, nil, {})
         return a .. '|' .. b .. '|' .. tostring(c)",
    );
    assert_eq!(out, "0|0|false");
    assert_eq!(vm.last(), "(nothing) | (nothing) | (nothing)");
}
