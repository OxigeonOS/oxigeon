-- mudlib/cmds/wear.lua — Put armour on.
--
-- `wear` and `wield` are the same operation with different words and different
-- refusals: you wield a weapon and wear everything else. Both go through
-- `equipment.equip`, so the requirement check, the displaced-item handling and
-- the `equip:` effect source are written once.

local Carry     = require('lib.carry')
local Equipment = require('lib.equipment')
local Weapon    = require('components.weapon')

local M = {}
M.name = 'wear'
M.aliases = { 'don' }
M.category = 'items'
M.summary = 'Put on a piece of equipment.'
M.usage = {
    "wear <item>     put it on",
    "wear all        everything wearable you are carrying",
}
M.permission = nil

--- Shared by `wear` and `wield`. `verb` is what to say; `expect` decides which
--- refusal a mismatch gets, so `wear sword` says "you wield a sword" rather
--- than a generic no.
--- @param expect string  "armour" | "weapon"
function M.perform(player, args_str, verb, expect)
    if not args_str or args_str == "" then
        player:send("{cyan}" .. verb:sub(1, 1):upper() .. verb:sub(2) .. " what?{/}")
        return
    end

    local entry, item, _, why = Carry.find(player, args_str, { inventory = true, room = false })
    if not entry then
        player:send(why or ("{red}You are not carrying a " .. args_str .. ".{/}"))
        return
    end

    local is_weapon = Weapon.is(item)
    if expect == "weapon" and not is_weapon then
        player:send("{red}You cannot wield that. Try `wear`.{/}")
        return
    end
    if expect == "armour" and is_weapon then
        player:send("{red}You wield a weapon rather than wearing it.{/}")
        return
    end

    local name = item.display_name and item:display_name() or item.short or entry.template
    local ok, why, displaced = Equipment.equip(player, entry, item)
    if not ok then
        player:send("{red}" .. (why or "You cannot equip that.") .. "{/}")
        return
    end

    -- The displaced pieces are named, because "you wield the greatsword" while
    -- a shield silently comes off is how a player loses track of what they are
    -- holding.
    for _, old in ipairs(displaced or {}) do
        local old_item = DAEMON.items.resolve(old)
        player:send("You stop using "
            .. ((old_item and old_item.short) or old.template) .. ".")
    end

    local slot = Equipment.slot_for(item)
    player:send("You " .. verb .. " " .. name .. " (" .. slot .. ").")
    player:message_room(player.name .. " " .. verb .. "s " .. name .. ".")
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end
    if not (DAEMON and DAEMON.items) then
        player:send("{red}You cannot equip anything here.{/}")
        return
    end

    if args_str and args_str:lower() == "all" then
        if not player.inventory or #player.inventory == 0 then
            player:send("You are not carrying anything.")
            return
        end
        local carried = {}
        for i, entry in ipairs(player.inventory) do carried[i] = entry end

        local worn = 0
        for _, entry in ipairs(carried) do
            local item = DAEMON.items.resolve(entry)
            -- Silently skips what cannot be worn: `wear all` on a full pack
            -- would otherwise be a screen of refusals about rations and rope.
            if item and item.equippable and not Weapon.is(item) then
                if Equipment.equip(player, entry, item) then
                    worn = worn + 1
                    player:send("You wear " .. (item.short or entry.template) .. ".")
                end
            end
        end
        if worn == 0 then player:send("You have nothing you can wear.") end
        return
    end

    M.perform(player, args_str, "wear", "armour")
end

return M
