//! The line editor, and the one property that makes it safe.
//!
//! Nothing in the repository could accept multi-line input: `pager_d` is
//! output-only, `login.lua` is reachable only from `on_input`, and
//! `game/cmds/board.lua` fakes a two-field post with a `|` pipe because of it. A
//! room description is six lines of prose, so OLC needed one.
//!
//! What it must not do is *lose* input. While it is open every line is text, and
//! the interception has to happen above command dispatch — otherwise typing
//! "look at the far wall" in the middle of a description runs `look`.

use crate::common::RealVm;

/// While the editor is open, a verb is **text**, not a command.
///
/// The property the whole design rests on. `quit` inside a description is
/// ordinary English, and losing an hour's writing to it would not be — so `.q`
/// is the only way out, and nothing else is interpreted.
#[test]
fn a_verb_typed_into_the_editor_is_buffered_not_executed() {
    let mut vm = RealVm::boot_with_fixture_world(0);
    vm.command("pagesize 0");

    vm.lua(
        "DAEMON.editor.open(SESSION, { title = 'test', on_save = function(text) SAVED = text end })",
    );

    // Three lines that are all real commands.
    let out = vm.command("look");
    assert!(
        !out.contains("Obvious exits"),
        "`look` was executed instead of buffered:\n{out}"
    );
    vm.command("quit");
    vm.command("who");

    vm.command(".s");
    assert_eq!(
        vm.lua("return SAVED"),
        "look\nquit\nwho",
        "the buffer should hold exactly what was typed"
    );

    // …and the session is still here, which `quit` would have ended.
    assert!(vm.command("look").contains("Obvious exits"), "the session ended");
}

/// A blank line is a paragraph break, not nothing.
///
/// The interception sits *above* dispatch's empty-line check for this reason: an
/// editor that swallows blank lines cannot type a paragraph.
#[test]
fn a_blank_line_is_kept() {
    let mut vm = RealVm::boot_with_fixture_world(0);
    vm.lua(
        "DAEMON.editor.open(SESSION, { on_save = function(t) SAVED = t end })",
    );
    vm.command("First paragraph.");
    vm.command("");
    vm.command("Second paragraph.");
    vm.command(".s");

    assert_eq!(vm.lua("return SAVED"), "First paragraph.\n\nSecond paragraph.");
}

/// `..` escapes a leading dot, so prose may begin with one.
#[test]
fn a_literal_leading_dot_can_be_typed() {
    let mut vm = RealVm::boot_with_fixture_world(0);
    vm.lua("DAEMON.editor.open(SESSION, { on_save = function(t) SAVED = t end })");
    vm.command("..s is how you save.");
    vm.command(".s");

    assert_eq!(vm.lua("return SAVED"), ".s is how you save.");
}

/// The buffer is pre-loaded, so editing is editing rather than retyping.
#[test]
fn an_existing_value_is_loaded_for_editing() {
    let mut vm = RealVm::boot_with_fixture_world(0);
    vm.command("pagesize 0");

    // Straight through `command`, because what is being checked here is the
    // *banner* rather than a returned value — `lua` gives you the latter.
    let out = vm.command(
        "fixtureeval DAEMON.editor.open(SESSION, { title = 'crypt.hall.description', \
           initial = 'Line one.\\nLine two.', on_save = function(t) SAVED = t end })",
    );
    assert!(out.contains("2 lines"), "{out}");
    assert!(out.contains("Line one."), "the existing text should be shown:\n{out}");

    // Append and save: what comes back is the old text plus the new line.
    vm.command("Line three.");
    vm.command(".s");
    assert_eq!(vm.lua("return SAVED"), "Line one.\nLine two.\nLine three.");
}

/// The line commands do what they say.
#[test]
fn lines_can_be_deleted_inserted_and_cleared() {
    let mut vm = RealVm::boot_with_fixture_world(0);
    vm.command("pagesize 0");
    vm.lua(
        "DAEMON.editor.open(SESSION, { initial = 'a\\nb\\nc', on_save = function(t) SAVED = t end })",
    );

    vm.command(".d 2");
    vm.command(".i 1 zero");
    vm.command(".s");
    assert_eq!(vm.lua("return SAVED"), "zero\na\nc");

    // `.c` empties it, and `.q` discards rather than saving.
    vm.lua(
        "ABORTED = false DAEMON.editor.open(SESSION, { initial = 'x', \
           on_save = function() SAVED = 'saved' end, on_abort = function() ABORTED = true end })",
    );
    vm.command(".c");
    vm.command(".q");
    assert_eq!(vm.lua("return tostring(ABORTED)"), "true");
    assert_eq!(vm.lua("return SAVED"), "zero\na\nc", "abort must not save");
}

/// An unknown dot-command is refused rather than buffered.
///
/// Buffering it would put `.savee` in the middle of somebody's prose and look
/// like the editor had simply ignored them.
#[test]
fn an_unknown_dot_command_is_refused() {
    let mut vm = RealVm::boot_with_fixture_world(0);
    vm.lua("DAEMON.editor.open(SESSION, { on_save = function(t) SAVED = t end })");

    let out = vm.command(".savee");
    assert!(out.contains("Unknown command"), "{out}");
    vm.command(".s");
    assert_eq!(vm.lua("return SAVED"), "", "the typo should not be in the buffer");
}

/// Closing releases the session, so commands work again.
#[test]
fn closing_the_editor_gives_the_session_back() {
    let mut vm = RealVm::boot_with_fixture_world(0);

    vm.lua("DAEMON.editor.open(SESSION, {})");
    assert!(
        !vm.command("look").contains("Obvious exits"),
        "the editor should be holding input"
    );
    vm.command(".q");
    assert!(vm.command("look").contains("Obvious exits"), "the session stayed held");
}

// ─── daemon-level, through the probe VM ──────────────────────────────────────
//
// `fixtureeval` is a *command*, so while the editor is open it is buffered as
// prose like everything else — which is the feature working. Anything that has
// to inspect editor state while it is open therefore asks the daemon directly,
// where dispatch is not involved at all.

/// A dropped connection releases the editor.
///
/// Left open, every subsequent line from a reconnecting session would be
/// buffered as prose into a buffer nobody will ever save — a wedged session with
/// no visible cause and no way out.
#[test]
fn a_disconnect_releases_the_editor() {
    let mut vm = RealVm::boot_fixture_with_probe();

    assert_eq!(
        vm.eval("DAEMON.editor.open('s1', {}) return tostring(DAEMON.editor.is_editing('s1'))")
            .unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval("DAEMON.editor.cleanup('s1') return tostring(DAEMON.editor.is_editing('s1'))")
            .unwrap(),
        "false"
    );

    // `cleanup` runs no callback — a dropped connection is not a save and not
    // an abort, and firing either would be inventing an intention.
    assert_eq!(
        vm.eval(
            "FIRED = false DAEMON.editor.open('s2', { on_abort = function() FIRED = true end }) \
             DAEMON.editor.cleanup('s2') return tostring(FIRED)"
        )
        .unwrap(),
        "false"
    );
}

/// The buffer is bounded, so a runaway paste stops rather than growing.
#[test]
fn the_buffer_has_a_limit() {
    let mut vm = RealVm::boot_fixture_with_probe();

    let out = vm
        .eval(
            "local n = DAEMON.editor.MAX_LINES \
             DAEMON.editor.open('s1', { on_save = function(_, lines) COUNT = #lines end }) \
             for i = 1, n + 5 do DAEMON.editor.handle_input('s1', 'line ' .. i) end \
             DAEMON.editor.save('s1') return COUNT .. '|' .. n",
        )
        .unwrap();
    let (kept, cap) = out.split_once('|').unwrap();
    assert_eq!(kept, cap, "the buffer grew past its limit");
}

/// One session at a time. Opening over an open editor would silently discard
/// whatever was already in it.
#[test]
fn a_second_open_is_refused() {
    let mut vm = RealVm::boot_fixture_with_probe();

    assert_eq!(
        vm.eval(
            "DAEMON.editor.open('s1', { initial = 'kept' }) \
             local second = DAEMON.editor.open('s1', { initial = 'lost' }) \
             DAEMON.editor.open = DAEMON.editor.open \
             local held = '' \
             DAEMON.editor.stop('s1') \
             return tostring(second)"
        )
        .unwrap(),
        "false"
    );
}
