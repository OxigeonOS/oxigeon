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
    "{cyan}Online Creation (OLC){/}",
    "{cyan}─────────────────────{/}",
    "  olc <area_name>   Enter OLC mode for an area",
    "  olc done          Exit OLC mode",
    "  olc quit          Exit OLC mode",
    "  olc               Show this help / current status",
    "",
    "{yellow}While in OLC mode:{/}",
    "  dig <dir> <room>  Create a room and link it",
}, "\r\n")

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    local session = get_session(session_id)
    if not session or not session.character_id then return end

    local char_id = session.character_id

    -- ── No args: show status / help ──────────────────────────────────────
    if not args[1] then
        if DAEMON.olc and DAEMON.olc.is_active(session_id) then
            local state = DAEMON.olc.get_state(session_id)
            local lines = {}
            table.insert(lines, "{green}OLC active for area: {yellow}" .. tostring(state.area_name) .. "{/}")
            table.insert(lines, "Use 'olc done' to exit.")
            player:send(table.concat(lines, "\r\n"))
        else
            player:send(HELP_TEXT)
        end
        return
    end

    -- ── olc done / quit ──────────────────────────────────────────────────
    local sub = args[1]:lower()
    if sub == "done" or sub == "quit" then
        if DAEMON.olc and DAEMON.olc.is_active(session_id) then
            DAEMON.olc.stop(session_id)
            player:send("{green}[OLC] Exiting build mode.{/}")
        else
            player:send("{red}You are not in OLC mode.{/}")
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
            player:send("{red}Area '{yellow}" .. area_name .. "{red}' does not exist and you lack the 'olc.areas' permission to create it.{/}")
            return
        end

        -- Get builder name from player
        local builder_name = player.name or "Unknown"

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
            player:send("{red}[OLC] Error creating area. See logs.{/}")
            return
        end

        player:send("{green}[OLC] Created new area: {yellow}" .. area_name .. "{/}")

        -- Show the entrance room
        if DAEMON.world then
            local room = DAEMON.world.get_character_room_obj(char_id)
            if room then
                player:send(room:get_appearance(session_id))
            end
        end
    end

    -- Enter OLC mode
    DAEMON.olc.start(session_id, area_name)
    player:send("{green}[OLC] Entering build mode for area: {yellow}" .. area_name .. "{/}")
end

return M
