local M = {}
M.name = 'areas'
M.aliases = {'@areas'}
M.category = 'admin'
M.summary = 'List and manage areas.'
M.permission = "cmd.areas"

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end
    
    if not DAEMON.world then
        player:send("World daemon not loaded.\r\n")
        return
    end

    if not args_str or args_str == "" then
        local metas = DAEMON.world.all_area_meta and DAEMON.world.all_area_meta() or {}
        local lines = {"Areas:"}
        for name, meta in pairs(metas) do
            local rooms = DAEMON.world.get_area_rooms and DAEMON.world.get_area_rooms(name) or {}
            table.insert(lines, string.format("  %-20s | %-20s | Level: %-5s | Status: %-10s | Rooms: %d", name, meta.title or "Unknown", tostring(meta.level or "?"), meta.status or "unknown", #rooms))
        end
        if #lines == 1 then table.insert(lines, "  (none)") end
        player:send(table.concat(lines, "\r\n") .. "\r\n")
        return
    end

    if args[1] == "reset" then
        if not args[2] then
            player:send("Usage: areas reset <name> | areas reset all\r\n")
            return
        end
        if args[2] == "all" then
            if DAEMON.world.reset_all_areas then
                local success, err = pcall(DAEMON.world.reset_all_areas)
                if success then
                    player:send("All areas reset successfully.\r\n")
                else
                    player:send("Error resetting areas: " .. tostring(err) .. "\r\n")
                end
            else
                player:send("Reset all areas not supported.\r\n")
            end
        else
            if DAEMON.world.reset_area then
                local success, err = pcall(DAEMON.world.reset_area, args[2])
                if success then
                    player:send(string.format("Area '%s' reset successfully.\r\n", args[2]))
                else
                    player:send(string.format("Error resetting area '%s': %s\r\n", args[2], tostring(err)))
                end
            else
                player:send("Reset area not supported.\r\n")
            end
        end
        return
    end

    local area_name = args_str
    local rooms = DAEMON.world.get_area_rooms and DAEMON.world.get_area_rooms(area_name)
    if not rooms or #rooms == 0 then
        player:send("Area not found or has no rooms.\r\n")
        return
    end

    local lines = {string.format("Rooms in %s:", area_name)}
    for _, rid in ipairs(rooms) do
        local room = DAEMON.world.get_room(rid)
        local short = room and room.short or "Unknown"
        local chars = {}
        if DAEMON.world._locations then
            for cid, r in pairs(DAEMON.world._locations) do
                if r == rid then
                    local p = get_character(cid)
                    if p then table.insert(chars, p.name) end
                end
            end
        end
        if #chars > 0 then
            table.insert(lines, string.format("  %-30s | %-20s | Chars: %s", rid, short, table.concat(chars, ", ")))
        else
            table.insert(lines, string.format("  %-30s | %-20s |", rid, short))
        end
    end
    player:send(table.concat(lines, "\r\n") .. "\r\n")
end

return M
