//! Chunk-name <-> debug-client path mapping.
//!
//! A breakpoint only resolves if the key derived from a Lua chunk name matches
//! the key derived from the path VS Code sends. These tests pin both directions,
//! including one that checks the mapping against a chunk name LuaJIT *actually*
//! produced rather than one we constructed ourselves.

use mlua::prelude::*;
use mlua::{HookTriggers, VmState};
use oxigeon::core::scripting::debugger::paths;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A real file under a real directory.
///
/// `mudlib/` will not do, and not only because it is gitignored: a creator may
/// have it as a junction or symlink into their own repository, and
/// `abs_lua_path` canonicalizes — so the chunk name comes back rooted at the
/// link *target* while the path handed in is rooted at the link, and
/// `client_path_and_chunk_name_agree` fails on a difference that is nothing to
/// do with the code it is testing.
const REAL_LUA_FILE: &str = "tests/fixture/mudlib/cmds/who.lua";

#[test]
fn chunk_name_is_absolute_forward_slashed_and_at_prefixed() {
    let p = project_root().join(REAL_LUA_FILE);
    let name = paths::chunk_name(&p);

    assert!(name.starts_with('@'), "chunk name must be @-prefixed: {name}");
    assert!(!name.contains('\\'), "chunk name must use forward slashes: {name}");
    assert!(!name.contains("//?/"), "verbatim prefix must be stripped: {name}");
    assert!(name.ends_with(REAL_LUA_FILE), "unexpected chunk name: {name}");
}

#[test]
fn client_path_and_chunk_name_agree() {
    let p = project_root().join(REAL_LUA_FILE);
    let from_chunk = paths::chunk_key(&paths::chunk_name(&p)).expect("file chunk");

    // What a debug client sends: native separators, and on Windows often a
    // lowercased drive letter.
    let client = p.to_string_lossy().to_string();
    assert_eq!(from_chunk, paths::normalize(&client));

    if cfg!(windows) {
        let lower_drive = client
            .char_indices()
            .map(|(i, c)| if i == 0 { c.to_ascii_lowercase() } else { c })
            .collect::<String>();
        assert_eq!(
            from_chunk,
            paths::normalize(&lower_drive),
            "drive-letter case must not break breakpoint matching"
        );
    }
}

/// LuaJIT's `require` substitutes `?` into `package.path` using the platform
/// separator, so on Windows a chunk loaded from a forward-slashed template comes
/// back mixed: `C:/Code/oxigeon/mudlib/cmds\who.lua`. Matching tolerates it, but
/// a path handed back to the editor must not.
#[test]
fn display_path_unifies_separators_but_keeps_case() {
    let mixed = r"@C:/Code/oxigeon/mudlib/cmds\who.lua";
    let shown = paths::display_path(mixed).expect("file chunk");

    assert_eq!(shown, "C:/Code/oxigeon/mudlib/cmds/who.lua");
    assert!(!shown.contains('\\'), "client paths must not be mixed: {shown}");
    assert!(shown.starts_with("C:"), "case must survive, unlike the match key");

    // ...and it still refers to the same file as far as matching is concerned.
    assert_eq!(paths::normalize(&shown), paths::chunk_key(mixed).unwrap());
    assert!(paths::display_path("init.lua").is_none(), "not a file chunk");
}

#[test]
fn non_file_chunks_have_no_key() {
    // These are what the three pre-M1 `set_name` sites used to produce, and what
    // `load`ed strings still produce. A breakpoint can never apply to them.
    assert!(paths::chunk_key("init.lua").is_none());
    assert!(paths::chunk_key("game/init.lua").is_none());
    assert!(paths::chunk_key("=(load)").is_none());
}

/// The load-bearing test: run a `require` through a real VM configured exactly
/// like `ScriptEngine::start`, capture the chunk name LuaJIT reports to the hook,
/// and assert it maps to the same key as the file on disk.
#[test]
fn required_module_chunk_name_matches_the_file_on_disk() {
    let root = project_root();
    let lua = Lua::new();

    let mudlib = paths::abs_lua_path(&root.join("tests/fixture/mudlib"));
    let game = paths::abs_lua_path(&root.join("tests/fixture/game"));
    lua.load(format!(
        "package.path = \"{game}/?.lua;{game}/?/init.lua;{mudlib}/?.lua;{mudlib}/?/init.lua;\" .. package.path"
    ))
    .exec()
    .unwrap();

    let seen: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    lua.set_hook(HookTriggers::EVERY_LINE, move |_lua, debug| {
        if let Some(src) = debug.source().source {
            let src = src.to_string();
            if !sink.borrow().contains(&src) {
                sink.borrow_mut().push(src);
            }
        }
        Ok(VmState::Continue)
    })
    .expect("the hook must install, or this test asserts nothing");

    // `lib.strings` is pure Lua with no efun dependencies, so it loads standalone.
    lua.load("require('lib.strings')").exec().unwrap();
    lua.remove_hook();

    let expected = paths::chunk_key(&paths::chunk_name(
        &root.join("tests/fixture/mudlib/lib/strings.lua"),
    ))
    .expect("file chunk");
    let observed: Vec<String> = seen.borrow().iter().filter_map(|s| paths::chunk_key(s)).collect();

    assert!(
        observed.contains(&expected),
        "chunk name LuaJIT reported for a required module does not map to the file on disk.\n\
         expected key: {expected}\n\
         observed keys: {observed:#?}"
    );
}
