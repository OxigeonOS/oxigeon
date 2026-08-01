-- game/cmds/olc.lua — Online Creation command
-- Enter, exit, and manage OLC building mode.
-- Permissions: "olc" to enter, "olc.areas" to create new areas.

local M = {}

M.name       = "olc"
M.aliases    = {}
M.category   = "building"
M.summary    = "Enter or exit the Online Creation system."
M.permission = "olc"

local HELP_TEXT = table.concat({
    "",
    "Online Creation (OLC)",
    "─────────────────────",
    "  olc <area_name>   Enter OLC mode for an area",
    "  olc done          Exit OLC mode",
    "  olc quit          Exit OLC mode",
    "  olc               Show this help / current status",
    "",
    "While in OLC mode:",
    "  dig <dir> <room>  Create a room and link it",
    "",
}, "\r\n")

function M.execute(session_id, args_str, args)
    local session = get_session(session_id)
    if not session or not session.character_id then return end

    local char_id = session.character_id

    -- ── No args: show status / help ──────────────────────────────────────
    if not args[1] then
        if DAEMON.olc and DAEMON.olc.is_active(session_id) then
            local state = DAEMON.olc.get_state(session_id)
            send(session_id, "\r\nOLC active for area: " .. tostring(state.area_name) .. "\r\n")
            send(session_id, "Use 'olc done' to exit.\r\n")
        else
            send(session_id, HELP_TEXT)
        end
        return
    end

    -- ── olc done / quit ──────────────────────────────────────────────────
    local sub = args[1]:lower()
    if sub == "done" or sub == "quit" then
        if DAEMON.olc and DAEMON.olc.is_active(session_id) then
            DAEMON.olc.stop(session_id)
            send(session_id, "\r\n[OLC] Exiting build mode.\r\n")
        else
            send(session_id, "\r\nYou are not in OLC mode.\r\n")
        end
        return
    end

    -- ── olc <area_name> ──────────────────────────────────────────────────
    local area_name = sub  -- already lowercased

    -- Already in OLC? Switch areas.
    if DAEMON.olc and DAEMON.olc.is_active(session_id) then
        DAEMON.olc.stop(session_id)
    end

    -- Check if area exists in the world
    local area_exists = false
    if DAEMON.world then
        local meta = DAEMON.world.get_area_meta(area_name)
        if meta then area_exists = true end
    end

    -- If area doesn't exist, need olc.areas permission to create it
    if not area_exists then
        if not has_permission(session_id, "olc.areas") then
            send(session_id, "\r\nArea '" .. area_name .. "' does not exist "
                .. "and you lack the 'olc.areas' permission to create it.\r\n")
            return
        end

        -- Get builder name from player
        local builder_name = "Unknown"
        if DAEMON.character then
            local ok, player = pcall(DAEMON.character.get, char_id)
            if ok and player then builder_name = player.name or "Unknown" end
        end

        -- Create the area skeleton
        local ok, err = pcall(function()
            -- Write _meta.lua
            DAEMON.codegen.write_meta_file(area_name, {
                name   = area_name,
                title  = area_name:gsub("_", " "):gsub("^%l", string.upper),
                author = builder_name,
                status = "building",
            })

            -- Write entrance room
            local entrance_id = area_name .. ".entrance"
            local entrance_data = {
                id          = entrance_id,
                short       = "Entrance",
                description = "A bare room awaiting description.",
                exits       = {},
                builder     = builder_name,
            }
            DAEMON.codegen.write_room_file(area_name, "entrance", entrance_data)

            -- Load the room live into the world
            local room = DAEMON.room.from_data(entrance_data)
            DAEMON.world.register_room(room)

            -- Register area metadata with world_d
            DAEMON.world.set_area_meta(area_name, {
                name   = area_name,
                title  = area_name,
                author = builder_name,
                status = "building",
            })

            -- Move the builder into the new entrance room
            DAEMON.world.move_character(char_id, entrance_id)
        end)

        if not ok then
            log("error", "OLC: Failed to create area '" .. area_name .. "': " .. tostring(err))
            if DAEMON.journal then
                DAEMON.journal.error("OLC: Failed to create area '" .. area_name
                    .. "': " .. tostring(err))
            end
            send(session_id, "\r\n[OLC] Error creating area. See logs.\r\n")
            return
        end

        send(session_id, "\r\n[OLC] Created new area: " .. area_name .. "\r\n")

        -- Show the entrance room
        if DAEMON.world then
            local room = DAEMON.world.get_character_room_obj(char_id)
            if room then
                send(session_id, room:get_appearance(session_id))
            end
        end
    end

    -- Enter OLC mode
    DAEMON.olc.start(session_id, area_name)
    send(session_id, "\r\n[OLC] Entering build mode for area: " .. area_name .. "\r\n")
end

return M
