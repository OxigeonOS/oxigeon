-- mudlib/cmds/announce.lua — Server-wide announcement to ALL players
-- Sends a formatted message to every connected session regardless of permissions.
-- Different from "alert" (staff-only). This is for public server announcements.

local M = {}

M.name       = "announce"
M.aliases    = {}
M.category   = "admin"
M.summary    = "Send a server-wide announcement to all players. Usage: announce <message>"
M.permission = "daemon.announce"

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args_str or args_str:match("^%s*$") then
        local lines = {}
        table.insert(lines, "Usage: announce <message>")
        table.insert(lines, "Sends a message to every connected player.")
        player:send(table.concat(lines, "\r\n"))
        return
    end

    -- Get the sender's character name for display
    local sender_session = get_session(session_id)
    local sender_name = "System"
    if sender_session and sender_session.character_id then
        local char = get_character(sender_session.character_id)
        if char then sender_name = char.name end
    end

    local msg = string.format("{cyan}[ANNOUNCEMENT from %s]{/} %s",
        sender_name, args_str)

    -- Use the broadcast efun to hit all sessions
    if type(broadcast) == "function" then
        broadcast(msg .. "\r\n")
    else
        -- Fallback: iterate manually
        if type(all_sessions) == "function" then
            for _, sid in ipairs(all_sessions()) do
                send(sid, msg .. "\r\n")
            end
        end
    end

    -- Journal the announcement
    if DAEMON and DAEMON.journal then
        DAEMON.journal.info(string.format("[announce] %s: %s", sender_name, args_str))
    end

    -- Audit the successful announcement
    if DAEMON and DAEMON.audit then
        DAEMON.audit.log("cmd.announce", true, sender_name .. ": " .. args_str:sub(1, 80))
    end

    player:send("{green}Announcement sent.{/}")
end

return M
