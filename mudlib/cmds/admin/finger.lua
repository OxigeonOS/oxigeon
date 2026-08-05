-- mudlib/cmds/finger.lua — Who is this, really.
--
-- The one caller `get_account` has. It was registered in Rust and never called
-- from Lua, which meant the account behind a character — when it was created,
-- whether it is the superuser — was invisible to staff who needed it and
-- visible in the database to anyone with a shell.
--
-- Staff-gated rather than admin-gated: knowing when an account was made is a
-- moderation question, not a server-administration one.

local M = {}
M.name = 'finger'
M.aliases = { 'whois' }
M.category = 'admin'
M.summary = 'Show the account behind a character.'
M.usage = { "finger <player>" }
M.permission = 'cmd.finger'

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args_str or args_str == "" then
        player:send("{cyan}Finger whom?{/}")
        return
    end
    if type(get_account) ~= "function" then
        player:send("{red}Account lookup is not available.{/}")
        return
    end

    local want = args_str:lower()
    local target, sid
    for _, s in ipairs(all_sessions()) do
        local session = get_session(s)
        if session and session.state == "playing" and session.character_id then
            local p = DAEMON.character and DAEMON.character.get(session.character_id)
            if p and p.name and p.name:lower() == want then
                target, sid = p, s
                break
            end
        end
    end

    if not target then
        player:send("{red}" .. args_str .. " is not online.{/}")
        return
    end

    local ok, account = pcall(get_account, target.account_id)
    if not ok or not account then
        player:send("{red}No account record for " .. target.name .. ".{/}")
        return
    end

    local lines = {
        "{cyan}" .. target.name .. "{/}",
        string.format("  {yellow}%-14s{/} %s", "Account", account.username or "?"),
        string.format("  {yellow}%-14s{/} %s", "Account id", tostring(account.id)),
        string.format("  {yellow}%-14s{/} %s", "Created", account.created_at or "?"),
        string.format("  {yellow}%-14s{/} %s", "Character id", tostring(target.char_id)),
    }

    if account.is_admin then
        -- Worth calling out separately: the superuser bypass is an *account*
        -- flag, not a role, and it cannot be granted or revoked with `role`.
        lines[#lines + 1] = "  {red}Superuser — bypasses every permission check.{/}"
    end

    if type(get_roles) == "function" then
        local rok, roles = pcall(get_roles, target.char_id)
        local held = {}
        for _, r in ipairs((rok and roles) or {}) do
            held[#held + 1] = type(r) == "table" and r.name or tostring(r)
        end
        table.sort(held)
        lines[#lines + 1] = string.format("  {yellow}%-14s{/} %s", "Roles",
            #held > 0 and table.concat(held, ", ") or "(none)")
    end

    local session = get_session(sid)
    if session then
        lines[#lines + 1] = string.format("  {yellow}%-14s{/} %s", "Address",
            tostring(session.address or "?"))
        if session.terminal_type then
            lines[#lines + 1] = string.format("  {yellow}%-14s{/} %s  (%s cols)",
                "Client", session.terminal_type,
                tostring(session.window_width or "?"))
        end
    end

    -- Looking someone up is itself a moderation action, and the audit trail is
    -- for "who did this" rather than "what went wrong".
    if DAEMON and DAEMON.audit then
        pcall(DAEMON.audit.log, "cmd.finger", true, "looked up " .. target.name)
    end

    player:send_lines(lines)
end

return M
