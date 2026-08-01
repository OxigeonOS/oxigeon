-- mudlib/cmds/drink.lua — Drink command
-- Searches the player's inventory for a drinkable item matching the given
-- name, displays drink messages, and fires the on_drink hook.
--
-- Requires: DAEMON.items (Item Registry), get_player() global

local M = {}

M.name = 'drink'
M.aliases = {'quaff'}
M.category = 'items'
M.summary = 'Drink a potion or beverage.'

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args[1] then
        player:send("Drink what?")
        return
    end

    -- Item registry is required
    if not DAEMON or not DAEMON.items then
        player:send("{red}The item system is not available.{/}")
        return
    end

    -- Find the item in the player's inventory via the item registry
    local item_id, item = DAEMON.items.find_by_name(args_str, player.inventory)
    if not item then
        player:send("You don't have anything like that to drink.")
        return
    end

    if not item.drinkable then
        player:send("You can't drink that.")
        return
    end

    -- Resolve template variables in drink messages
    local name = player.name or "Someone"
    local short = (type(item.short) == "string" and item.short) or item.id

    local msg = (item.drink_message or "You drink {short}.")
        :gsub("{name}", name):gsub("{short}", short)
    local room_msg = (item.drink_room_message or "{name} drinks {short}.")
        :gsub("{name}", name):gsub("{short}", short)

    -- Send the drink message to the player
    player:send(msg)

    -- Broadcast to the room
    player:message_room(room_msg)

    -- Consume the item if configured to do so
    if item.consumed then
        player:remove_item(item_id)
    end

    -- Fire the on_drink hook for custom behavior (healing, teleportation, etc.)
    if item.on_drink then
        local ok, err = pcall(item.on_drink, item, player)
        if not ok then
            log("error", "DRINK: on_drink hook error for item '"
                .. item_id .. "': " .. tostring(err))
            if DAEMON and DAEMON.journal then
                DAEMON.journal.error("DRINK: on_drink error for '"
                    .. item_id .. "': " .. tostring(err))
            end
        end
    end
end

return M
