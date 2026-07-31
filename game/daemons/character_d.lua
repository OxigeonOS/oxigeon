-- game/daemons/character_d.lua — Character data service daemon
-- Manages Player objects as the in-memory cache with persistence to the database.
--
-- On login:      CHARACTER_D.load(char_id) hydrates a Player object from DB
-- During play:   CHARACTER_D.get(char_id) returns the live Player object
-- On save:       CHARACTER_D.save(char_id) serializes Player.to_save() → DB
-- On disconnect: CHARACTER_D.unload(char_id) saves + removes from cache
--
-- The Player object IS the authoritative state. There is no separate
-- "character data" table — the Player IS the data.
--
-- Usage:
--   DAEMON.character.load(char_id)               -- hydrate Player from DB
--   DAEMON.character.get(char_id)                -- get the live Player object
--   DAEMON.character.save(char_id)               -- persist Player → DB
--   DAEMON.character.unload(char_id)             -- save + remove from cache
--
-- Legacy compatibility (for simple key/value access):
--   DAEMON.character.set(char_id, "gold", 100)   -- sets player.gold = 100
--   DAEMON.character.get_value(char_id, "gold")   -- returns player.gold

local Player = require('lib.player')

local M = {}

-- In-memory cache: char_id -> Player object
M._cache = {}

--- Helper: log an error to both log() and journald (if available).
local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then
        DAEMON.journal.error(message)
    end
end

--- Helper: log a warning to both log() and journald (if available).
local function log_warn(message)
    log("warn", message)
    if DAEMON and DAEMON.journal then
        DAEMON.journal.warn(message)
    end
end

--- Load character data from the database and hydrate a Player object.
-- If no saved data exists, creates a Player with default starting stats.
-- @param char_id number  The character ID
-- @return table          The Player object
function M.load(char_id)
    -- Get the character record from the database (name, account_id, etc.)
    local char_info = get_character(char_id)
    if not char_info then
        log_error("CHARACTER_D: Cannot load — character "
            .. tostring(char_id) .. " not found in database")
        return nil
    end

    -- Load the persisted JSON blob
    local ok, saved = pcall(load_character_data, char_id)
    if not ok then
        log_error("CHARACTER_D: Failed to load saved data for character "
            .. tostring(char_id) .. ": " .. tostring(saved))
        saved = nil
    end
    if type(saved) ~= "table" then
        if saved == nil then
            log("debug", "CHARACTER_D: No existing data for character "
                .. tostring(char_id) .. ", initializing with defaults")
        end
        saved = {}
    end

    -- Hydrate the Player object
    local player = Player:from_save(char_id, char_info, saved)
    M._cache[char_id] = player
    log("debug", "CHARACTER_D: Loaded Player '" .. player.name
        .. "' (char_id=" .. tostring(char_id) .. ")")
    return player
end

--- Save a Player's persistent data to the database.
-- Serializes only the fields listed in Player.SAVE_FIELDS.
-- @param char_id number  The character ID
-- @return boolean        true if save succeeded
function M.save(char_id)
    local player = M._cache[char_id]
    if not player then
        log_warn("CHARACTER_D: Attempted to save unloaded character "
            .. tostring(char_id))
        return false
    end

    local data = player:to_save()

    local ok, result = pcall(save_character_data, char_id, data)
    if not ok then
        log_error("CHARACTER_D: Exception saving data for character "
            .. tostring(char_id) .. ": " .. tostring(result))
        return false
    end

    if result then
        log("debug", "CHARACTER_D: Saved Player '" .. player.name
            .. "' (char_id=" .. tostring(char_id) .. ")")
    else
        log_error("CHARACTER_D: save_character_data returned false for character "
            .. tostring(char_id) .. " — data may be lost!")
    end
    return result == true
end

--- Get the live Player object for a character (from cache).
-- @param char_id number  The character ID
-- @return table|nil      The Player object, or nil if not loaded
function M.get(char_id)
    return M._cache[char_id]
end

--- Get a single value from a Player's fields.
-- Searches the Player object directly.
-- @param char_id number  The character ID
-- @param key string      The field name
-- @return any            The value, or nil
function M.get_value(char_id, key)
    local player = M._cache[char_id]
    if player then
        return player[key]
    end
    return nil
end

--- Set a single field on a Player object (in-memory only).
-- Does NOT automatically persist to DB. Call save() to persist.
-- @param char_id number  The character ID
-- @param key string      The field name
-- @param value any        The value
function M.set(char_id, key, value)
    local player = M._cache[char_id]
    if not player then
        log_warn("CHARACTER_D: set() called on unloaded character "
            .. tostring(char_id))
        return
    end
    player[key] = value
end

--- Merge multiple key/value pairs into a Player (in-memory only).
-- @param char_id number  The character ID
-- @param tbl table       Key/value pairs to set
function M.merge(char_id, tbl)
    local player = M._cache[char_id]
    if not player then
        log_warn("CHARACTER_D: merge() called on unloaded character "
            .. tostring(char_id))
        return
    end
    for k, v in pairs(tbl) do
        player[k] = v
    end
end

--- Save and remove a Player from the in-memory cache.
-- Called on disconnect to persist state and free memory.
-- @param char_id number  The character ID
function M.unload(char_id)
    local player = M._cache[char_id]
    if player then
        -- Clean up any event subscriptions this player had
        if DAEMON and DAEMON.event then
            local ok, err = pcall(DAEMON.event.off_by_prefix, "player." .. char_id .. ".")
            if not ok then
                log_error("CHARACTER_D: Failed to clean up events for character "
                    .. tostring(char_id) .. ": " .. tostring(err))
            end
        end

        -- Clean up any tickers this player had
        if DAEMON and DAEMON.ticker then
            local ok, err = pcall(DAEMON.ticker.remove_by_prefix, "player." .. char_id .. ".")
            if not ok then
                -- ticker might not have remove_by_prefix — that's ok
                log("debug", "CHARACTER_D: ticker cleanup note for char "
                    .. tostring(char_id) .. ": " .. tostring(err))
            end
        end
    end

    M.save(char_id)
    M._cache[char_id] = nil
    log("debug", "CHARACTER_D: Unloaded Player for char_id=" .. tostring(char_id))
end

--- Check if a character's Player object is currently in cache.
-- @param char_id number  The character ID
-- @return boolean
function M.is_loaded(char_id)
    return M._cache[char_id] ~= nil
end

--- Get all currently loaded Player objects.
-- @return table  char_id → Player mapping
function M.all_loaded()
    return M._cache
end

return M
