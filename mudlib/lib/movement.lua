-- mudlib/lib/movement.lua — Movement library
-- Handles player movement between rooms with exit checks,
-- on_traverse hooks, and GMCP room updates.

local messaging = require('lib.messaging')
local M = {}

--- Every direction, in the order they should be listed anywhere.
---
--- Canonical, and there are three reasons it has to be:
---
--- * `cmds/directions.lua` had its own copy to register verbs from.
--- * `cmds/building/dig.lua` had a third, private `REVERSE` table — while
---   `docs/src/lua-api/olc.md` claimed it came "from the same table
---   `movement.lua` uses". It did not.
--- * The schema orders a room's `exits` by this, so a generated file reads
---   north-south-east-west rather than alphabetically.
---
--- Three copies of a list is three chances for one of them to be missing a
--- direction, and the symptom is a stair you can author and cannot climb.
M.ORDER = {
    "north", "south", "east", "west",
    "northeast", "northwest", "southeast", "southwest",
    "up", "down", "in", "out",
}

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

--- Short forms, for anything that takes a direction from a player.
---
--- `i` is deliberately absent: it has meant `inventory` for as long as MUDs have
--- had one, so `in` takes no single-letter alias.
M.ABBREVIATIONS = {
    n = "north", s = "south", e = "east", w = "west",
    ne = "northeast", nw = "northwest", se = "southeast", sw = "southwest",
    u = "up", d = "down",
}

--- Expand an abbreviation, or return what was given if it is already a
--- direction. Nil for anything that is neither.
--- @param word string
--- @return string|nil
function M.expand(word)
    if type(word) ~= "string" then return nil end
    word = word:lower()
    if M.OPPOSITES[word] then return word end
    return M.ABBREVIATIONS[word]
end

function M.move(session_id, direction)
    local session = get_session(session_id)
    if not session or not session.character_id then return end

    local char_id = session.character_id

    -- Walking out of a working ends it, and never refuses the move: a channel
    -- that pinned you in place until it finished would be a trap, not a cost.
    if DAEMON and DAEMON.ability then
        pcall(DAEMON.ability.on_moved, char_id)
    end

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

    -- Arriving somewhere pitch dark says so rather than printing the room.
    -- Walking is still allowed: feeling your way through a mine is the point of
    -- the mine, and a movement system that refused would make a lantern a key
    -- rather than a light.
    local Light = require('lib.light')
    local player = get_player(session_id)
    if player and not Light.can_see(player, target_room) then
        send(session_id, "\r\n" .. Light.DARKNESS .. "\r\n")
    else
        send(session_id, target_room:get_appearance(session_id))
    end

    -- Send GMCP Room.Info update
    if DAEMON and DAEMON.gmcp then
        local ok, err = pcall(DAEMON.gmcp.send_room, session_id)
        if not ok then
            log("error", "movement: GMCP send_room failed: " .. tostring(err))
        end
    end
end

return M
