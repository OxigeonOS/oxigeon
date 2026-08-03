//! Does the running VM enforce what the sandbox docs promise?
//!
//! `docs/src/lua-api/sandboxing.md` says `io.*`, `os.execute`, `os.exit` and
//! friends are removed. `tests/sandbox.rs` checks that `apply_sandbox` removes
//! them; this checks that the server *calls it* — every probe below runs inside
//! the VM `ScriptEngine` builds, reached the same way a room file would reach
//! it.
//!
//! This file was originally written as a tripwire that failed on a green suite:
//! `io.open` really could read `Cargo.toml` from mudlib code. The assertions
//! are now inverted, which is what the fix was for.

mod common;

use common::RealVm;

/// The list from `docs/src/lua-api/sandboxing.md`, verbatim.
#[test]
fn the_documented_removals_are_actually_removed() {
    let mut vm = RealVm::boot();

    for expr in [
        "io",
        "io and io.open",
        "io and io.popen",
        "os.execute",
        "os.exit",
        "os.getenv",
        "os.remove",
        "os.rename",
        "os.tmpname",
        "debug",
        "loadfile",
        "dofile",
        "package.loadlib",
    ] {
        assert!(
            !vm.reaches(expr),
            "`{expr}` is documented as removed but is reachable from mudlib code"
        );
    }
}

/// The specific escape that motivated all of this: raw `io` walked straight
/// around the jail `resolve_jailed_path` enforces for the file efuns.
#[test]
fn io_cannot_read_a_file_outside_the_mudlib_jail() {
    let mut vm = RealVm::boot();
    let probe = vm.eval(
        "local f = io.open('Cargo.toml', 'r'); \
         if not f then return 'no handle' end; \
         local l = f:read('*l'); f:close(); return l",
    );
    assert!(
        probe.is_err(),
        "mudlib code reached a file outside the jail: {probe:?}"
    );
    assert!(
        probe.err().contains("io"),
        "the failure should be `io` being nil, not something incidental"
    );
}

/// `read_file` is the supported replacement and stays jailed.
#[test]
fn the_file_efuns_still_work_and_still_refuse_to_escape() {
    let mut vm = RealVm::boot();

    assert_eq!(
        vm.eval("write_file('probe.txt', 'hello'); return read_file('probe.txt')")
            .unwrap(),
        "hello"
    );
    assert_eq!(
        vm.eval("return tostring(read_file('../../Cargo.toml'))").unwrap(),
        "nil",
        "read_file must not resolve outside the mudlib root"
    );
}

/// Stripping `os` must not have taken the clock with it — the mudlib formats
/// dates and stamps records with these.
#[test]
fn the_clock_functions_survive() {
    let mut vm = RealVm::boot();
    assert_eq!(vm.eval("return type(os.time())").unwrap(), "number");
    assert_eq!(vm.eval("return type(os.clock())").unwrap(), "number");
    assert_eq!(vm.eval("return os.date('%Y'):len()").unwrap(), "4");
    assert_eq!(vm.eval("return type(os_time())").unwrap(), "number");
}

/// `require` is how the entire mudlib is assembled. Removing the native module
/// loaders must not have broken the Lua one.
#[test]
fn require_still_loads_lua_modules() {
    let mut vm = RealVm::boot();
    assert_eq!(
        vm.eval("return type(require('init'))").unwrap(),
        "boolean",
        "require should still find and run a module on package.path"
    );
}

/// Pre-compiled bytecode is not validated by LuaJIT and is a known route to
/// memory corruption, so only text may be compiled.
#[test]
fn binary_bytecode_will_not_load() {
    let mut vm = RealVm::boot();
    let out = vm
        .eval(
            "local bc = string.dump(function() return 1 end); \
             local f, err = load(bc); \
             return tostring(f) .. '|' .. tostring(err)",
        )
        .unwrap();
    assert!(
        out.starts_with("nil|"),
        "binary bytecode should not compile, got {out:?}"
    );
    assert!(out.contains("binary bytecode"), "got {out:?}");
}
