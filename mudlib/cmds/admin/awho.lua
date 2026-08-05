local M = {}
M.name = 'awho'
M.aliases = {'@who'}
M.category = 'admin'
M.summary = 'Detailed admin who list.'
M.permission = 'admin'

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    local lines = {}
    table.insert(lines, "Admin Who List:")
    table.insert(lines, string.format("%-15s | %-10s | %-15s | %-20s | %-30s", "Name", "Char ID", "IP Address", "State", "Room"))
    table.insert(lines, string.rep("-", 98))

    local count = 0
    for _, sid in ipairs(all_sessions()) do
        local s = get_session(sid)
        if s and s.state == "playing" and s.character_id then
            local p = get_character(s.character_id)
            if p then
                count = count + 1
                local rid = DAEMON.world and DAEMON.world.get_character_room(s.character_id) or "Unknown"
                local ip = s.address or "Unknown"
                table.insert(lines, string.format("%-15s | %-10s | %-15s | %-20s | %-30s", p.name, tostring(s.character_id), ip, s.state, rid))
            end
        end
    end
    
    table.insert(lines, string.rep("-", 98))
    table.insert(lines, string.format("Total players: %d", count))

    player:send(table.concat(lines, "\r\n") .. "\r\n")
end

return M
