local M = {}
M.name = 'snoop'
M.aliases = {}
M.category = 'admin'
M.summary = 'Snoop on a player.'
M.permission = "cmd.snoop"

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args or #args == 0 then
        local target_sid = DAEMON.snoop.get_target(session_id)
        if target_sid then
            DAEMON.snoop.stop(session_id)
            player:send("Snooping stopped.")
            if DAEMON and DAEMON.audit then
                pcall(DAEMON.audit.log, "cmd.snoop", true, "stopped snooping")
            end
        else
            player:send("You are not snooping anyone.")
        end
        return
    end

    local target_name = args[1]

    local target_player, target_sid
    for _, sid in ipairs(all_sessions()) do
        local p = get_player(sid)
        if p and p.name and p.name:lower() == target_name:lower() then
            target_player = p
            target_sid = sid
            break
        end
    end

    if not target_player then
        player:send("No player named '" .. target_name .. "' is online.")
        return
    end

    DAEMON.snoop.start(session_id, target_sid)
    player:send("You begin snooping " .. target_player.name .. ".")
    
    if DAEMON and DAEMON.audit then
        pcall(DAEMON.audit.log, "cmd.snoop", true, "started snooping " .. target_player.name)
    end
end

return M
