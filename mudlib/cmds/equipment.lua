-- mudlib/cmds/equipment.lua — What you are wearing, slot by slot.
--
-- Every slot is listed, empty ones included. An equipment display that hides
-- what is empty answers "what am I wearing" and not "what could I be wearing",
-- and the second question is the one a player with a full pack is asking.

local Equipment = require('lib.equipment')
local Armor     = require('lib.armor')
local Weapon    = require('lib.weapon')

local M = {}
M.name = 'equipment'
M.aliases = { 'eq', 'worn' }
M.category = 'items'
M.summary = 'Show what you have equipped.'
M.permission = nil

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    local lines = { "{cyan}You are using:{/}", "" }
    local worn_count = 0

    for _, slot in ipairs(Equipment.SLOTS) do
        local entry, item = Equipment.worn(player, slot)
        if entry and item then
            worn_count = worn_count + 1
            local name = item.display_name and item:display_name() or item.short or entry.template

            -- One trailing note per piece, so the line says why it is worth
            -- wearing rather than only that it is worn.
            local note = ""
            if Weapon.is(item) then
                note = string.format("  {yellow}(%d-%d %s){/}",
                    item.weapon.min or 0, item.weapon.max or 0,
                    item.weapon.damage_type or "physical")
            elseif Armor.is(item) then
                note = string.format("  {yellow}(defense %d){/}", Armor.defense(item) or 0)
            end

            lines[#lines + 1] = string.format("  {green}%-9s{/} %s%s", slot, name, note)
        else
            lines[#lines + 1] = string.format("  %-9s {cyan}(empty){/}", slot)
        end
    end

    lines[#lines + 1] = ""
    if worn_count == 0 then
        lines[#lines + 1] = "You are not using anything at all."
    else
        local encumbrance = Equipment.encumbrance(player)
        if encumbrance > 0 then
            lines[#lines + 1] = "Encumbrance: " .. encumbrance
        end
    end

    player:send_lines(lines)
end

return M
