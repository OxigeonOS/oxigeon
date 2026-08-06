//! The file efuns are jailed to **two** roots, **through the engine's VM**.
//!
//! `write_file` reached only the mudlib. The world loads its content from
//! `game/`, so every file OLC ever generated landed in `mudlib/areas/` — a
//! directory that does not exist in this repository — while `dig` told the
//! builder "File written: game/areas/…". Nothing OLC created could load,
//! because nothing was where the loader looks.
//!
//! Widening the jail is a security change, so these ask the question the way
//! `CLAUDE.md`'s testing section requires: what can Lua running in *this server*
//! actually reach? `efuns_io.rs` has unit tests for the resolver and they were
//! green the whole time the reachable `list_dir` was a different, unjailed
//! function — which is the bug that rule was written about.

mod common;

use common::RealVm;

/// The repository's own `permissions.toml`. The harness default has no rules at
/// all, so a test against it would pass whether a rule existed or not.
fn shipped_permissions() -> oxigeon::config::PermissionConfig {
    oxigeon::config::PermissionConfig::load_from_file(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("config/permissions.toml")
            .as_path(),
    )
}

/// The bug in one assertion: a builder's file has to land where the loader looks.
#[test]
fn a_prefixed_write_lands_in_the_game_root() {
    let (mut vm, mudlib, game) = RealVm::boot_two_roots(Default::default());

    assert_eq!(
        vm.eval(
            "local ok, err = write_file('game:areas/crypt/rooms.lua', 'return {}') \
             return tostring(ok) .. '|' .. tostring(err)"
        )
        .unwrap(),
        "true|nil"
    );

    assert!(
        game.join("areas/crypt/rooms.lua").exists(),
        "the write did not reach the game root"
    );
    assert!(
        !mudlib.join("areas/crypt/rooms.lua").exists(),
        "the write also reached the mudlib — it should have gone to exactly one root"
    );
    assert_eq!(
        vm.eval("return tostring(file_root('areas/crypt/rooms.lua'))").unwrap(),
        "game"
    );
}

/// The back-compatibility that let every existing caller stay put.
///
/// `audit_d` writes `logs/audit_watch.json` and creates it on first use. A rule
/// sending new files to the game root would have relocated it, a later read
/// would still have found it through the fallback, and the two copies would
/// have drifted with nothing reporting it.
#[test]
fn an_unprefixed_write_still_means_the_mudlib() {
    let (mut vm, mudlib, game) = RealVm::boot_two_roots(Default::default());

    assert_eq!(
        vm.eval("return tostring(write_file('logs/audit_watch.json', '{}'))").unwrap(),
        "true"
    );
    assert!(mudlib.join("logs/audit_watch.json").exists());
    assert!(!game.join("logs/audit_watch.json").exists());
}

/// A read prefers the layer that would be `require`d, matching `package.path`
/// and what `list_dir` has always done.
#[test]
fn a_read_prefers_the_game_layer_and_falls_back_to_the_mudlib() {
    let (mut vm, mudlib, game) = RealVm::boot_two_roots(Default::default());

    std::fs::create_dir_all(mudlib.join("shadowed")).unwrap();
    std::fs::create_dir_all(game.join("shadowed")).unwrap();
    std::fs::write(mudlib.join("shadowed/both.lua"), "mudlib version").unwrap();
    std::fs::write(game.join("shadowed/both.lua"), "game version").unwrap();
    std::fs::write(mudlib.join("shadowed/only_mudlib.lua"), "mudlib only").unwrap();

    assert_eq!(vm.eval("return read_file('shadowed/both.lua')").unwrap(), "game version");
    assert_eq!(vm.eval("return tostring(file_root('shadowed/both.lua'))").unwrap(), "game");

    // The fallback: a file only one layer has is still found.
    assert_eq!(vm.eval("return read_file('shadowed/only_mudlib.lua')").unwrap(), "mudlib only");
    assert_eq!(
        vm.eval("return tostring(file_root('shadowed/only_mudlib.lua'))").unwrap(),
        "mudlib"
    );

    // Naming the root reaches the shadowed one, which is the point of saying so.
    assert_eq!(
        vm.eval("return read_file('mudlib:shadowed/both.lua')").unwrap(),
        "mudlib version"
    );
    assert_eq!(
        vm.eval("return tostring(read_file('game:shadowed/only_mudlib.lua'))").unwrap(),
        "nil",
        "an explicit root must not fall back to the other one"
    );
}

/// `file_exists` and `read_file` must agree about which files there are.
/// Two answers to one question is how a caller ends up guarding on the wrong one.
#[test]
fn file_exists_agrees_with_read_file_about_both_roots() {
    let (mut vm, mudlib, game) = RealVm::boot_two_roots(Default::default());
    std::fs::write(game.join("in_game.lua"), "g").unwrap();
    std::fs::write(mudlib.join("in_mudlib.lua"), "m").unwrap();

    for (path, expected) in [
        ("in_game.lua", true),
        ("in_mudlib.lua", true),
        ("in_neither.lua", false),
        ("game:in_game.lua", true),
        ("game:in_mudlib.lua", false),
        ("mudlib:in_mudlib.lua", true),
    ] {
        let exists = vm
            .eval(&format!("return tostring(file_exists('{path}'))"))
            .unwrap();
        let readable = vm
            .eval(&format!("return tostring(read_file('{path}') ~= nil)"))
            .unwrap();
        assert_eq!(exists, expected.to_string(), "file_exists('{path}')");
        assert_eq!(readable, exists, "file_exists and read_file disagree on '{path}'");
    }
}

/// Widening the jail must not open it. A prefix is a choice of root, not a way
/// past the root.
#[test]
fn traversal_out_of_either_root_is_still_refused() {
    let (mut vm, mudlib, game) = RealVm::boot_two_roots(Default::default());

    for attempt in [
        "../escaped.lua",
        "../../escaped.lua",
        "game:../escaped.lua",
        "mudlib:../escaped.lua",
        "game:areas/../../escaped.lua",
        "mudlib:cmds/../../escaped.lua",
    ] {
        assert_eq!(
            vm.eval(&format!(
                "local ok = write_file([[{attempt}]], 'x') return tostring(ok)"
            ))
            .unwrap(),
            "false",
            "write_file({attempt:?}) escaped the jail"
        );
    }

    for parent in [mudlib.parent().unwrap(), game.parent().unwrap()] {
        assert!(
            !parent.join("escaped.lua").exists(),
            "a refused write reached {parent:?} anyway"
        );
    }
}

/// A typo in the root is an error, not a filename.
#[test]
fn an_unknown_root_is_refused_and_says_what_is_valid() {
    let (mut vm, _mudlib, _game) = RealVm::boot_two_roots(Default::default());

    let err = vm
        .eval(
            "local ok, err = write_file('gmae:areas/x.lua', 'x') \
             return tostring(ok) .. '|' .. tostring(err)",
        )
        .unwrap();
    assert!(err.starts_with("false|"), "{err}");
    assert!(err.contains("unknown root"), "{err}");
    assert!(err.contains("game:"), "the message should name the valid roots: {err}");
}

/// **The regression that matters.** The same `[directories]` table governs both
/// roots.
///
/// `list_dir` used to exempt the game root entirely, on the reasoning that
/// `permissions.toml` described only the mudlib. Carrying that into `write_file`
/// would have made `dir.write.game.areas` decorative for *exactly* the files
/// OLC writes — a rule that is a no-op, which is the failure
/// `config/permissions.toml` already documents once about this very directory.
#[test]
fn dir_write_areas_denies_a_session_without_it_for_the_game_root_too() {
    let (mut vm, _mudlib, game) = RealVm::boot_two_roots(shipped_permissions());

    // This session is not playing, so it holds nothing and is not the superuser.
    let out = vm
        .eval(
            "local ok, err = write_file('game:areas/crypt/rooms.lua', 'return {}') \
             return tostring(ok) .. '|' .. tostring(err)",
        )
        .unwrap();
    assert!(
        out.starts_with("false|"),
        "the game area tree is world-writable: {out}"
    );
    assert!(
        out.contains("dir.write.game.areas"),
        "the refusal should name the permission that would allow it: {out}"
    );
    assert!(
        !game.join("areas/crypt/rooms.lua").exists(),
        "the refused write reached disk anyway"
    );

    // A directory with no rule is still writable, so the refusal above is the
    // rule doing its job rather than the game root being read-only.
    assert_eq!(
        vm.eval("return tostring(write_file('game:notes/scratch.txt', 'x'))").unwrap(),
        "true"
    );
}

/// `verify_file` reaches the game root, and reports the file by the name the
/// builder typed.
///
/// It had a second jail of its own — `sandbox::resolve_jailed_path`, mudlib-only
/// and refusing any path containing `..` — so `verify` and `read_file` disagreed
/// about which paths existed, and no game-layer file could be compile-checked
/// at all.
#[test]
fn verify_file_compiles_a_file_in_the_game_root() {
    let (mut vm, _mudlib, game) = RealVm::boot_two_roots(Default::default());

    std::fs::create_dir_all(game.join("areas/crypt")).unwrap();
    std::fs::write(game.join("areas/crypt/rooms.lua"), "return { { id = 'crypt.hall' } }\n").unwrap();
    std::fs::write(game.join("areas/crypt/broken.lua"), "return { id = ,,, }\n").unwrap();

    assert_eq!(
        vm.eval("return tostring((verify_file('game:areas/crypt/rooms.lua')))").unwrap(),
        "true"
    );

    let out = vm
        .eval(
            "local ok, err = verify_file('game:areas/crypt/broken.lua') \
             return tostring(ok) .. '|' .. tostring(err)",
        )
        .unwrap();
    assert!(out.starts_with("false|"), "a broken file should not compile: {out}");
    assert!(
        out.contains("/game/areas/crypt/broken.lua"),
        "the error should name the file by its virtual path: {out}"
    );
}

/// `list_dir` merges the roots when asked for neither, and lists exactly one
/// when asked for one.
///
/// The merge is what command and area discovery want. It is the opposite of what
/// a builder deciding where a file goes wants, so which behaviour you get is
/// chosen at the call site rather than assumed.
#[test]
fn list_dir_merges_by_default_and_narrows_when_a_root_is_named() {
    let (mut vm, mudlib, game) = RealVm::boot_two_roots(Default::default());

    std::fs::create_dir_all(mudlib.join("shared")).unwrap();
    std::fs::create_dir_all(game.join("shared")).unwrap();
    std::fs::write(mudlib.join("shared/from_mudlib.lua"), "m").unwrap();
    std::fs::write(mudlib.join("shared/both.lua"), "m").unwrap();
    std::fs::write(game.join("shared/from_game.lua"), "g").unwrap();
    std::fs::write(game.join("shared/both.lua"), "g").unwrap();

    let names = |vm: &mut RealVm, path: &str| {
        vm.eval(&format!(
            "local out = {{}} for _, e in ipairs(list_dir('{path}') or {{}}) do \
               out[#out+1] = e.name .. ':' .. tostring(e.root) end \
             table.sort(out) return table.concat(out, ',')"
        ))
        .unwrap()
    };

    // Merged, deduplicated, game winning — the layer that would be required is
    // the layer that is reported.
    assert_eq!(
        names(&mut vm, "shared"),
        "both.lua:game,from_game.lua:game,from_mudlib.lua:mudlib"
    );
    assert_eq!(names(&mut vm, "game:shared"), "both.lua:game,from_game.lua:game");
    assert_eq!(
        names(&mut vm, "mudlib:shared"),
        "both.lua:mudlib,from_mudlib.lua:mudlib"
    );
}

/// The efuns *return* failure; they do not raise it.
///
/// `pcall(write_file, ...)` yields `ok = true, err = false`, so every guard
/// written that way was dead — `codegen_d` reported success for refused writes
/// for as long as it existed. The second return value is what makes checking
/// possible without changing the documented boolean contract.
#[test]
fn a_failed_write_returns_a_reason_rather_than_raising() {
    let (mut vm, _mudlib, _game) = RealVm::boot_two_roots(shipped_permissions());

    // The shape that was broken, spelled out.
    assert_eq!(
        vm.eval(
            "local ok, err = pcall(write_file, 'game:areas/x.lua', 'x') \
             return tostring(ok) .. '|' .. tostring(err)"
        )
        .unwrap(),
        "true|false",
        "pcall reports the CALL succeeded; the refusal is in the return value"
    );

    // The shape that works.
    let out = vm
        .eval(
            "local ok, err = write_file('game:areas/x.lua', 'x') \
             return tostring(ok) .. '|' .. type(err)",
        )
        .unwrap();
    assert_eq!(out, "false|string");
}
