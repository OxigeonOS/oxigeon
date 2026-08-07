-- mudlib/cmds/drop.lua — Put something on the floor.
--
-- The other half of `get`, and the reason ground items had to exist: combat
-- loot "went straight to the killer" because there was nowhere else for it to
-- go, and there was nowhere else because nothing could put an item in a room.

local Carry = require('lib.carry')

local M = {}
M.name = 'drop'
M.aliases = { 'dr' }
M.category = 'items'
M.summary = 'Put something down.'
M.usage = {
    "drop <item>     put it on the floor",
    "drop all        everything you are carrying",
}
M.permission = nil

local function drop_one(player, entry, item)
    local name = item.display_name and item:display_name() or item.short or entry.template
    local ok, why = Carry.drop(player, entry, item)
    if not ok then
        player:send("{red}" .. (why or "You cannot drop that.") .. "{/}")
        return false
    end
    player:send("You drop " .. name .. ".")
    player:message_room(player.name .. " drops " .. name .. ".")
    return true
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args_str or args_str == "" then
        player:send("{cyan}Drop what?{/}")
        return
    end
    if not (DAEMON and DAEMON.items and DAEMON.world) then
        player:send("{red}You cannot drop things here.{/}")
        return
    end

    if args_str:lower() == "all" then
        if not player.inventory or #player.inventory == 0 then
            player:send("You are not carrying anything.")
            return
        end
        -- A copy, because dropping removes from the array being walked.
        local carried = {}
        for i, entry in ipairs(player.inventory) do carried[i] = entry end

        local dropped = 0
        for _, entry in ipairs(carried) do
            local item = DAEMON.items.resolve(entry)
            if item and drop_one(player, entry, item) then dropped = dropped + 1 end
        end
        if dropped == 0 then player:send("You could not drop anything.") end
        return
    end

    -- Inventory only: `drop lantern` must never mean the one already at your
    -- feet, however reasonable a prefix match makes that look.
    local entry, item, _, why = Carry.find(player, args_str, { inventory = true, room = false })
    if not entry then
        player:send(why or ("{red}You are not carrying a " .. args_str .. ".{/}"))
        return
    end
    drop_one(player, entry, item)
end

return M
