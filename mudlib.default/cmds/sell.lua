-- mudlib/cmds/sell.lua — The other direction, at a worse rate.
--
-- The gap between what a shop charges and what it pays is the gold sink, and
-- it is a per-shop number rather than a constant so one shop can be a bad place
-- to sell.

local M = {}
M.name = 'sell'
M.aliases = {}
M.category = 'items'
M.summary = 'Sell something to the shop here.'
M.usage = { "sell <item>" }
M.permission = nil

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end
    if not (DAEMON and DAEMON.shop and DAEMON.world) then
        player:send("{red}Nobody is buying.{/}")
        return
    end

    if not args_str or args_str == "" then
        player:send("{cyan}Sell what?{/}")
        return
    end

    local shop = DAEMON.shop.in_room(DAEMON.world.get_character_room(player.char_id))
    if not shop then
        player:send("You are not in a shop.")
        return
    end

    local ok, why, sale = DAEMON.shop.sell(player, shop.id, args_str)
    if not ok then
        player:send("{red}" .. (why or "They will not take that.") .. "{/}")
        return
    end

    local what = sale.item.short or args_str
    player:send("{green}You sell " .. what .. " for " .. sale.gold .. " gold.{/}"
        .. " You have " .. (player.gold or 0) .. " now.")
    player:message_room(player.name .. " sells " .. what .. ".")
end

return M
