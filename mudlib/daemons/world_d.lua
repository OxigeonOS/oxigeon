local M = {}

M._rooms = {}
M._locations = {}
M._virtuals = {}    -- prefix → generator function
M._area_meta = {}   -- area_name → metadata table

-- ─── Area metadata ───────────────────────────────────────────────────────────

--- Store metadata for a loaded area. Called by ROOM_D.load_area().
-- @param meta table  The _meta table from the area file
function M.set_area_meta(area_name, meta)
    M._area_meta[area_name] = meta
    log("debug", "world_d: Stored metadata for area '" .. area_name .. "'")
end

--- Get metadata for a loaded area.
-- @param area_name string
-- @return table|nil
function M.get_area_meta(area_name)
    return M._area_meta[area_name]
end

--- Get all loaded area metadata.
-- @return table  { area_name → meta }
function M.all_area_meta()
    local copy = {}
    for name, meta in pairs(M._area_meta) do
        copy[name] = meta
    end
    return copy
end

-- ─── Room registry ───────────────────────────────────────────────────────────

function M.get_room(room_id)
    -- 1. Check static registry
    if M._rooms[room_id] then
        return M._rooms[room_id]
    end

    -- 2. Check virtual providers by prefix
    local prefix = room_id:match("^([^%.]+)")
    if prefix and M._virtuals[prefix] then
        local ok, room = pcall(M._virtuals[prefix], room_id)
        if ok and room then
            M._rooms[room_id] = room  -- cache while occupied
            return room
        elseif not ok then
            log("error", "world_d: Virtual provider '" .. prefix
                .. "' failed for '" .. room_id .. "': " .. tostring(room))
        end
    end

    return nil
end

function M.register_room(room)
    if not room or not room.id then
        log("warn", "world_d: Attempted to register a room with no ID")
        return
    end
    if M._rooms[room.id] then
        log("warn", "world_d: Overwriting existing room '" .. room.id .. "'")
    end
    M._rooms[room.id] = room
end

function M.register_area(rooms_array)
    if type(rooms_array) ~= "table" then
        log("error", "world_d: register_area called with non-table argument")
        return
    end
    local count = 0
    for _, room in ipairs(rooms_array) do
        M.register_room(room)
        count = count + 1
    end
    log("info", "world_d: Registered " .. count .. " rooms in area.")
end

-- ─── Virtual providers ───────────────────────────────────────────────────────

--- Register a virtual room provider for a given room ID prefix.
-- When get_room() can't find a room in the static registry, it checks
-- virtual providers by matching the prefix (everything before the first dot).
-- The generator function receives the full room_id and must return a Room object
-- (via ROOM_D.from_data()) or nil.
-- @param prefix string         e.g. "ocean", "desert", "sky"
-- @param generator function    function(room_id) → Room|nil
function M.register_virtual(prefix, generator)
    if type(prefix) ~= "string" or prefix == "" then
        log("error", "world_d: register_virtual requires a non-empty string prefix")
        return
    end
    if type(generator) ~= "function" then
        log("error", "world_d: register_virtual requires a function generator")
        return
    end
    M._virtuals[prefix] = generator
    log("info", "world_d: Registered virtual provider for prefix '" .. prefix .. "'")
end

--- Unregister a virtual provider.
function M.unregister_virtual(prefix)
    M._virtuals[prefix] = nil
end

--- List registered virtual prefixes.
function M.virtual_prefixes()
    local prefixes = {}
    for prefix, _ in pairs(M._virtuals) do
        prefixes[#prefixes + 1] = prefix
    end
    return prefixes
end

--- Evict a cached virtual room (e.g. when no players are in it).
-- Only removes from the room registry — does not affect static rooms.
function M.evict_virtual(room_id)
    local prefix = room_id:match("^([^%.]+)")
    if prefix and M._virtuals[prefix] then
        M._rooms[room_id] = nil
        log("debug", "world_d: Evicted virtual room '" .. room_id .. "'")
    end
end

-- ─── Character location tracking ─────────────────────────────────────────────

function M.move_character(char_id, target_room_id)
    local old_room_id = M._locations[char_id]
    if old_room_id then
        local old_room = M._rooms[old_room_id]
        if old_room then
            old_room:remove_character(char_id)
        end
    end

    local new_room = M.get_room(target_room_id) -- uses virtual fallback
    if new_room then
        new_room:add_character(char_id)
        M._locations[char_id] = target_room_id
        return true
    end
    log("warn", "world_d: move_character failed — room '"
        .. tostring(target_room_id) .. "' not found")
    return false
end

function M.get_character_room(char_id)
    return M._locations[char_id]
end

function M.get_character_room_obj(char_id)
    local room_id = M._locations[char_id]
    if room_id then
        return M.get_room(room_id) -- uses virtual fallback
    end
    return nil
end

function M.place_character(char_id, room_id)
    local room = M.get_room(room_id) -- uses virtual fallback
    if room then
        room:add_character(char_id)
        M._locations[char_id] = room_id
        log("debug", "world_d: Placed character " .. tostring(char_id)
            .. " in room '" .. room_id .. "'")
    else
        log("error", "world_d: Cannot place character " .. tostring(char_id)
            .. " — room '" .. tostring(room_id) .. "' does not exist!")
    end
end

function M.remove_character(char_id)
    local room_id = M._locations[char_id]
    if room_id then
        local room = M._rooms[room_id]
        if room then
            room:remove_character(char_id)
        end
        M._locations[char_id] = nil
        log("debug", "world_d: Removed character " .. tostring(char_id)
            .. " from room '" .. room_id .. "'")
    else
        log("debug", "world_d: remove_character called for character "
            .. tostring(char_id) .. " who had no location")
    end
end

-- ─── Area source tracking & resets ───────────────────────────────────────────

-- Stores the require-path for each area so we can reload it fresh.
-- area_name → { module = "areas.wizard_workshop.rooms", items_module = "areas.wizard_workshop.items" }
M._area_sources = {}

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then
        pcall(function() DAEMON.journal.error(message) end)
    end
end

--- Register the Lua module paths for an area so it can be reloaded later.
-- @param area_name     string  e.g. "wizard_workshop"
-- @param room_module   string  require-path for rooms, e.g. "areas.wizard_workshop.rooms"
-- @param items_module  string|nil  optional require-path for items
function M.register_area_source(area_name, room_module, items_module)
    if type(area_name) ~= "string" or area_name == "" then
        log("warn", "world_d: register_area_source requires a non-empty area name")
        return
    end
    if type(room_module) ~= "string" or room_module == "" then
        log("warn", "world_d: register_area_source requires a non-empty room module path")
        return
    end
    M._area_sources[area_name] = {
        module       = room_module,
        items_module = items_module,
    }
    log("debug", "world_d: Registered area source '" .. area_name
        .. "' (module=" .. room_module .. ")")
end

--- Get all room IDs belonging to a given area (prefix match on room_id).
-- @param area_name string  e.g. "wizard_workshop"
-- @return table  array of room ID strings
function M.get_area_rooms(area_name)
    local prefix = area_name .. "."
    local result = {}
    for room_id, _ in pairs(M._rooms) do
        if room_id:sub(1, #prefix) == prefix then
            result[#result + 1] = room_id
        end
    end
    return result
end

--- Reset a single area: reload its Lua module, rebuild Room objects,
-- clear object state for all rooms, and re-place characters.
-- Players currently in the area stay in their room (the Room object is new
-- but the ID is the same).
-- @param area_name string  e.g. "wizard_workshop"
-- @return boolean, string  success, message
function M.reset_area(area_name)
    local source = M._area_sources[area_name]
    if not source then
        return false, "No registered source for area '" .. area_name .. "'"
    end

    log("info", "world_d: Resetting area '" .. area_name .. "'...")

    -- 1. Collect characters currently in this area's rooms
    local prefix = area_name .. "."
    local chars_in_area = {}  -- { char_id = room_id }
    for char_id, room_id in pairs(M._locations) do
        if room_id:sub(1, #prefix) == prefix then
            chars_in_area[char_id] = room_id
        end
    end

    -- 2. Remove characters from old Room objects so they don't hold stale refs
    for char_id, room_id in pairs(chars_in_area) do
        local old_room = M._rooms[room_id]
        if old_room then
            local ok, err = pcall(old_room.remove_character, old_room, char_id)
            if not ok then
                log_error("world_d: Failed to remove char " .. tostring(char_id)
                    .. " from old room during reset: " .. tostring(err))
            end
        end
    end

    -- 3. Purge old rooms belonging to this area
    local old_room_ids = M.get_area_rooms(area_name)
    for _, room_id in ipairs(old_room_ids) do
        M._rooms[room_id] = nil
    end

    -- 4. Clear object state for all rooms in this area
    if type(clear_object_state) == "function" then
        for _, room_id in ipairs(old_room_ids) do
            local ok, err = pcall(clear_object_state, room_id)
            if not ok then
                log_error("world_d: Failed to clear object state for '"
                    .. room_id .. "': " .. tostring(err))
            end
        end
    end

    -- 5. Purge the require cache so the module is re-evaluated fresh
    package.loaded[source.module] = nil
    if source.items_module then
        package.loaded[source.items_module] = nil
    end

    -- 6. Re-require items (must come before rooms in case rooms reference items)
    if source.items_module and DAEMON.items then
        local ok, err = pcall(function()
            local items = require(source.items_module)
            DAEMON.items.register_all(items)
        end)
        if not ok then
            log_error("world_d: Failed to reload items for area '"
                .. area_name .. "': " .. tostring(err))
        end
    end

    -- 7. Re-require the area module, rebuild rooms, register them
    local ok, err = pcall(function()
        local area_data = require(source.module)
        local rooms = DAEMON.room.load_area(area_data)
        M.register_area(rooms)
    end)
    if not ok then
        log_error("world_d: Failed to reload area '" .. area_name
            .. "': " .. tostring(err))
        return false, "Reload failed: " .. tostring(err)
    end

    -- 8. Re-place characters into the new Room objects
    for char_id, room_id in pairs(chars_in_area) do
        local new_room = M._rooms[room_id]
        if new_room then
            new_room:add_character(char_id)
            -- _locations[char_id] still points to the same room_id, no change needed
        else
            -- Room disappeared during reload — move character to start room
            local start = config and config("game.start_room") or nil
            if start and M._rooms[start] then
                M._rooms[start]:add_character(char_id)
                M._locations[char_id] = start
                log("warn", "world_d: Room '" .. room_id
                    .. "' gone after reset, moved char " .. tostring(char_id)
                    .. " to start room")
            else
                log_error("world_d: Room '" .. room_id
                    .. "' gone after reset and no start room available for char "
                    .. tostring(char_id))
            end
        end
    end

    local msg = "Area '" .. area_name .. "' reset successfully ("
        .. #old_room_ids .. " rooms, "
        .. (function()
            local c = 0
            for _ in pairs(chars_in_area) do c = c + 1 end
            return c
        end)() .. " characters repositioned)"
    log("info", "world_d: " .. msg)

    if DAEMON and DAEMON.journal then
        pcall(function() DAEMON.journal.info("WORLD_D: " .. msg) end)
    end

    return true, msg
end

--- Reset all registered areas.
-- @return number  count of areas successfully reset
function M.reset_all_areas()
    local count = 0
    for area_name, _ in pairs(M._area_sources) do
        local ok, msg = M.reset_area(area_name)
        if ok then
            count = count + 1
        else
            log_error("world_d: reset_all_areas — failed for '"
                .. area_name .. "': " .. tostring(msg))
        end
    end
    log("info", "world_d: reset_all_areas complete (" .. count .. " areas reset)")
    return count
end

return M

