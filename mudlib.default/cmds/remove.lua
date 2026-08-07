-- mudlib/cmds/remove.lua — Take equipment off.
--
--   remove helmet     by name
--   remove head       by slot, because that is how people say it
--   remove all        everything

local Equipment = require('lib.equipment')

local M = {}
M.name = 'remove'
M.aliases = { 'unwield', 'doff', 'rem' }
M.category = 'items'
M.summary = 'Take off a piece of equipment.'
M.usage = {
    "remove <item>   by name",
    "remove <slot>   by slot: head, chest, weapon, ...",
    "remove all      everything you are wearing",
}
M.permission = nil

local function take_off(player, slot)
    local _, item = Equipment.worn(player, slot)
    local name = item and (item.short or item.id) or slot
    local ok, why = Equipment.unequip(player, slot)
    if not ok then
        player:send("{red}" .. (why or "You are not wearing that.") .. "{/}")
        return false
    end
    player:send("You stop using " .. name .. ".")
    player:message_room(player.name .. " stops using " .. name .. ".")
    return true
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args_str or args_str == "" then
        player:send("{cyan}Remove what?{/}")
        return
    end

    local want = args_str:lower()

    if want == "all" then
        local worn = Equipment.all_worn(player)
        if #worn == 0 then
            player:send("You are not wearing anything.")
            return
        end
        for _, w in ipairs(worn) do take_off(player, w.slot) end
        return
    end

    -- A slot name is unambiguous, so it wins over a name match: `remove weapon`
    -- must mean the weapon slot even if you are also carrying "a weapon rack".
    if Equipment.is_slot(want) then
        if not player.equipment or not player.equipment[want] then
            player:send("{red}You are not wearing anything on your " .. want .. ".{/}")
            return
        end
        take_off(player, want)
        return
    end

    for _, w in ipairs(Equipment.all_worn(player)) do
        local short = w.item and w.item.short
        if (type(short) == "string" and short:lower():find(want, 1, true))
            or (type(w.entry) == "table" and w.entry.template
                and w.entry.template:lower():gsub("_", " "):find(want, 1, true)) then
            take_off(player, w.slot)
            return
        end
    end

    player:send("{red}You are not wearing a " .. args_str .. ".{/}")
end

return M
