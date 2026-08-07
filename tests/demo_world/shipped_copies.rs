//! `game.example/` and `mudlib.default/` are what a fresh checkout copies from.
//!
//! The repo does not want to dictate a game. `game/` and `mudlib/` are what the
//! server actually loads, and somebody bringing their own world symlinks or
//! copies over them — so the templates beside them have to be **exactly** what
//! the demo world is, or `cp -r game.example game` gives you something nobody
//! has run.
//!
//! This exists because they drifted within a day of being created: ten files
//! changed in `game/` and `mudlib/` and nothing anywhere noticed. Nothing loads
//! the copies, so no test failed and no boot broke — which is precisely the
//! shape of thing that is discovered by a user rather than by a suite.
//!
//! Here rather than in the mudlib bucket because it asserts *shipped content*,
//! and it goes when `game/` goes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every file under `dir`, keyed by its path relative to `dir`.
///
/// Read as **bytes**, so a line ending is a difference. `.gitattributes` says
/// `eol=lf` for every text file here, and a copy that differs only by CRLF is
/// still a copy somebody's editor will keep rewriting.
fn tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = match std::fs::read_dir(&d) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(bytes) = std::fs::read(&path) {
                let rel = path
                    .strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, bytes);
            }
        }
    }
    out
}

fn assert_mirrors(live_name: &str, copy_name: &str) {
    let live = root().join(live_name);
    let copy = root().join(copy_name);

    // A checkout that has not made its working copy yet is not a failure — the
    // whole point is that `game/` may be somebody else's, or absent.
    if !live.is_dir() || !copy.is_dir() {
        return;
    }

    let a = tree(&live);
    let b = tree(&copy);

    let missing: Vec<&String> = a.keys().filter(|k| !b.contains_key(*k)).collect();
    assert!(
        missing.is_empty(),
        "{copy_name}/ is missing files that {live_name}/ has, so copying it \
         gives an incomplete world: {missing:?}"
    );

    let extra: Vec<&String> = b.keys().filter(|k| !a.contains_key(*k)).collect();
    assert!(
        extra.is_empty(),
        "{copy_name}/ has files {live_name}/ does not, so it ships something \
         nothing has run: {extra:?}"
    );

    let differing: Vec<&String> = a
        .iter()
        .filter(|(k, v)| b.get(*k) != Some(*v))
        .map(|(k, _)| k)
        .collect();
    assert!(
        differing.is_empty(),
        "these differ between {live_name}/ and {copy_name}/, so a fresh copy \
         would not be the world this suite tests:\n  {}",
        differing
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

/// `game.example/` is byte-for-byte the game this suite tests.
#[test]
fn the_example_game_is_the_game() {
    assert_mirrors("game", "game.example");
}

/// `mudlib.default/` is byte-for-byte the mudlib this suite tests.
#[test]
fn the_default_mudlib_is_the_mudlib() {
    assert_mirrors("mudlib", "mudlib.default");
}
