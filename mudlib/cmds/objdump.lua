local M = {}
M.name = 'objdump'
M.aliases = {'@objdump'}
M.category = 'admin'
M.summary = 'Show detailed information dump for a player or room.'
M.permission = 'admin'

-- Sorted, not `pairs` order: a dump you cannot diff against the last one is
-- most of the way to useless.
local function format_dict(d)
    if not d or next(d) == nil then return "(none)" end
    local keys = {}
    for k in pairs(d) do keys[#keys + 1] = k end
    table.sort(keys, function(a, b) return tostring(a) < tostring(b) end)

    local parts = {}
    for _, k in ipairs(keys) do
        parts[#parts + 1] = tostring(k) .. " = " .. tostring(d[k])
    end
    return table.concat(parts, ", ")
end

local function format_array(arr)
    if not arr or #arr == 0 then return "(none)" end
    local parts = {}
    for i, v in ipairs(arr) do parts[i] = tostring(v) end
    return table.concat(parts, ", ")
end

--- Collapse an inventory into "id x2, other_id" in carry order.
---
--- Entries are `{ template = "id" }` tables (Mobile:add_item), with bare
--- strings still reaching this from older saves. Counting the *entry* used a
--- table as the key, so every count was 1 and the table itself went on to
--- `table.concat` — which raised for any player carrying anything at all.
--- Exposed for testing, as `_roll` and `_plan_flush` are.
--- @param inventory table|nil
--- @return string
function M._format_inventory(inventory)
    if type(inventory) ~= "table" then return "(empty)" end

    local counts, order = {}, {}
    for _, entry in ipairs(inventory) do
        local id
        if type(entry) == "string" then
            id = entry
        elseif type(entry) == "table" then
            id = entry.template
        end
        if type(id) == "string" then
            if not counts[id] then
                counts[id] = 0
                order[#order + 1] = id
            end
            counts[id] = counts[id] + 1
        end
    end

    if #order == 0 then return "(empty)" end

    local parts = {}
    for _, id in ipairs(order) do
        parts[#parts + 1] = counts[id] > 1 and (id .. " x" .. counts[id]) or id
    end
    return table.concat(parts, ", ")
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args_str or args_str == "" then
        player:send("Usage: objdump <player_name> | <room_id>\r\n")
        return
    end

    local target_player = nil
    for _, sid in ipairs(all_sessions()) do
        local s = get_session(sid)
        if s and s.state == "playing" and s.character_id then
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
        table.insert(lines, string.format("─── {green}Player{/}: %s ─────────────────────────────", p.name))
        table.insert(lines, string.format("  Char ID: %s | Account: %s | Session: %s", tostring(p.char_id), tostring(p.account_id), tostring(p.session_id)))
        
        local room_id = DAEMON.world and DAEMON.world.get_character_room(p.char_id) or "Unknown"
        table.insert(lines, string.format("  Room: %s", room_id))
        
        -- Through `:trait()` rather than `p.stats`. A derived trait stores
        -- nothing at all, so reading `stats.max_hp` reported 0 for every
        -- character; an effect-modified attribute reported the unbuffed number.
        -- `traits <name>` is where base and effective are shown side by side.
        local function tv(id) return tostring(p:trait(id)) end
        table.insert(lines, string.format("  HP: %s/%s | MP: %s/%s | Level: %s",
            tv("hp"), tv("max_hp"), tv("mp"), tv("max_mp"), tv("level")))
        table.insert(lines, string.format("  STR: %s | DEX: %s | INT: %s | CON: %s",
            tv("strength"), tv("dexterity"), tv("intelligence"), tv("constitution")))
        table.insert(lines, string.format("  XP: %s | Gold: %s", tostring(p.xp or 0), tostring(p.gold or 0)))
        table.insert(lines, string.format("  Title: %s | Race: %s | Gender: %s", p.title or "(none)", p.race or "(none)", p.gender or "(none)"))
        
        local eq_slots = {}
        if type(p.equipment) == "table" then
            for slot in pairs(p.equipment) do eq_slots[#eq_slots + 1] = slot end
            table.sort(eq_slots)
        end
        local eq_parts = {}
        for _, slot in ipairs(eq_slots) do
            eq_parts[#eq_parts + 1] = slot .. " -> " .. tostring(p.equipment[slot])
        end
        table.insert(lines, "  Equipment: " .. (#eq_parts > 0 and table.concat(eq_parts, ", ") or "(empty)"))

        table.insert(lines, "  Inventory: " .. M._format_inventory(p.inventory))
        
        table.insert(lines, "  Channels: " .. format_array(p.channels))
        table.insert(lines, "  Quest flags: " .. format_dict(p.quest_flags))
        -- Skills used to have a line of their own. They are traits now, so
        -- `traits <name>` shows them alongside everything else the character
        -- holds — one place to look rather than two that can disagree.
        table.insert(lines, "  Tags: " .. format_array(p.tags))
        table.insert(lines, "  Custom: " .. format_dict(p.custom))

        player:send(table.concat(lines, "\r\n") .. "\r\n")
        return
    end

    local room = DAEMON.world and DAEMON.world.get_room(args_str)
    if room then
        local lines = {}
        table.insert(lines, string.format("─── {cyan}Room{/}: %s ─────────────", room.id))
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
        
        player:send(table.concat(lines, "\r\n") .. "\r\n")
        return
    end

    player:send("Player or room not found.\r\n")
end

return M
