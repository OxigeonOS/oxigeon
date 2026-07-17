-- mudlib/cmds/alert.lua — Send an alert to all staff (privileged) accounts
-- Sends a formatted message to all online sessions that hold daemon.alert perm.
-- This is for staff-to-staff communication, not player announcements.

local M = {}

M.name       = "alert"
M.aliases    = {}
M.category   = "admin"
M.summary    = "Send an alert to all online staff. Usage: alert <message>"
M.permission = "daemon.alert"

function M.execute(session_id, args_str, args)
    if not args_str or args_str:match("^%s*$") then
        send(session_id, "\r\nUsage: alert <message>\r\n")
        send(session_id, "Sends an alert to all online staff members.\r\n")
        send_prompt(session_id, "> ")
        return
    end

    -- Get the sender's character name for display
    local sender_session = get_session(session_id)
    local sender_name = "Staff"
    if sender_session and sender_session.character_id then
        local char = get_character(sender_session.character_id)
        if char then sender_name = char.name end
    end

    local msg = string.format("\r\n\27[33m[STAFF ALERT from %s]\27[0m %s\r\n",
        sender_name, args_str)

    -- broadcast_to_perm sends to all sessions with the given permission
    local count = 0
    if type(broadcast_to_perm) == "function" then
        count = broadcast_to_perm("daemon.alert", msg)
    else
        -- Fallback: iterate sessions manually
        if type(all_sessions) == "function" then
            for _, sid in ipairs(all_sessions()) do
                if type(has_permission) == "function" and has_permission(sid, "daemon.alert") then
                    send(sid, msg)
                    count = count + 1
                end
            end
        end
    end

    -- Journal the alert
    if DAEMON and DAEMON.journal then
        DAEMON.journal.info(string.format("[alert] %s: %s", sender_name, args_str))
    end

    send(session_id, string.format("\r\nAlert sent to %d staff member%s.\r\n",
        count, count == 1 and "" or "s"))
    send_prompt(session_id, "> ")
end

return M
