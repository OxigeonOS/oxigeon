-- game/init.lua — Game layer entry point
-- Loaded by the engine after mudlib/init.lua.
-- Initializes game-specific daemons and loads areas.

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

-- ─── Items ───────────────────────────────────────────────────────────────────
-- Item definition files return arrays of Item objects. ITEM_D registers them
-- so the drink/use/drop commands can look up items by name.

if DAEMON.items then
    ok, err = pcall(function()
        local ww_items = require('items.wizard_workshop_items')
        DAEMON.items.register_all(ww_items)
    end)
    if not ok then
        log("error", "Failed to load wizard_workshop items: " .. tostring(err))
    end
end

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

-- ─── Autosave ────────────────────────────────────────────────────────────────
-- Periodically save all loaded player data to prevent data loss on crash.

if DAEMON.ticker and DAEMON.character then
    DAEMON.ticker.every(300, "system.autosave", function()
        local loaded = DAEMON.character.all_loaded and DAEMON.character.all_loaded()
        if not loaded then return end

        local count = 0
        for char_id, _ in pairs(loaded) do
            local save_ok, save_err = pcall(DAEMON.character.save, char_id)
            if not save_ok then
                log("error", "Autosave failed for char "
                    .. tostring(char_id) .. ": " .. tostring(save_err))
            else
                count = count + 1
            end
        end
        if count > 0 then
            log("info", "Autosave: saved " .. count .. " character(s)")
        end
    end)
    log("info", "Autosave timer registered (every 300s)")
end
