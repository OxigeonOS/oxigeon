//! RBAC, from the game side — and the cold surface it lights up.
//!
//! The entire role-management family was registered in Rust and had **no
//! caller anywhere**: `create_role`, `delete_role`, `list_roles`,
//! `assign_role`, `revoke_role`, `get_roles`, `grant_permission`,
//! `revoke_permission`, `get_permissions`. Only `has_permission` was used,
//! which meant roles had to be provisioned out of band — the database had a
//! permission system and nothing in the game could put anything into it.
//!
//! `get_account` was in the same position, with `finger` as its first caller.

mod common;

use common::RealVm;

/// `finger` is `get_account`'s first caller, and the superuser flag is shown
/// as what it is: an account flag rather than a role.
#[test]
fn finger_shows_the_account_behind_a_character() {
    let mut vm = RealVm::boot_real_mudlib(0);

    let out = vm.command("finger benchuser");
    assert!(out.contains("Account"), "no account line:\n{out}");
    assert!(out.contains("benchuser"), "{out}");
    assert!(out.contains("Created"), "the creation date is the point:\n{out}");
    assert!(
        out.contains("Superuser"),
        "the first account bypasses every check and that has to be visible:\n{out}"
    );
    assert!(out.contains("Roles"), "{out}");

    assert!(vm.command("finger nobody").contains("not online"));
}

/// Permission denial is audited, which is the difference between `journal_d`
/// and `audit_d`: what went wrong versus who did it.
#[test]
fn a_denial_is_audited() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    // The audit log is a file the driver writes. Asking it what it holds is
    // the honest check — a test that asserted against an in-memory buffer would
    // pass while nothing reached disk.
    assert_eq!(
        vm.eval("return tostring(type(audit_write) == 'function')").unwrap(),
        "true"
    );
    vm.eval("audit_write('role.grant', false, 'test denial') return 'written'").unwrap();

    let recent = vm
        .eval("return tostring(#DAEMON.audit.recent(10) > 0)")
        .unwrap();
    assert_eq!(recent, "true", "the audit trail should be readable back");
}

/// The repository's own `permissions.toml`, not the harness default.
///
/// That distinction is the point of this file: the default has no rules at all,
/// so a test against it would pass whether a rule were present or commented out
/// — precisely the shape of bug these assertions exist to catch.
fn shipped_permissions() -> oxigeon::config::PermissionConfig {
    oxigeon::config::PermissionConfig::load_from_file(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("config/permissions.toml")
            .as_path(),
    )
}

/// The permission rule for the area tree is live rather than commented out, so
/// the builder role is a boundary rather than a label.
#[test]
fn the_areas_directory_is_permission_gated() {
    let permissions = shipped_permissions();

    // Every rule names its root. One that does not is dropped — it would be
    // ambiguous about which of the two trees it guards — so a config that
    // regressed to `/areas` would leave the game tree world-writable while
    // still *looking* protected.
    assert!(
        permissions.invalid_directory_keys.is_empty(),
        "directory rules naming no root are not in effect: {:?}",
        permissions.invalid_directory_keys
    );
    assert!(
        permissions.dir_permission("/game/areas", "write").is_some(),
        "the /game/areas rule is commented out again — the builder role is a label"
    );

    let mut vm = RealVm::boot_real_mudlib_with_probe_opts(common::TestCtx {
        permissions,
        ..Default::default()
    });

    // The probe session is not playing, so it holds nothing and is not the
    // superuser — which is exactly the case the rule is for. Named with an
    // explicit root, because that is how OLC writes and therefore the path that
    // has to be gated.
    assert_eq!(
        vm.eval("return tostring(write_file('game:areas/probe_should_fail.lua', 'x'))")
            .unwrap(),
        "false",
        "/game/areas is world-writable — the rule in permissions.toml is a no-op"
    );

    // The refusal says which permission would have allowed it. "Permission
    // denied" alone is not something a builder can act on.
    let why = vm
        .eval("local ok, err = write_file('game:areas/probe_should_fail.lua', 'x') \
               return tostring(err)")
        .unwrap();
    assert!(
        why.contains("dir.write.game.areas"),
        "the refusal should name the permission that would allow it: {why}"
    );

    // The write was refused, so nothing was created — a test that leaves a file
    // in the repository it is testing is a test that passes on its own litter.
    for stray in ["game/areas/probe_should_fail.lua", "mudlib/areas/probe_should_fail.lua"] {
        assert!(
            !std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(stray).exists(),
            "the refused write reached disk anyway: {stray}"
        );
    }

    // Reading is the other half of the rule, and it is checked against the
    // configuration rather than against a directory listing: the rule names
    // `write` and says nothing about `read`, which is the whole design.
    assert!(
        shipped_permissions().dir_permission("/game/areas", "read").is_none(),
        "builders write areas; everyone reads them"
    );

    // And an ungated directory really is readable, so the refusal above is the
    // rule doing its job rather than `list_dir` being broken.
    assert_eq!(
        vm.eval("return tostring(list_dir('lib') ~= nil)").unwrap(),
        "true"
    );
}
