//! D1 — `list_dir` must not escape the mudlib root, **through the engine's VM**.
//!
//! There were two `list_dir` efuns. `register_io_efuns` installed the
//! permission-checked, path-jailed one from `efuns_io.rs`; `register_utility_efuns`
//! ran later and overwrote it with a version that joined the caller's path
//! straight onto the mudlib and game roots — no jail, no permission check. So
//! `list_dir("../../..")` escaped, while `file-access.md` and `sandboxing.md`
//! both claimed traversal prevention "for all file efuns".
//!
//! `efuns_io.rs` has unit tests for `resolve_jailed_path` and they were green
//! the whole time, because they test a function production did not reach. This
//! file asks the only question that matters: what can Lua running in this
//! server actually list? That is the rule in `CLAUDE.md`'s testing section, and
//! this is the bug it was written about.

use crate::common::RealVm;

/// The escape itself. `../..` from the mudlib root reaches the repository, and
/// with the shadowing copy in place this returned `Cargo.toml` and `src`.
#[test]
fn a_traversal_out_of_the_mudlib_is_refused() {
    let mut vm = RealVm::boot_fixture_with_probe();

    for attempt in ["..", "../..", "../../..", "cmds/../..", "../mudlib/../.."] {
        let out = vm
            .eval(&format!(
                "local e = list_dir('{attempt}'); return e and #e or 'nil'"
            ))
            .unwrap();
        assert_eq!(
            out, "nil",
            "list_dir({attempt:?}) escaped the jail and listed {out} entries"
        );
    }
}

/// An absolute path is not a relative path, and must not be treated as one.
#[test]
fn an_absolute_path_is_refused() {
    let mut vm = RealVm::boot_fixture_with_probe();

    // Both separators, because the jail normalizes lexically and Windows
    // accepts either.
    for attempt in ["/", "C:/", "C:\\Windows", "//server/share"] {
        let out = vm
            .eval(&format!(
                "local e = list_dir([[{attempt}]]); return e and #e or 'nil'"
            ))
            .unwrap();
        assert_eq!(
            out, "nil",
            "list_dir({attempt:?}) listed {out} entries from outside the mudlib"
        );
    }
}

/// The jail is not a refusal of everything: a legitimate listing still works,
/// or the fix would be indistinguishable from deleting the efun.
#[test]
fn a_legitimate_listing_still_works() {
    let mut vm = RealVm::boot_fixture_with_probe();

    let n: i64 = vm
        .eval("local e = list_dir('cmds'); return e and #e or 0")
        .unwrap()
        .parse()
        .unwrap();
    assert!(n > 20, "expected the real command directory, got {n} entries");

    // The documented contract: entry tables, not bare stems. The shadowing
    // copy returned stems, which is the contract `commands.lua` was written
    // against — so this pins the one the docs describe.
    assert_eq!(
        vm.eval("return type(list_dir('cmds')[1])").unwrap(),
        "table",
        "list_dir should return entry tables, per docs/src/lua-api/file-access.md"
    );
    assert_eq!(
        vm.eval(
            "local hit for _, e in ipairs(list_dir('cmds')) do \
             if e.name == 'look.lua' then hit = e end end \
             return hit and (tostring(hit.is_dir) .. ',' .. tostring(hit.size > 0)) or 'missing'"
        )
        .unwrap(),
        "false,true",
        "an entry should carry name, is_dir and size"
    );
}

/// A directory that does not exist is `nil`, not an empty table — the caller
/// can tell a misconfigured command path from a genuinely empty one.
#[test]
fn a_missing_directory_is_nil_and_an_empty_one_is_a_table() {
    let mut vm = RealVm::boot_fixture_with_probe();

    assert_eq!(
        vm.eval("return tostring(list_dir('no_such_directory_here'))").unwrap(),
        "nil"
    );

    // `lib` exists in the mudlib and is not empty, which is the positive
    // control for the branch above.
    assert_eq!(
        vm.eval("return tostring(list_dir('lib') ~= nil)").unwrap(),
        "true"
    );
}

/// Command discovery — the one real caller — still finds every command after
/// the contract change. If it did not, the game would boot with no verbs.
#[test]
fn command_discovery_still_loads_every_command() {
    let mut vm = RealVm::boot_real_mudlib(0);

    // `help all` generates its listing from the registry, so a discovery
    // failure shows up here as a short list rather than as a crash.
    //
    // `all` and not bare `help`, which lists *categories* now — a category
    // index would keep printing nine headings with every command missing, so
    // this needs the view that names the verbs. And unpaged, or the answer is
    // the first screen of it and the verbs below the fold look undiscovered.
    vm.command("pagesize 0");
    let out = vm.command("help all");
    for verb in ["look", "score", "skills", "traits", "inventory", "say", "quit"] {
        assert!(
            out.contains(verb),
            "'{verb}' was not discovered — command loading is broken:\n{out}"
        );
    }
}
