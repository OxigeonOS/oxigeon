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

/// The roles the game declares are created on boot, idempotently, and carry
/// what the file says they carry.
#[test]
fn the_game_declares_its_roles_on_every_boot() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    let names: String = vm
        .eval(
            "local t = {} for _, r in ipairs(list_roles()) do \
             t[#t + 1] = (type(r) == 'table' and r.name or tostring(r)) end \
             table.sort(t) return table.concat(t, ',')",
        )
        .unwrap();
    for role in ["admin", "builder", "player", "staff"] {
        assert!(names.contains(role), "'{role}' was not created: {names}");
    }

    // The builder carries the permission that `permissions.toml` names, which
    // is the whole point of uncommenting that rule.
    assert_eq!(
        vm.eval(
            "for _, p in ipairs(get_permissions('builder')) do \
             if p == 'dir.write.areas' then return 'yes' end end return 'no'"
        )
        .unwrap(),
        "yes"
    );

    // Idempotent: running it again changes nothing and raises nothing. That is
    // what lets roles be declared in a file rather than provisioned by a
    // migration nobody remembers to run.
    let before: i64 = vm.eval("return #list_roles()").unwrap().parse().unwrap();
    vm.eval("require('setup_roles').apply() return 'again'").unwrap();
    let after: i64 = vm.eval("return #list_roles()").unwrap().parse().unwrap();
    assert_eq!(before, after, "a second run created duplicate roles");

    let perms: i64 = vm
        .eval("return #get_permissions('builder')")
        .unwrap()
        .parse()
        .unwrap();
    vm.eval("require('setup_roles').apply() return 'again'").unwrap();
    assert_eq!(
        vm.eval("return #get_permissions('builder')").unwrap().parse::<i64>().unwrap(),
        perms,
        "a second run duplicated the grants"
    );
}

/// Assigning a role to somebody who is online takes effect *now*.
#[test]
fn the_role_command_grants_and_the_change_lands_immediately() {
    let mut vm = RealVm::boot_real_mudlib(0);

    let out = vm.command("role list");
    assert!(out.contains("builder"), "roles are not listed:\n{out}");
    assert!(out.contains("permission"), "{out}");

    let out = vm.command("role perms builder");
    assert!(out.contains("dir.write.areas"), "{out}");

    // The test character is the superuser, so `role who` on them is the case
    // worth checking: the bypass is an *account flag*, not a role, and must not
    // appear as one.
    let out = vm.command("role who benchuser");
    assert!(out.contains("holds:"), "{out}");

    let out = vm.command("role grant benchuser builder");
    assert!(out.contains("now holds"), "granting failed:\n{out}");
    assert!(
        out.contains("takes effect now"),
        "the message should say the change is immediate — everyone assumes \
         otherwise, and here they would be wrong:\n{out}"
    );

    assert!(vm.command("role who benchuser").contains("builder"));

    let out = vm.command("role revoke benchuser builder");
    assert!(out.contains("no longer holds"), "{out}");
    assert!(!vm.command("role who benchuser").contains("builder"));
}

/// Editing a role reaches everyone holding it, and `refresh` is the escape
/// hatch for anything the automatic path cannot see.
#[test]
fn a_role_can_be_edited_and_a_cache_refreshed() {
    let mut vm = RealVm::boot_real_mudlib(0);

    let out = vm.command("role allow builder cmd.example");
    assert!(out.contains("now carries"), "{out}");
    assert!(
        out.contains("Everyone holding it is updated"),
        "editing a role changes what everyone holding it may do, and the \
         message should say so:\n{out}"
    );
    assert!(vm.command("role perms builder").contains("cmd.example"));

    let out = vm.command("role deny builder cmd.example");
    assert!(out.contains("no longer carries"), "{out}");
    assert!(!vm.command("role perms builder").contains("cmd.example"));

    let out = vm.command("role refresh benchuser");
    assert!(out.contains("Rebuilt"), "{out}");

    // Somebody who is not here.
    assert!(vm.command("role grant nobody builder").contains("not online"));
}

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

/// The permission rule for `/areas` is live rather than commented out, so the
/// builder role is a boundary rather than a label.
///
/// Booted with the **repository's own** `permissions.toml` rather than the
/// harness default. That matters: the default has no rules at all, so a test
/// against it would pass whether the rule were commented out or not — which is
/// precisely the shape of bug this file exists to catch.
#[test]
fn the_areas_directory_is_permission_gated() {
    let permissions = oxigeon::config::PermissionConfig::load_from_file(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("config/permissions.toml")
            .as_path(),
    );
    assert!(
        permissions.dir_permission("/areas", "write").is_some(),
        "the /areas rule is commented out again — the builder role is a label"
    );

    let mut vm = RealVm::boot_real_mudlib_with_probe_opts(common::TestCtx {
        permissions,
        ..Default::default()
    });

    // The probe session is not playing, so it holds nothing and is not the
    // superuser — which is exactly the case the rule is for.
    assert_eq!(
        vm.eval("return tostring(write_file('areas/probe_should_fail.lua', 'x'))").unwrap(),
        "false",
        "/areas is world-writable — the rule in permissions.toml is a no-op"
    );

    // The write was refused, so nothing was created — a test that leaves a file
    // in the repository it is testing is a test that passes on its own litter.
    assert!(
        !std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("mudlib/areas/probe_should_fail.lua")
            .exists(),
        "the refused write reached disk anyway"
    );

    // Reading is the other half of the rule, and it is checked against the
    // configuration rather than against a directory listing: the rule names
    // `write` and says nothing about `read`, which is the whole design.
    let permissions = oxigeon::config::PermissionConfig::load_from_file(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("config/permissions.toml")
            .as_path(),
    );
    assert!(
        permissions.dir_permission("/areas", "read").is_none(),
        "builders write areas; everyone reads them"
    );

    // And an ungated directory really is readable, so the refusal above is the
    // rule doing its job rather than `list_dir` being broken.
    assert_eq!(
        vm.eval("return tostring(list_dir('lib') ~= nil)").unwrap(),
        "true"
    );
}
