-- game/lib/movement.lua — Movement library
-- Handles player movement between rooms with exit checks,
-- on_traverse hooks, and GMCP room updates.

local messaging = require('lib.messaging')
local M = {}

M.OPPOSITES = {
    north = "south",
    south = "north",
    east = "west",
    west = "east",
    northeast = "southwest",
    southwest = "northeast",
    northwest = "southeast",
    southeast = "northwest",
    up = "down",
    down = "up",
    ["in"] = "out",
    out = "in"
}

function M.move(session_id, direction)
    local session = get_session(session_id)
    if not session or not session.character_id then return end

    local char_id = session.character_id
    local current_room_id = DAEMON.world.get_character_room(char_id)
    local current_room = DAEMON.world.get_room(current_room_id)

    if not current_room then
        send(session_id, "You are nowhere.\r\n")
        return
    end

    if not current_room:has_exit(direction) then
        send(session_id, "There is no exit in that direction.\r\n")
        return
    end

    -- Resolve exit info (supports both simple strings and rich table exits)
    local exit_info = current_room:get_exit_info(direction)
    local target_room_id

    if type(exit_info) == "string" then
        -- Simple exit: just a room ID
        target_room_id = exit_info
    elseif type(exit_info) == "table" then
        target_room_id = exit_info.target

        -- Hidden exit check (player might try "go secret_passage")
        -- Hidden exits still work if the player knows the direction name.

        -- Run check function if present
        if exit_info.check then
            local player = get_player(session_id)
            if player then
                local ok, reason = exit_info.check(player)
                if not ok then
                    local msg = reason
                        or exit_info.check_fail
                        or "You can't go that way."
                    send(session_id, msg .. "\r\n")
                    return
                end
            end
        end
    else
        send(session_id, "That exit leads nowhere.\r\n")
        return
    end

    local target_room = DAEMON.world.get_room(target_room_id)
    if not target_room then
        send(session_id, "That exit leads nowhere.\r\n")
        return
    end

    local char_data = get_character(char_id)
    local char_name = char_data and char_data.name or "Someone"

    -- Fire on_traverse hook before moving (for custom messages, item consumption, etc.)
    if type(exit_info) == "table" and exit_info.on_traverse then
        local player = get_player(session_id)
        if player then
            local ok, err = pcall(exit_info.on_traverse, player, direction)
            if not ok then
                log("error", "movement: on_traverse error for exit '"
                    .. direction .. "' in room '" .. current_room_id
                    .. "': " .. tostring(err))
            end
        end
    end

    -- Notify the old room before moving
    messaging.send_to_room(current_room_id, char_name .. " leaves " .. direction .. ".", char_id)

    -- Attempt the move
    local moved = DAEMON.world.move_character(char_id, target_room_id)
    if not moved then
        log("error", "movement: move_character failed for char "
            .. tostring(char_id) .. " to room '" .. target_room_id .. "'")
        send(session_id, "Something went wrong with that exit.\r\n")
        return
    end

    local opp_dir = M.OPPOSITES[direction] or "somewhere"
    messaging.send_to_room(target_room_id, char_name .. " arrives from the " .. opp_dir .. ".", char_id)

    local appearance = target_room:get_appearance(session_id)
    send(session_id, appearance)

    -- Send GMCP Room.Info update
    if DAEMON and DAEMON.gmcp then
        local ok, err = pcall(DAEMON.gmcp.send_room, session_id)
        if not ok then
            log("error", "movement: GMCP send_room failed: " .. tostring(err))
        end
    end
end

return M
