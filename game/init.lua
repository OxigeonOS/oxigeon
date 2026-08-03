-- game/init.lua — Game content layer entry point
-- Loaded by the engine after mudlib/init.lua.
-- Registers game-specific areas. All infrastructure (daemons, commands,
-- tasks, libraries) lives in mudlib/. This file handles authored content only.

local ok, err

-- ─── Attributes and effects ──────────────────────────────────────────────────
-- Traits before effects: effect_d refuses a modifier aimed at a gauge or a
-- counter, and it needs the trait definitions to know which those are.
--
-- `seal()` works out the evaluation order and reports anything broken — a
-- dependency on a trait that does not exist, or a cycle, named as a path. A
-- broken trait answers with its default rather than taking the server down.

if DAEMON.trait then
    ok, err = pcall(function()
        DAEMON.trait.define_all(require('traits.core'))
        if not DAEMON.trait.seal() then
            log("warn", "Some traits are broken — see the journal; they will use their defaults")
        end
    end)
    if not ok then log("error", "Failed to load traits: " .. tostring(err)) end
else
    log("warn", "trait_d is not loaded — this game has no attributes")
end

if DAEMON.effect then
    ok, err = pcall(function()
        DAEMON.effect.define_all(require('effects.core'))
    end)
    if not ok then log("error", "Failed to load effects: " .. tostring(err)) end
end

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

        -- Creatures, after the rooms exist for them to stand in.
        if DAEMON.mobs then
            DAEMON.mobs.register_all(require('areas.wizard_workshop.mobs'))
            DAEMON.mobs.populate()
        end
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
