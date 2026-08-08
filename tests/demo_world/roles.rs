//! The roles *this game* declares, and the `role` command operating on them.
//!
//! `game/setup_roles.lua` decides which roles exist and what they may do —
//! explicitly a game decision the driver has no view on. So a suite that asserts
//! `builder` exists and carries `dir.write.areas` is asserting content, and
//! belongs here rather than in `tests/staff.rs`.

use crate::common::RealVm;
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
             if p == 'dir.write.game.areas' then return 'yes' end end return 'no'"
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

/// A re-run does not even *attempt* to create a role that exists.
///
/// The distinction the test above cannot see. `create_role` on an existing role
/// is not a no-op: the driver attempts the insert, hits `UNIQUE constraint
/// failed: roles.name`, and logs a warning of its own **before** returning the
/// error. `setup_roles.apply` caught that with `pcall` and the role count stayed
/// right, so every assertion about state passed — while every server with a
/// database older than its first boot greeted its owner with four warnings.
///
/// So this counts calls rather than rows. The fix is `list_roles` first, create
/// only what is missing; the failure it guards against is invisible in the
/// database and visible only in the log.
#[test]
fn a_second_run_does_not_re_create_roles_that_exist() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    // Boot already applied it, so every declared role is present by now.
    let attempts: i64 = vm
        .eval(
            "local calls = 0 \
             local real = create_role \
             create_role = function(...) calls = calls + 1 return real(...) end \
             local ok, err = pcall(function() require('setup_roles').apply() end) \
             create_role = real \
             if not ok then return 'apply failed: ' .. tostring(err) end \
             return tostring(calls)",
        )
        .unwrap()
        .parse()
        .unwrap_or_else(|_| panic!("apply() did not run cleanly under the counting stub"));

    assert_eq!(
        attempts, 0,
        "apply() tried to create {attempts} role(s) that already existed — each \
         one is a `UNIQUE constraint failed` the driver logs before the pcall \
         here can swallow it"
    );

    // And the fallback is still wired: a driver with no `list_roles` must fall
    // back to create-and-swallow rather than skipping role setup entirely.
    let created: i64 = vm
        .eval(
            "local calls = 0 \
             local real_create, real_list = create_role, list_roles \
             create_role = function(...) calls = calls + 1 return real_create(...) end \
             list_roles = nil \
             pcall(function() require('setup_roles').apply() end) \
             create_role, list_roles = real_create, real_list \
             return tostring(calls)",
        )
        .unwrap()
        .parse()
        .unwrap();

    assert_eq!(
        created, 4,
        "without `list_roles` the file must still try to create all four roles"
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
    assert!(out.contains("dir.write.game.areas"), "{out}");

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

/// Every permission a command asks for is granted by some role.
///
/// This is the check that was missing, and its absence is why the builder role
/// did nothing for months: `setup_roles.lua` granted `cmd.olc`, `cmd.verify` and
/// `efun.write_file` while the code required `olc`, `efun.verify` and
/// `efun.file.write`. Every grant was a string nothing would ever ask for. The
/// role existed, `role list` printed it, and the only account that could build
/// was account 1 through the `is_admin` bypass.
///
/// Here rather than in `tests/command_layout.rs` because it reads
/// `game/setup_roles.lua` — which roles exist is a game decision, and a mudlib
/// with no `game/` must not fail for having no opinion about it.
///
/// `tests/command_layout.rs` asserts the *shape* of the strings; this asserts
/// somebody can actually be given them. Both are needed: a uniform naming
/// scheme that nothing grants is as useless as a grant that names nothing.
#[test]
fn every_command_permission_is_granted_by_some_role() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    let ungranted = vm
        .eval(
            "local held = {} \
             for _, r in ipairs(list_roles()) do \
               local name = type(r) == 'table' and r.name or tostring(r) \
               for _, p in ipairs(get_permissions(name)) do held[p] = true end \
             end \
             local out = {} \
             for verb, mod in pairs(require('lib.commands').registry()) do \
               if mod.permission and not held[mod.permission] then \
                 out[#out+1] = verb .. ' wants ' .. mod.permission \
               end \
             end \
             table.sort(out) return table.concat(out, '; ')",
        )
        .unwrap();

    assert_eq!(
        ungranted, "",
        "commands nobody but account 1 can ever run: {ungranted}"
    );
}

