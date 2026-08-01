-- game/init.lua — Game layer entry point
-- Loaded by the engine after mudlib/init.lua.
-- Initializes game-specific daemons, loads areas, and registers system tasks.

-- ─── Daemons ─────────────────────────────────────────────────────────────────
-- Each daemon load is protected so one failure doesn't prevent the others.

local ok, err

-- Ensure the first account is always admin (covers pre-existing databases)
if type(set_admin) == "function" then
    pcall(set_admin, 1, true)
end

ok, err = pcall(function() DAEMON.room = require('daemons.room_d') end)
if not ok then log("error", "Failed to load room_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.character = require('daemons.character_d') end)
if not ok then log("error", "Failed to load character_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.world = require('daemons.world_d') end)
if not ok then log("error", "Failed to load world_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.codegen = require('daemons.codegen_d') end)
if not ok then log("error", "Failed to load codegen_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.olc = require('daemons.olc_d') end)
if not ok then log("error", "Failed to load olc_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.items = require('daemons.item_d') end)
if not ok then log("error", "Failed to load item_d daemon: " .. tostring(err)) end

-- ─── Areas ───────────────────────────────────────────────────────────────────
-- Each area lives in its own subdirectory under game/areas/.
-- Area files return plain data tables. ROOM_D.load_area() processes them
-- into Room objects, then world_d registers them and records the source
-- for later resets.

if DAEMON.world and DAEMON.room then
    -- wizard_workshop
    ok, err = pcall(function()
        -- Load items first so they're available if rooms reference them
        if DAEMON.items then
            local ww_items = require('areas.wizard_workshop.items')
            DAEMON.items.register_all(ww_items)
        end

        local area_data = require('areas.wizard_workshop.rooms')
        local rooms = DAEMON.room.load_area(area_data)
        DAEMON.world.register_area(rooms)
        DAEMON.world.register_area_source(
            "wizard_workshop",
            "areas.wizard_workshop.rooms",
            "areas.wizard_workshop.items"
        )
    end)
    if not ok then
        log("error", "Failed to load area 'wizard_workshop': " .. tostring(err))
    end

    -- Register additional areas here using the same pattern:
    -- ok, err = pcall(function()
    --     if DAEMON.items then
    --         local items = require('areas.my_area.items')
    --         DAEMON.items.register_all(items)
    --     end
    --     local area_data = require('areas.my_area.rooms')
    --     local rooms = DAEMON.room.load_area(area_data)
    --     DAEMON.world.register_area(rooms)
    --     DAEMON.world.register_area_source("my_area", "areas.my_area.rooms", "areas.my_area.items")
    -- end)
    -- if not ok then log("error", "Failed to load area 'my_area': " .. tostring(err)) end
else
    log("error", "Cannot register areas: world_d or room_d daemon failed to load.")
end

log("info", "Game world loaded successfully.")

-- ─── System Tasks ────────────────────────────────────────────────────────────
-- Tasks are defined in game/tasks/ and registered here using ticker_d.
-- Intervals are pulled from server.toml via the config() efun.

-- Autosave — periodically save all loaded player data to prevent data loss
if DAEMON.ticker and DAEMON.character then
    local autosave_interval = config("game.autosave_seconds") or 300
    if autosave_interval > 0 then
        local autosave = require('tasks.autosave')
        DAEMON.ticker.every(autosave_interval, "system.autosave", autosave.run)
        log("info", "Autosave timer registered (every " .. autosave_interval .. "s)")
    else
        log("info", "Autosave disabled (autosave_seconds = 0)")
    end
end

-- Area reset — periodically reload area Lua and clear transient state
if DAEMON.ticker and DAEMON.world then
    local reset_interval = config("game.area_reset_seconds") or 900
    if reset_interval > 0 then
        local area_reset = require('tasks.area_reset')
        DAEMON.ticker.every(reset_interval, "system.area_reset", area_reset.run)
        log("info", "Area reset timer registered (every " .. reset_interval .. "s)")
    else
        log("info", "Area resets disabled (area_reset_seconds = 0)")
    end
end
