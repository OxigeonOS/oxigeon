-- game/cmds/dig.lua — Dig a new exit and room (OLC-only)
-- Usage: dig <direction> <room_id>
-- Creates exit from current room → target. If target doesn't exist, creates it
-- with a return exit. Both rooms are written to disk as Lua data files.

local M = {}

M.name       = "dig"
M.aliases    = {}
M.category   = "building"
M.summary    = "Create a new exit and optionally a new room."
M.permission = "olc"

-- ─── Direction tables ────────────────────────────────────────────────────────

local EXPAND = {
    n  = "north",     s  = "south",     e  = "east",      w  = "west",
    u  = "up",        d  = "down",
    ne = "northeast", nw = "northwest", se = "southeast",  sw = "southwest",
}

local REVERSE = {
    north     = "south",     south     = "north",
    east      = "west",      west      = "east",
    up        = "down",      down      = "up",
    northeast = "southwest", southwest = "northeast",
    northwest = "southeast", southeast = "northwest",
}

--- Split a room_id into area and room_name.
-- "wizard_workshop.laboratory" → "wizard_workshop", "laboratory"
local function split_room_id(room_id)
    local area, room_name = room_id:match("^(.+)%.([^%.]+)$")
    return area, room_name
end

--- Capitalize and humanize a room name.
-- "dark_laboratory" → "Dark Laboratory"
local function humanize(name)
    return name:gsub("_", " "):gsub("(%a)([%w_']*)", function(first, rest)
        return first:upper() .. rest
    end)
end

-- ─── Execute ─────────────────────────────────────────────────────────────────

function M.execute(session_id, args_str, args)
    -- Must be in OLC mode
    if not DAEMON.olc or not DAEMON.olc.is_active(session_id) then
        send(session_id, "\r\nYou must enter OLC mode first. Use: olc <area_name>\r\n")
        return
    end

    if #args < 2 then
        send(session_id, "\r\nUsage: dig <direction> <room_id>\r\n")
        send(session_id, "Example: dig east wizard_workshop.laboratory\r\n")
        send(session_id, "Example: dig n store_room  (area auto-prefixed)\r\n")
        return
    end

    local direction = args[1]:lower()
    local room_id   = args[2]

    -- Expand shorthand directions
    direction = EXPAND[direction] or direction
    if not REVERSE[direction] then
        send(session_id, "\r\nInvalid direction: " .. direction .. "\r\n")
        return
    end

    local olc_state = DAEMON.olc.get_state(session_id)
    local area_name = olc_state.area_name

    -- Auto-prefix area name if room_id has no dot
    if not room_id:find("%.") then
        room_id = area_name .. "." .. room_id
    end

    -- Get the current room via WORLD_D
    local session = get_session(session_id)
    if not session or not session.character_id then return end
    local char_id = session.character_id

    local current_room_id = DAEMON.world.get_character_room(char_id)
    if not current_room_id then
        send(session_id, "\r\nYou are not in any room.\r\n")
        return
    end
    local current_room = DAEMON.world.get_room(current_room_id)
    if not current_room then
        send(session_id, "\r\nCannot find your current room.\r\n")
        return
    end

    -- Check if exit already exists
    if current_room.exits[direction] then
        send(session_id, "\r\nAn exit " .. direction .. " already exists"
            .. " (to " .. current_room.exits[direction] .. ").\r\n")
        return
    end

    -- Split target room_id for codegen paths
    local target_area, target_room_name = split_room_id(room_id)
    if not target_area or not target_room_name then
        send(session_id, "\r\nInvalid room ID format. Expected: area.room_name\r\n")
        return
    end

    -- Get builder name
    local builder_name = "Unknown"
    if DAEMON.character then
        local ok, player = pcall(DAEMON.character.get, char_id)
        if ok and player then builder_name = player.name or "Unknown" end
    end

    local target_room = DAEMON.world.get_room(room_id)
    local created_new = false
    local reverse_dir = REVERSE[direction]

    if not target_room then
        -- ── Create new room ──────────────────────────────────────────────
        local short_name = humanize(target_room_name)

        local new_room_data = {
            id          = room_id,
            short       = short_name,
            description = "A bare room awaiting description.",
            exits       = { [reverse_dir] = current_room_id },
            builder     = builder_name,
        }

        local ok, err = pcall(function()
            -- Write room file to disk
            DAEMON.codegen.write_room_file(target_area, target_room_name, new_room_data)

            -- Load live into world
            local room = DAEMON.room.from_data(new_room_data)
            DAEMON.world.register_room(room)
        end)

        if not ok then
            log("error", "DIG: Failed to create room '" .. room_id .. "': " .. tostring(err))
            if DAEMON.journal then
                DAEMON.journal.error("DIG: Failed to create room: " .. tostring(err))
            end
            send(session_id, "\r\n[OLC] Error creating room. See logs.\r\n")
            return
        end

        created_new = true
        send(session_id, "\r\n[OLC] Created room: " .. room_id .. "\r\n")
    else
        -- ── Existing room: add return exit ───────────────────────────────
        if not target_room.exits[reverse_dir] then
            target_room.exits[reverse_dir] = current_room_id

            -- Update target room's file on disk
            pcall(function()
                DAEMON.codegen.update_room_exits(target_area, target_room_name,
                    { [reverse_dir] = current_room_id })
            end)
        end
    end

    -- ── Add exit on current room ─────────────────────────────────────────
    current_room.exits[direction] = room_id

    -- Update current room's file on disk
    local c_area, c_room_name = split_room_id(current_room_id)
    if c_area and c_room_name then
        pcall(function()
            DAEMON.codegen.update_room_exits(c_area, c_room_name,
                { [direction] = room_id })
        end)
    end

    -- ── Report ───────────────────────────────────────────────────────────
    send(session_id, "[OLC] Exit added: " .. direction .. " → " .. room_id .. "\r\n")
    if reverse_dir then
        send(session_id, "[OLC] Exit added: " .. reverse_dir .. " → "
            .. current_room_id .. " (return exit)\r\n")
    end
    if created_new then
        local path = "areas/" .. target_area .. "/rooms/" .. target_room_name .. ".lua"
        send(session_id, "[OLC] File written: game/" .. path .. "\r\n")
    end
end

return M
