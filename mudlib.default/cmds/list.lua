-- mudlib/cmds/list.lua — What is for sale here.

local M = {}
M.name = 'list'
M.aliases = { 'wares', 'shop' }
M.category = 'items'
M.summary = 'See what the shop here is selling.'
M.permission = nil

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end
    if not (DAEMON and DAEMON.shop and DAEMON.world) then
        player:send("{red}There is nothing to buy anywhere.{/}")
        return
    end

    local room_id = DAEMON.world.get_character_room(player.char_id)
    local shop = DAEMON.shop.in_room(room_id)
    if not shop then
        player:send("You are not in a shop.")
        return
    end

    local stock = DAEMON.shop.stock(shop.id)
    if #stock == 0 then
        player:send("{yellow}" .. shop.name .. " has nothing at all today.{/}")
        return
    end

    local lines = { "{cyan}" .. shop.name .. "{/}" }
    if shop.greeting then lines[#lines + 1] = "  " .. shop.greeting end
    lines[#lines + 1] = ""
    lines[#lines + 1] = string.format("  {yellow}%-32s %8s %6s{/}", "item", "price", "stock")

    for _, line in ipairs(stock) do
        -- A sold-out line is shown rather than hidden. "It is not here right
        -- now" and "they never had one" are different answers, and a player who
        -- cannot tell them apart will keep coming back to check.
        local count = line.quantity > 0 and tostring(line.quantity) or "{red}--{/}"
        lines[#lines + 1] = string.format("  %-32s %8d %6s",
            line.item.short or line.item_id, line.price, count)
    end

    lines[#lines + 1] = ""
    lines[#lines + 1] = "You have {yellow}" .. (player.gold or 0) .. "{/} gold."
    lines[#lines + 1] = "Try {cyan}buy <item> [count]{/} or {cyan}sell <item>{/}."

    player:send_lines(lines)
end

return M
