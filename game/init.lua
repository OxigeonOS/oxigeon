-- game/init.lua — Game layer entry point
-- Loaded by the engine after mudlib/init.lua.
-- Initializes game-specific daemons and loads areas.

-- ─── Daemons ─────────────────────────────────────────────────────────────────
-- Each daemon load is protected so one failure doesn't prevent the others.

local ok, err

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

-- ─── Areas ───────────────────────────────────────────────────────────────────
-- Area files return plain data tables. ROOM_D.load_area() processes them
-- into Room objects, then world_d registers them.

if DAEMON.world and DAEMON.room then
    ok, err = pcall(function()
        local area_data = require('areas.wizard_workshop')
        local rooms = DAEMON.room.load_area(area_data)
        DAEMON.world.register_area(rooms)
    end)
    if not ok then
        log("error", "Failed to load area 'wizard_workshop': " .. tostring(err))
    end
else
    log("error", "Cannot register areas: world_d or room_d daemon failed to load.")
end

log("info", "Game world loaded successfully.")
