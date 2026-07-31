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

return M
