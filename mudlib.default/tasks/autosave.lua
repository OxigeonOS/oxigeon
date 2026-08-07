-- game/tasks/autosave.lua — Periodic save of all loaded player data
-- Prevents data loss on crash by saving character state to the database.
-- Registered as a ticker in game/init.lua using the configured interval.

local M = {}

--- Execute the autosave: iterate all loaded characters and save each one.
-- Errors on individual saves are logged but do not prevent other saves.
function M.run()
    if not DAEMON or not DAEMON.character then
        log("warn", "AUTOSAVE: character_d not available, skipping")
        return
    end

    local loaded = DAEMON.character.all_loaded and DAEMON.character.all_loaded()
    if not loaded then return end

    local count = 0
    for char_id, _ in pairs(loaded) do
        local ok, err = pcall(DAEMON.character.save, char_id)
        if not ok then
            log("error", "Autosave failed for char "
                .. tostring(char_id) .. ": " .. tostring(err))
            if DAEMON.journal then
                pcall(function()
                    DAEMON.journal.error("AUTOSAVE: Failed for char "
                        .. tostring(char_id) .. ": " .. tostring(err))
                end)
            end
        else
            count = count + 1
        end
    end
    if count > 0 then
        log("info", "Autosave: saved " .. count .. " character(s)")
    end
end

return M
