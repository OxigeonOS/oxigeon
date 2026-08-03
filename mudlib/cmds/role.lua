-- mudlib/cmds/role.lua — Grant and revoke roles, in game, to people who are
-- online.
--
-- The command that makes the whole RBAC family reachable. `create_role`,
-- `assign_role`, `revoke_role`, `get_roles`, `list_roles`, `grant_permission`,
-- `revoke_permission` and `get_permissions` were all registered in Rust and had
-- no caller anywhere; only `has_permission` was used, which meant roles had to
-- be provisioned out of band.
--
-- ─── The thing this is really about ──────────────────────────────────────────
--
-- `has_permission` reads a **per-session cache** seeded at
-- `enter_game_session`. Changing what somebody may do therefore has to say so,
-- or the change reaches nobody who is already logged in. `assign_role` and
-- `revoke_role` push into the cache themselves, and so do `grant_permission`
-- and `revoke_permission` — but `refresh_permissions` is the explicit escape
-- hatch, and this command exposes it, because the first thing anyone does when
-- a permission is not taking effect is look for a way to force it.

local M = {}
M.name = 'role'
M.aliases = { 'roles' }
M.category = 'admin'
M.summary = 'Manage roles and what they may do.'
M.usage = {
    "role list                        every role and its permissions",
    "role who <player>                what they hold",
    "role grant <player> <role>",
    "role revoke <player> <role>",
    "role perms <role>                what one role carries",
    "role allow <role> <permission>",
    "role deny <role> <permission>",
    "role refresh <player>            rebuild their permission cache",
}
M.permission = 'admin'

--- An online character by exact name. Prefix matching on a command that hands
--- out permissions is a bug waiting for two names that share three letters.
local function find_session(name)
    local want = name:lower()
    for _, sid in ipairs(all_sessions()) do
        local s = get_session(sid)
        if s and s.state == "playing" and s.character_id then
            local p = DAEMON.character and DAEMON.character.get(s.character_id)
            if p and p.name and p.name:lower() == want then
                return sid, p
            end
        end
    end
    return nil, nil
end

local function audit(action, ok, detail)
    if DAEMON and DAEMON.audit then
        pcall(DAEMON.audit.log, "role." .. action, ok, detail)
    end
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end
    if type(list_roles) ~= "function" then
        player:send("{red}The role system is not available.{/}")
        return
    end

    local verb = (args[1] or ""):lower()

    -- ─── list ────────────────────────────────────────────────────────────────
    if verb == "" or verb == "list" then
        local ok, roles = pcall(list_roles)
        if not ok or #roles == 0 then
            player:send("{yellow}No roles are defined.{/}")
            return
        end
        local lines = { "{cyan}Roles{/}", "" }
        for _, role in ipairs(roles) do
            local name = type(role) == "table" and role.name or tostring(role)
            local perms = select(2, pcall(get_permissions, name)) or {}
            lines[#lines + 1] = "  {yellow}" .. name .. "{/}  ("
                .. #perms .. " permission(s))"
        end
        lines[#lines + 1] = ""
        lines[#lines + 1] = "See one with {cyan}role perms <role>{/}."
        player:send_lines(lines)
        return
    end

    if verb == "perms" then
        local name = args[2]
        if not name then player:send("{cyan}Which role?{/}") return end
        local ok, perms = pcall(get_permissions, name)
        if not ok or type(perms) ~= "table" then
            player:send("{red}No role called '" .. name .. "'.{/}")
            return
        end
        table.sort(perms)
        local lines = { "{cyan}" .. name .. "{/} — " .. #perms .. " permission(s)", "" }
        for _, perm in ipairs(perms) do lines[#lines + 1] = "  " .. perm end
        if #perms == 0 then lines[#lines + 1] = "  (none)" end
        player:send_lines(lines)
        return
    end

    if verb == "who" then
        local name = args[2]
        if not name then player:send("{cyan}Which player?{/}") return end
        local sid, target = find_session(name)
        if not target then
            player:send("{red}" .. name .. " is not online.{/}")
            return
        end
        local ok, roles = pcall(get_roles, target.char_id)
        local held = {}
        for _, r in ipairs((ok and roles) or {}) do
            held[#held + 1] = type(r) == "table" and r.name or tostring(r)
        end
        table.sort(held)
        player:send("{cyan}" .. target.name .. "{/} holds: "
            .. (#held > 0 and table.concat(held, ", ") or "nothing"))
        return
    end

    -- ─── grant / revoke a role ───────────────────────────────────────────────
    if verb == "grant" or verb == "revoke" then
        local who, role = args[2], args[3]
        if not who or not role then
            player:send("{cyan}Usage: role " .. verb .. " <player> <role>{/}")
            return
        end
        local sid, target = find_session(who)
        if not target then
            player:send("{red}" .. who .. " is not online.{/}")
            return
        end

        local fn = verb == "grant" and assign_role or revoke_role
        local ok, changed = pcall(fn, target.char_id, role)
        if not ok or not changed then
            player:send("{red}Could not " .. verb .. " '" .. role .. "'.{/}")
            audit(verb, false, verb .. " " .. role .. " for " .. target.name)
            return
        end

        -- The efun already pushed the new permissions into every session that
        -- character has. Saying so out loud, because "it will take effect next
        -- time they log in" is what everyone assumes and it is not true here.
        player:send("{green}" .. target.name .. " " ..
            (verb == "grant" and "now holds" or "no longer holds")
            .. " '" .. role .. "'. It takes effect now, not on their next login.{/}")
        if target.send then
            pcall(target.send, target, "{cyan}Your permissions have changed.{/}")
        end
        audit(verb, true, verb .. " " .. role .. " for " .. target.name)
        return
    end

    -- ─── edit a role ─────────────────────────────────────────────────────────
    if verb == "allow" or verb == "deny" then
        local role, perm = args[2], args[3]
        if not role or not perm then
            player:send("{cyan}Usage: role " .. verb .. " <role> <permission>{/}")
            return
        end
        local fn = verb == "allow" and grant_permission or revoke_permission
        local ok, changed = pcall(fn, role, perm)
        if not ok or not changed then
            player:send("{red}Could not " .. verb .. " '" .. perm .. "'.{/}")
            audit(verb, false, verb .. " " .. perm .. " on " .. role)
            return
        end
        -- Editing a role changes what *everyone* holding it may do, and the
        -- efun resyncs every playing session for exactly that reason.
        player:send("{green}'" .. role .. "' " ..
            (verb == "allow" and "now carries" or "no longer carries")
            .. " '" .. perm .. "'. Everyone holding it is updated.{/}")
        audit(verb, true, verb .. " " .. perm .. " on " .. role)
        return
    end

    -- ─── refresh ─────────────────────────────────────────────────────────────
    if verb == "refresh" then
        local who = args[2]
        if not who then player:send("{cyan}Refresh whose permissions?{/}") return end
        local sid, target = find_session(who)
        if not sid then
            player:send("{red}" .. who .. " is not online.{/}")
            return
        end
        local ok, done = pcall(refresh_permissions, sid)
        player:send((ok and done)
            and ("{green}Rebuilt " .. target.name .. "'s permission cache.{/}")
            or "{red}Could not refresh — are they still playing?{/}")
        audit("refresh", ok and done, "refreshed " .. tostring(target.name))
        return
    end

    player:send("{red}Unknown option '" .. verb .. "'.{/}")
    player:send_lines(M.usage)
end

return M
