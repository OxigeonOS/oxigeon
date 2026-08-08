-- game/setup_roles.lua — The roles this game has, and what they may do.
--
-- The entire RBAC management family — `create_role`, `grant_permission`,
-- `assign_role`, `list_roles`, `get_permissions` — was registered in Rust and
-- had **no caller anywhere**. Only `has_permission` was ever used, which meant
-- roles had to be provisioned out of band: the database had a permission system
-- and nothing in the game could put anything into it.
--
-- This is what puts something into it. Run from `game/init.lua` on every boot,
-- and idempotent — which is what makes "declare the roles in a file" work at
-- all, the alternative being a migration nobody remembers to run.
--
-- Idempotent by *arrangement* rather than by construction, and the distinction
-- cost four warnings on every boot: `grant_permission` on a grant that already
-- exists really is a no-op, but `create_role` on a role that already exists is
-- an error the driver logs before returning. `M.apply` asks `list_roles` what
-- is there and creates only the rest. See the note on it.
--
-- ─── Why the game layer ──────────────────────────────────────────────────────
--
-- Which roles exist is a policy decision. A game with one staff tier and a game
-- with seven need different files, not a configuration option, and the driver
-- has no business having an opinion. What the *driver* provides is the
-- machinery: roles, permissions, the session cache and the superuser bypass.

local M = {}

-- ─── The grants have to spell the same strings the code requires ────────────
--
-- They did not. This file granted `cmd.olc`, `cmd.verify` and `efun.write_file`
-- while the code required `olc`, `efun.verify` and `efun.file.write` — not one
-- of the builder role's eight grants matched anything it was meant to unlock.
-- The role existed, the database held it, `role list` printed it, and it did
-- nothing at all. The only account that could build was account 1, through the
-- `is_admin` superuser bypass, which is why nobody noticed.
--
-- `tests/demo_world/` now asserts that every permission a command names is
-- granted by some role, which is the check that would have caught it.
--
-- The scheme is in `config/permissions.toml`. In short: `cmd.<verb>` for a
-- command, `efun.<name>` for an efun, `dir.<op>.<root>.<top>` for a directory,
-- and a bare `<thing>.<capability>` for anything that is none of those.

--- role -> permissions. Ordered as a list of pairs rather than a map, so the
--- order roles are created in is stable and a diff of this file reads as a
--- diff of the policy.
M.ROLES = {
    {
        name = "player",
        summary = "Everyone. Exists so there is something to take away.",
        permissions = {},
    },

    {
        name = "builder",
        summary = "May write area files and use the online builder.",
        permissions = {
            -- The rule in `permissions.toml` that was commented out until this
            -- existed to be granted. Without it `/areas` was world-writable
            -- and the builder role was a label rather than a boundary.
            "dir.write.game.areas",

            -- Not `dir.write.game.prototypes`. A prototype holds functions and
            -- one edit reaches every area that names it, so it is a code change
            -- wearing content's clothes — deliberately outside what the builder
            -- role carries, the same way `/game/lib` is.

            -- Calling the efun at all. Distinct from where it may write:
            -- `[directories]` answers that, and both have to pass.
            "efun.write_file",
            "efun.append_file",
            "efun.delete_file",
            "efun.verify_file",

            -- The builder's toolchain.
            "cmd.olc",
            "cmd.olc.areas",   -- creating a new area, not just editing one
            "cmd.dig",
            "cmd.verify",
            "cmd.reload",
            "cmd.objdump",     -- you cannot edit a field you cannot see

            -- Reading around the tree they build in.
            "cmd.ls",
            "cmd.cd",
            "cmd.pwd",
            "cmd.cat",
        },
    },

    {
        name = "staff",
        summary = "Moderation: watching, warning and taking things down.",
        permissions = {
            "cmd.snoop",
            "cmd.alert",
            "cmd.announce",
            "cmd.journal",
            "cmd.audit",
            "cmd.awho",
            "cmd.finger",

            -- Raising an alert and hearing one are different powers. They used
            -- to be one string, so the only way to be told about an incident
            -- was to be able to page everyone about it.
            "alert.receive",

            -- The efuns behind `journal` and `audit`. The command gate stops
            -- the verb; these stop mudlib code reaching past it.
            "efun.journal_read",
            "efun.audit_read",
            "efun.broadcast_to_perm",

            "board.moderate",
            "channel.staff",
        },
    },

    {
        name = "admin",
        summary = "Everything a role can carry. Not the superuser bypass, "
               .. "which is an account flag and cannot be granted.",
        permissions = {
            -- `has_permission` is exact-match with no wildcards, so a blanket
            -- has to be spelled out. Listing every verb is the honest form:
            -- what this role can do is readable here rather than inferred from
            -- a prefix rule that lives somewhere else.
            "cmd.goto",
            "cmd.spawn",
            "cmd.force",
            "cmd.teleport",
            "cmd.role",
            "cmd.trace",
            "cmd.mudstatus",
            "cmd.affect",
            "cmd.areas",
            "cmd.events",
            "cmd.objdump",
            "cmd.stat",
            "cmd.tasks",
            "cmd.traits",
            "efun.db.clear",
            "efun.trace",
            "efun.disconnect",
            "efun.broadcast",
        },
    },
}

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

--- Which roles the database already has, as a set of names.
---
--- Nil — rather than an empty table — when the question cannot be asked, so the
--- caller can tell "no roles exist" from "no way to find out" and fall back
--- accordingly.
--- @return table|nil  { [name] = true }
local function existing_roles()
    if type(list_roles) ~= "function" then return nil end

    local ok, roles = pcall(list_roles)
    if not ok or type(roles) ~= "table" then return nil end

    local set = {}
    for _, entry in ipairs(roles) do
        -- `list_roles` answers with RoleInfo records; tolerate a bare string
        -- in case a driver ever simplifies it.
        local name = type(entry) == "table" and entry.name or entry
        if type(name) == "string" then set[name] = true end
    end
    return set
end

--- Create every role and grant every permission.
---
--- Idempotent, and it has to be: this runs on every boot, and a version that
--- only worked on an empty database would be a version nobody could trust
--- after the first day.
---
--- ─── Ask first; do not create and swallow ───────────────────────────────────
---
--- Creating a role that exists is **not** a no-op, whatever this file used to
--- claim. The driver attempts the insert, hits `UNIQUE constraint failed:
--- roles.name`, and logs a warning of its own *before* returning — so the
--- `pcall` here caught the failure but could do nothing about four alarming
--- lines already in the log. Every server with a database older than its first
--- boot greeted its owner with them.
---
--- `list_roles` is the fix: check what exists, create only what does not. The
--- fallback for a driver without it is the old behaviour, because a noisy boot
--- is better than no roles.
--- @return number roles, number grants
function M.apply()
    if type(create_role) ~= "function" then
        log("warn", "SETUP_ROLES: the RBAC efuns are not available")
        return 0, 0
    end

    local present = existing_roles()

    local roles, grants = 0, 0
    for _, spec in ipairs(M.ROLES) do
        if present and present[spec.name] then
            -- Already there. Its grants are still reconciled below, because a
            -- role can outlive the permissions this file says it should carry.
            roles = roles + 1
        else
            local ok = pcall(create_role, spec.name)
            if not ok then
                log_error("SETUP_ROLES: could not create role '" .. spec.name .. "'")
            else
                roles = roles + 1
            end
        end

        for _, perm in ipairs(spec.permissions) do
            local gok, granted = pcall(grant_permission, spec.name, perm)
            if not gok then
                log_error("SETUP_ROLES: could not grant '" .. perm
                    .. "' to '" .. spec.name .. "'")
            elseif granted then
                grants = grants + 1
            end
        end
    end

    log("info", "SETUP_ROLES: " .. roles .. " role(s), " .. grants .. " grant(s)")
    return roles, grants
end

--- What a role carries, for the `role` command and for a test.
--- @param name string
--- @return table  array of permission strings
function M.permissions_of(name)
    if type(get_permissions) ~= "function" then return {} end
    local ok, perms = pcall(get_permissions, name)
    return (ok and type(perms) == "table") and perms or {}
end

--- The declared summary, so `role list` reads as policy rather than as a table
--- of identifiers.
--- @param name string
--- @return string|nil
function M.summary_of(name)
    for _, spec in ipairs(M.ROLES) do
        if spec.name == name then return spec.summary end
    end
    return nil
end

return M
