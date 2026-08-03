-- game/setup_roles.lua — The roles this game has, and what they may do.
--
-- The entire RBAC management family — `create_role`, `grant_permission`,
-- `assign_role`, `list_roles`, `get_permissions` — was registered in Rust and
-- had **no caller anywhere**. Only `has_permission` was ever used, which meant
-- roles had to be provisioned out of band: the database had a permission system
-- and nothing in the game could put anything into it.
--
-- This is what puts something into it. Run from `game/init.lua` on every boot,
-- and idempotent by construction: `create_role` on a role that exists is a
-- no-op, and so is `grant_permission` on a grant that exists. That is what
-- makes "declare the roles in a file" work at all — the alternative is a
-- migration nobody remembers to run.
--
-- ─── Why the game layer ──────────────────────────────────────────────────────
--
-- Which roles exist is a policy decision. A game with one staff tier and a game
-- with seven need different files, not a configuration option, and the driver
-- has no business having an opinion. What the *driver* provides is the
-- machinery: roles, permissions, the session cache and the superuser bypass.

local M = {}

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
            -- existed to be granted. Without it `/areas` was world-writable and
            -- the builder role was a label rather than a boundary.
            "dir.write.areas",
            "efun.write_file",
            "efun.append_file",
            "efun.delete_file",
            "cmd.olc",
            "cmd.dig",
            "cmd.verify",
            "cmd.reload",
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
            "board.moderate",
            "channel.staff",
        },
    },

    {
        name = "admin",
        summary = "Everything a role can carry. Not the superuser bypass, "
               .. "which is an account flag and cannot be granted.",
        permissions = {
            "admin",
            "cmd.goto",
            "cmd.spawn",
            "cmd.force",
            "cmd.teleport",
            "cmd.role",
            "cmd.trace",
            "cmd.mudstatus",
            "efun.db.clear",
        },
    },
}

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

--- Create every role and grant every permission.
---
--- Idempotent, and it has to be: this runs on every boot, and a version that
--- only worked on an empty database would be a version nobody could trust
--- after the first day.
--- @return number roles, number grants
function M.apply()
    if type(create_role) ~= "function" then
        log("warn", "SETUP_ROLES: the RBAC efuns are not available")
        return 0, 0
    end

    local roles, grants = 0, 0
    for _, spec in ipairs(M.ROLES) do
        -- `create_role` returns nil for a role that already exists, which is
        -- the answer rather than a failure — there is nothing to do.
        local ok = pcall(create_role, spec.name)
        if not ok then
            log_error("SETUP_ROLES: could not create role '" .. spec.name .. "'")
        else
            roles = roles + 1
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
