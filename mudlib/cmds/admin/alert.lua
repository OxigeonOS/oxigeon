-- mudlib/cmds/alert.lua — Send an alert to all staff (privileged) accounts
-- This is for staff-to-staff communication, not player announcements.
--
-- Two permissions, deliberately: `cmd.alert` is who may *send* one, and
-- `alert.receive` is who *gets* one. They were one string, which meant the only
-- way to hear an alert was to be able to raise one — so a moderator who should
-- be told about an incident had to be given the ability to page everyone.
-- `alert.receive` is a capability rather than a command, so it is not `cmd.*`;
-- `board.moderate` and `channel.staff` are the same shape.

local M = {}

M.name       = "alert"
M.aliases    = {}
M.category   = "admin"
M.summary    = "Send an alert to all online staff. Usage: alert <message>"
M.permission = "cmd.alert"

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args_str or args_str:match("^%s*$") then
        local lines = {}
        table.insert(lines, "Usage: alert <message>")
        table.insert(lines, "Sends an alert to all online staff members.")
        player:send(table.concat(lines, "\r\n"))
        return
    end

    -- Get the sender's character name for display
    local sender_session = get_session(session_id)
    local sender_name = "Staff"
    if sender_session and sender_session.character_id then
        local char = get_character(sender_session.character_id)
        if char then sender_name = char.name end
    end

    local msg = string.format("{yellow}[STAFF ALERT from %s]{/} %s",
        sender_name, args_str)

    -- broadcast_to_perm sends to all sessions with the given permission
    local count = 0
    if type(broadcast_to_perm) == "function" then
        count = broadcast_to_perm("alert.receive", msg .. "\r\n")
    else
        -- Fallback: iterate sessions manually
        if type(all_sessions) == "function" then
            for _, sid in ipairs(all_sessions()) do
                if type(has_permission) == "function" and has_permission(sid, "alert.receive") then
                    send(sid, msg .. "\r\n")
                    count = count + 1
                end
            end
        end
    end

    -- Journal the alert
    if DAEMON and DAEMON.journal then
        DAEMON.journal.info(string.format("[alert] %s: %s", sender_name, args_str))
    end

    player:send(string.format("{green}Alert sent to %d staff member%s.{/}",
        count, count == 1 and "" or "s"))
end

return M
