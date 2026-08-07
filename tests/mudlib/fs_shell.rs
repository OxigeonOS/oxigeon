//! `ls`, `cd`, `pwd` and `cat` over the two-root jail.
//!
//! Booted with the **repository's own** `permissions.toml`, not the harness
//! default. The default has no directory rules at all, so a test against it
//! would pass whether the shell respected them or not — which is the failure
//! `tests/staff.rs` exists to prevent and applies just as much here: a file
//! browser that shows you what you may not read is a file browser that leaks.

use crate::common::RealVm;

fn shipped_permissions() -> oxigeon::config::PermissionConfig {
    oxigeon::config::PermissionConfig::load_from_file(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("config/permissions.toml")
            .as_path(),
    )
}

/// The virtual root lists the two mount points and nothing else.
///
/// It is answered from `fs_d.ROOTS`, not from the filesystem: there is no
/// directory holding the two roots, and inventing one would let the shell show
/// a place the efuns cannot reach.
#[test]
fn the_virtual_root_lists_exactly_the_two_mount_points() {
    let mut vm = RealVm::boot_with_fixture_world(0);

    let out = vm.command("ls /");
    assert!(out.contains("game/"), "{out}");
    assert!(out.contains("mudlib/"), "{out}");
    assert!(out.contains("0 files, 2 directories"), "{out}");
}

/// `cd` moves, `pwd` reports, and relative paths resolve against the cwd.
#[test]
fn cd_and_pwd_agree_about_where_you_are() {
    let mut vm = RealVm::boot_with_fixture_world(0);

    assert_eq!(vm.command("pwd").trim(), "/");

    vm.command("cd /mudlib");
    assert_eq!(vm.command("pwd").trim(), "/mudlib");

    // Relative.
    vm.command("cd cmds");
    assert_eq!(vm.command("pwd").trim(), "/mudlib/cmds");

    vm.command("cd admin");
    assert_eq!(vm.command("pwd").trim(), "/mudlib/cmds/admin");

    vm.command("cd ..");
    assert_eq!(vm.command("pwd").trim(), "/mudlib/cmds");

    // `-` is the previous directory, and everybody tries it.
    vm.command("cd -");
    assert_eq!(vm.command("pwd").trim(), "/mudlib/cmds/admin");
}

/// `..` above a mount point lands at the virtual root, not outside the tree.
///
/// The same question `tests/list_dir_jail.rs` asks of the efun, asked of the
/// interface: a shell that lets you walk out of the jail lexically is a shell
/// that will eventually be asked to read what it walked to.
#[test]
fn climbing_out_of_a_mount_point_lands_at_the_root() {
    let mut vm = RealVm::boot_with_fixture_world(0);

    vm.command("cd /mudlib/cmds/admin");
    vm.command("cd ../../../../../..");
    assert_eq!(vm.command("pwd").trim(), "/");

    // …and from the root, `..` stays put rather than erroring or escaping.
    vm.command("cd ..");
    assert_eq!(vm.command("pwd").trim(), "/");
}

/// A path whose first segment is not a mount point is refused, and the message
/// says what the roots are.
#[test]
fn a_path_that_names_no_root_is_refused_with_the_list() {
    let mut vm = RealVm::boot_with_fixture_world(0);

    let out = vm.command("cd /areas");
    assert!(out.contains("not a root"), "{out}");
    assert!(out.contains("game"), "the message should list the roots: {out}");
    assert_eq!(vm.command("pwd").trim(), "/", "a refused cd should not move you");
}

/// A mistyped `cd` fails where it was typed.
///
/// Accepting it would make the *next* `ls` mysteriously empty, which sends you
/// looking for the wrong bug.
#[test]
fn cd_into_a_missing_directory_is_refused() {
    let mut vm = RealVm::boot_with_fixture_world(0);

    let out = vm.command("cd /mudlib/no_such_directory");
    assert!(out.contains("no such directory"), "{out}");
    assert_eq!(vm.command("pwd").trim(), "/");
}

/// The two layers are separate places, so a file in each is two entries rather
/// than one.
///
/// This is the whole reason the shell does not use `list_dir`'s merged view.
/// Merged, `game/cmds/verify.lua` shadowing `mudlib/cmds/admin/verify.lua` shows
/// as one entry — so you edit the copy that is not loaded, and nothing happens.
#[test]
fn the_two_layers_are_listed_separately() {
    let mut vm = RealVm::boot_with_fixture_world(0);

    let mudlib = vm.command("ls /mudlib");
    assert!(mudlib.contains("daemons/"), "{mudlib}");
    assert!(mudlib.contains("cmds/"), "{mudlib}");

    let game = vm.command("ls /game");
    assert!(game.contains("cmds/"), "{game}");
    // The system layer's directories are not in the content layer's listing.
    assert!(
        !game.contains("components/"),
        "the game listing is showing mudlib entries — this is the merged view: {game}"
    );
}

/// `~` is the area you are building, and `/` when you are not.
///
/// Against a fixture world with a writable game root: entering an area now means
/// entering an *OLC-managed* one, and the shipped areas deliberately are not —
/// a regeneration would eat thornhollow's inline room actions.
#[test]
fn tilde_follows_the_build_session() {
    let mut vm = RealVm::boot_with_fixture_world(0);
    vm.command("pagesize 0");

    vm.command("cd ~");
    assert_eq!(vm.command("pwd").trim(), "/", "not building: ~ is the root");

    vm.command("olc new area crypt The Sunken Crypt");
    vm.command("cd ~");
    assert_eq!(vm.command("pwd").trim(), "/game/areas/crypt");

    // And the working directory survives leaving OLC, which is why `cd` state
    // lives on `fs_d` rather than on `olc_d`.
    vm.command("olc done");
    assert_eq!(vm.command("pwd").trim(), "/game/areas/crypt");
}

/// A directory you may not read is named, not vanished, and the footer says
/// which permission you are missing.
///
/// Omitting it silently makes `ls` look broken; showing its contents would be
/// the leak. The *name* of a directory is not a secret — it is in the
/// repository — and naming the permission turns "it does not work" into a
/// request somebody can grant.
///
/// Gated against `/mudlib/lib` with a config built here rather than against the
/// shipped `/mudlib/admin` rule, because `mudlib/admin/` does not exist: a test
/// that hides a directory which is not there proves nothing.
#[test]
fn an_unreadable_directory_is_counted_and_the_permission_named() {
    use oxigeon::config::permissions_config::{DirPerms, PermissionConfig};

    let mut directories = std::collections::HashMap::new();
    directories.insert(
        "/mudlib/lib".to_string(),
        DirPerms { read: Some("dir.read.mudlib.lib".to_string()), write: None },
    );
    let permissions = PermissionConfig { directories, ..Default::default() };

    let mut vm = RealVm::boot_fixture_with_probe_opts(crate::common::TestCtx {
        permissions,
        ..Default::default()
    });

    // The probe session is not playing, so it holds nothing and is not the
    // superuser — exactly the case the rule is for.
    let listed = vm
        .eval(
            "local out = {} \
             for _, e in ipairs(DAEMON.fs.list('/mudlib') or {}) do \
               out[#out+1] = e.name end \
             table.sort(out) return table.concat(out, ',')",
        )
        .unwrap();
    assert!(
        listed.contains("lib"),
        "the directory should still be named: {listed}"
    );

    let missing = vm
        .eval(&format!(
            "return tostring(DAEMON.fs.missing_permission('{}', '/mudlib/lib', 'read'))",
            vm.session_id()
        ))
        .unwrap();
    assert_eq!(
        missing, "dir.read.mudlib.lib",
        "the shell should know which permission is missing"
    );

    // …and reading its contents really is refused, so the report above is the
    // rule doing its job rather than a label.
    assert_eq!(
        vm.eval("return tostring(list_dir('mudlib:lib'))").unwrap(),
        "nil"
    );

    // A neighbour with no rule is unaffected, so the refusal is the rule and not
    // a blanket.
    assert_eq!(
        vm.eval(&format!(
            "return tostring(DAEMON.fs.missing_permission('{}', '/mudlib/cmds', 'read'))",
            vm.session_id()
        ))
        .unwrap(),
        "nil"
    );
}

/// The shipped rules are the ones the shell will actually meet, so check the
/// wiring against them too — separately, because they name a directory that
/// does not exist and can therefore say nothing about listing.
#[test]
fn the_shipped_rules_reach_the_shell() {
    let mut vm = RealVm::boot_fixture_with_probe_opts(crate::common::TestCtx {
        permissions: shipped_permissions(),
        ..Default::default()
    });

    assert_eq!(
        vm.eval("return tostring(dir_permission('/game/areas', 'write'))").unwrap(),
        "dir.write.game.areas"
    );
    assert_eq!(
        vm.eval("return tostring(dir_permission('/game/areas/crypt', 'write'))").unwrap(),
        "dir.write.game.areas",
        "a rule covers everything beneath it"
    );
    // Reading areas is open; the rule names `write` and says nothing about read.
    assert_eq!(
        vm.eval("return tostring(dir_permission('/game/areas', 'read'))").unwrap(),
        "nil"
    );
    // The virtual path is the key. A jail-relative one names no rule at all,
    // which is what made this efun wrong the first time.
    assert_eq!(
        vm.eval("return tostring(dir_permission('areas', 'write'))").unwrap(),
        "nil"
    );
}

/// `cat` shows a file, and `-n` numbers it.
///
/// `pagesize 0` first: `cat` pages, and the harness waits for the ordinary
/// prompt rather than `--More--`. Turning paging off is what a scripted client
/// would do and is the honest way to read the whole output at once.
#[test]
fn cat_shows_a_file_and_numbers_it_on_request() {
    let mut vm = RealVm::boot_with_fixture_world(0);
    vm.command("pagesize 0");

    let out = vm.command("cat /mudlib/lib/strings.lua");
    assert!(out.contains("function M.trim"), "the body should be shown:\n{out}");
    assert!(out.contains("/mudlib/lib/strings.lua"), "{out}");
    assert!(out.contains("bytes"), "the header should size it: {out}");

    let numbered = vm.command("cat -n /mudlib/lib/strings.lua");
    assert!(
        numbered.contains("   1  "),
        "line numbers should be shown:\n{numbered}"
    );
}

/// A file's own `{colour}` tags are shown, not rendered and not deleted.
///
/// Every command file in the mudlib is full of `{red}` and `{/}`. Rendering them
/// would paint the listing in the colours of the code you were reading;
/// stripping them would silently remove tags from the source you are inspecting.
#[test]
fn cat_shows_colour_tags_as_source() {
    let mut vm = RealVm::boot_with_fixture_world(0);
    vm.command("pagesize 0");

    let out = vm.command("cat /mudlib/cmds/building/dig.lua");
    assert!(
        out.contains("{red}"),
        "a colour tag in the file should appear verbatim:\n{out}"
    );
}

/// `cat` on a directory says so rather than "no such file", which would send
/// you looking for a spelling mistake that is not there.
#[test]
fn cat_on_a_directory_says_so_and_suggests_ls() {
    let mut vm = RealVm::boot_with_fixture_world(0);

    let out = vm.command("cat /mudlib/lib");
    assert!(out.contains("is a directory"), "{out}");
    assert!(out.contains("ls /mudlib/lib"), "it should suggest the fix: {out}");
}

/// The shell never writes. There is no `rm`, `mv`, `mkdir` or whole-file editor,
/// and that is a decision rather than an omission: an in-game `rm` is how areas
/// vanish, and an `edit` would invite hand-edits to the very files OLC
/// regenerates.
#[test]
fn the_shell_has_no_way_to_destroy_anything() {
    let mut vm = RealVm::boot_fixture_with_probe();

    let verbs = vm
        .eval(
            "local out = {} \
             for _, v in ipairs({'rm','mv','cp','mkdir','rmdir','edit','touch','chmod'}) do \
               if require('lib.commands').registry()[v] then out[#out+1] = v end end \
             return table.concat(out, ',')",
        )
        .unwrap();
    assert_eq!(verbs, "", "the file shell has grown a destructive verb: {verbs}");
}
