-- mudlib/cmds/inventory.lua — Show the player's inventory
-- Displays all items the player is carrying, using ITEM_D for display names.
-- Supports both instance tables and legacy string entries.

local M = {}

M.name = 'inventory'
M.aliases = {'i', 'inv'}
M.category = 'items'
M.summary = 'View what you are carrying.'

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then
        send(session_id, "\r\nYou need to be in the game to do that.\r\n")
        return
    end

    if not player.inventory or #player.inventory == 0 then
        send(session_id, "\r\nYou are not carrying anything.\r\n")
        return
    end

    send(session_id, "\r\nYou are carrying:\r\n")

    -- Count items for stacking display
    local counts = {}   -- template_id → count
    local order = {}    -- preserve insertion order
    for _, entry in ipairs(player.inventory) do
        local template_id
        if type(entry) == "string" then
            template_id = entry
        elseif type(entry) == "table" then
            template_id = entry.template
        end

        if template_id then
            if not counts[template_id] then
                counts[template_id] = 0
                order[#order + 1] = template_id
            end
            counts[template_id] = counts[template_id] + 1
        end
    end

    for _, template_id in ipairs(order) do
        local count = counts[template_id]
        local display_name = template_id  -- fallback

        -- Look up the item in the registry for a proper display name
        if DAEMON and DAEMON.items then
            local item = DAEMON.items.get(template_id)
            if item then
                local short = item.short
                if type(short) == "string" then
                    display_name = short
                end
            end
        end

        if count > 1 then
            send(session_id, "  " .. display_name .. " (x" .. count .. ")\r\n")
        else
            send(session_id, "  " .. display_name .. "\r\n")
        end
    end
end

return M
