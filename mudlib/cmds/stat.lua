local M = {}
M.name = 'stat'
M.aliases = {'@stat'}
M.category = 'admin'
M.summary = 'Show detailed stats for a player or room.'
M.permission = 'admin'

local function format_dict(d)
    if not d or next(d) == nil then return "(none)" end
    local parts = {}
    for k, v in pairs(d) do
        table.insert(parts, tostring(k) .. " = " .. tostring(v))
    end
    return table.concat(parts, ", ")
end

local function format_array(arr)
    if not arr or #arr == 0 then return "(none)" end
    return table.concat(arr, ", ")
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args_str or args_str == "" then
        player:send("Usage: stat <player_name> | <room_id>\r\n")
        return
    end

    local target_player = nil
    for _, s in pairs(all_sessions()) do
        if s.state == "playing" and s.character_id then
            local p = DAEMON.character and DAEMON.character.get(s.character_id)
            if p and p.name:lower() == args_str:lower() then
                target_player = p
                break
            end
        end
    end

    if target_player then
        local p = target_player
        local lines = {}
        table.insert(lines, string.format("─── Player: %s ─────────────────────────────", p.name))
        table.insert(lines, string.format("  Char ID: %s | Account: %s | Session: %s", tostring(p.char_id), tostring(p.account_id), tostring(p.session_id)))
        
        local room_id = DAEMON.world and DAEMON.world.get_character_room(p.char_id) or "Unknown"
        table.insert(lines, string.format("  Room: %s", room_id))
        
        local stats = p.stats or {}
        table.insert(lines, string.format("  HP: %s/%s | MP: %s/%s | Level: %s", tostring(stats.hp or 0), tostring(stats.max_hp or 0), tostring(stats.mp or 0), tostring(stats.max_mp or 0), tostring(stats.level or 1)))
        table.insert(lines, string.format("  STR: %s | DEX: %s | INT: %s | CON: %s", tostring(stats.strength or 0), tostring(stats.dexterity or 0), tostring(stats.intelligence or 0), tostring(stats.constitution or 0)))
        table.insert(lines, string.format("  XP: %s | Gold: %s", tostring(p.xp or 0), tostring(p.gold or 0)))
        table.insert(lines, string.format("  Title: %s | Race: %s | Gender: %s", p.title or "(none)", p.race or "(none)", p.gender or "(none)"))
        
        local eq_parts = {}
        if p.equipment and next(p.equipment) then
            for slot, item in pairs(p.equipment) do
                table.insert(eq_parts, slot .. " -> " .. item)
            end
        end
        table.insert(lines, "  Equipment: " .. (#eq_parts > 0 and table.concat(eq_parts, ", ") or "(empty)"))

        local inv_counts = {}
        if p.inventory then
            for _, item in ipairs(p.inventory) do
                inv_counts[item] = (inv_counts[item] or 0) + 1
            end
        end
        local inv_parts = {}
        for item, count in pairs(inv_counts) do
            if count > 1 then
                table.insert(inv_parts, item .. " x" .. count)
            else
                table.insert(inv_parts, item)
            end
        end
        table.insert(lines, "  Inventory: " .. (#inv_parts > 0 and table.concat(inv_parts, ", ") or "(empty)"))
        
        table.insert(lines, "  Channels: " .. format_array(p.channels))
        table.insert(lines, "  Quest flags: " .. format_dict(p.quest_flags))
        table.insert(lines, "  Skills: " .. format_dict(p.skills))
        table.insert(lines, "  Tags: " .. format_array(p.tags))
        table.insert(lines, "  Custom: " .. format_dict(p.custom))

        player:send_lines(lines)
        return
    end

    local room = DAEMON.world and DAEMON.world.get_room(args_str)
    if room then
        local lines = {}
        table.insert(lines, string.format("─── Room: %s ─────────────", room.id))
        table.insert(lines, string.format("  Short: %s", room.short or "(none)"))
        
        local area_name = room.id:match("^(.-)%.")
        local area_meta = area_name and DAEMON.world.all_area_meta and DAEMON.world.all_area_meta()[area_name]
        if area_meta then
            table.insert(lines, string.format("  Area: %s (Level %s, %s)", area_meta.name or area_name, tostring(area_meta.level or "?"), area_meta.status or "unknown"))
        else
            table.insert(lines, string.format("  Area: %s (Unknown)", area_name or "?"))
        end
        
        table.insert(lines, string.format("  Light: %s", tostring(room.light_level or 0)))
        
        local exit_parts = {}
        if room.exits then
            for dir, target in pairs(room.exits) do
                if type(target) == "table" then
                    table.insert(exit_parts, dir .. " → " .. tostring(target.target))
                else
                    table.insert(exit_parts, dir .. " → " .. tostring(target))
                end
            end
        end
        table.insert(lines, "  Exits: " .. (#exit_parts > 0 and table.concat(exit_parts, ", ") or "(none)"))
        
        local char_parts = {}
        if DAEMON.world and DAEMON.world._locations then
            for cid, rid in pairs(DAEMON.world._locations) do
                if rid == room.id then
                    local p = get_character(cid)
                    if p then
                        table.insert(char_parts, string.format("%s (char_id=%s)", p.name, tostring(cid)))
                    end
                end
            end
        end
        table.insert(lines, "  Characters: " .. (#char_parts > 0 and table.concat(char_parts, ", ") or "(none)"))
        
        local items_parts = {}
        if room.items then
            for name, _ in pairs(room.items) do
                table.insert(items_parts, name)
            end
        end
        table.insert(lines, "  Scenery items: " .. (#items_parts > 0 and table.concat(items_parts, ", ") or "(none)"))
        
        local action_parts = {}
        if room.actions then
            for action, _ in pairs(room.actions) do
                table.insert(action_parts, action)
            end
        end
        table.insert(lines, "  Actions: " .. (#action_parts > 0 and table.concat(action_parts, ", ") or "(none)"))
        
        table.insert(lines, "  Object state:")
        local state = get_all_object_state(room.id)
        if state and next(state) then
            for k, v in pairs(state) do
                table.insert(lines, string.format("    %s = %s", tostring(k), tostring(v)))
            end
        else
            table.insert(lines, "    (none)")
        end
        
        player:send_lines(lines)
        return
    end

    player:send("Player or room not found.\r\n")
end

return M
