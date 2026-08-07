-- mudlib/cmds/buy.lua — Hand over gold, take the thing.

local M = {}
M.name = 'buy'
M.aliases = {}
M.category = 'items'
M.summary = 'Buy something from the shop here.'
M.usage = { "buy <item> [count]" }
M.permission = nil

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end
    if not (DAEMON and DAEMON.shop and DAEMON.world) then
        player:send("{red}There is nothing to buy anywhere.{/}")
        return
    end

    if not args_str or args_str == "" then
        player:send("{cyan}Buy what? Try `list` first.{/}")
        return
    end

    local shop = DAEMON.shop.in_room(DAEMON.world.get_character_room(player.char_id))
    if not shop then
        player:send("You are not in a shop.")
        return
    end

    -- A trailing number is a count. Parsed off the end rather than taken from
    -- `args[2]`, because item names have spaces in them and `buy leather
    -- backpack 2` has to work.
    local name, count = args_str:match("^(.-)%s+(%d+)$")
    if not name then name, count = args_str, 1 end

    local ok, why, sale = DAEMON.shop.buy(player, shop.id, name, tonumber(count))
    if not ok then
        player:send("{red}" .. (why or "You cannot buy that.") .. "{/}")
        return
    end

    local what = sale.item.short or name
    if sale.count > 1 then what = what .. " x" .. sale.count end
    player:send("{green}You buy " .. what .. " for " .. sale.total .. " gold.{/}"
        .. " You have " .. (player.gold or 0) .. " left.")
    player:message_room(player.name .. " buys " .. what .. ".")
end

return M
