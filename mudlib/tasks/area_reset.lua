-- game/tasks/area_reset.lua — Periodic area reset task
-- Resets all registered areas on a timer, clearing transient object state
-- and reloading room Lua fresh. Player positions are preserved.
-- Registered as a ticker in game/init.lua using the configured interval.

local M = {}

--- Execute a full area reset via world_d.
function M.run()
    if not DAEMON or not DAEMON.world then
        log("warn", "AREA_RESET: world_d not available, skipping")
        return
    end

    local ok, err = pcall(DAEMON.world.reset_all_areas)
    if not ok then
        log("error", "AREA_RESET: reset_all_areas failed: " .. tostring(err))
        if DAEMON.journal then
            pcall(function()
                DAEMON.journal.error("AREA_RESET: reset_all_areas failed: " .. tostring(err))
            end)
        end
    end
end

return M
