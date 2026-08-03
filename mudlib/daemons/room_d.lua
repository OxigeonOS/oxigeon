-- game/daemons/room_d.lua — Room builder service daemon
-- Provides a chainable builder API for clean room definition in area files.
--
-- Usage:
--   local ROOM_D = require('daemons.room_d')
--
--   rooms[#rooms + 1] = ROOM_D.create("area.room_id")
--       :set_short("Room Title")
--       :set_description(my_long_desc)
--       :set_light(2)
--       :set_smell("Pine and cedar.")
--       :set_sound("Wind through branches.")
--       :add_exit("north", "area.other_room")
--       :add_action("search", search_func, "Search the area")
--       :add_item("statue", "A weathered stone statue of a forgotten king.")
--       :finish()

local Room = require('lib.room')

local M = {}

-- ─── Data-oriented room creation ─────────────────────────────────────────────

--- Default values for room data fields.
local DEFAULTS = {
    short       = "A Room",
    description = "You are in a room.",
    light       = 2,
}

--- Create a Room object from a plain data table.
-- This is the preferred way to define rooms in area files.
-- Field mapping:
--   data.id          → room.id           (required)
--   data.short       → room.short
--   data.description → room.long         (string or function)
--   data.light       → room.light_level  (0-3)
--   data.smell       → room.smell        (string or function)
--   data.sound       → room.sound        (string or function)
--   data.exits       → room.exits        { direction = target_room_id }
--   data.items       → room.items        { keyword = description }
--   data.actions     → room.actions      { verb = { func = fn, hint = "..." } }
--   data.tags        → room.tags         { "outdoor", "town", ... }
-- @param data table  A plain room definition table
-- @return Room       The constructed Room object
function M.from_data(data)
    if type(data) ~= "table" then
        log("error", "ROOM_D.from_data: expected table, got " .. type(data))
        return nil
    end
    if not data.id then
        log("error", "ROOM_D.from_data: room data is missing required 'id' field")
        return nil
    end
    if not data.short then
        log("warn", "ROOM_D.from_data: room '" .. data.id
            .. "' has no 'short' description, using default")
    end
    if not data.description then
        log("warn", "ROOM_D.from_data: room '" .. data.id
            .. "' has no 'description', using default")
    end

    -- Map clean field names to Room's internal field names
    local room_data = {
        id          = data.id,
        short       = data.short or DEFAULTS.short,
        long        = data.description or DEFAULTS.description,
        light_level = data.light or DEFAULTS.light,
        smell       = data.smell,
        sound       = data.sound,
        exits       = data.exits or {},
        items       = data.items or {},
        actions     = data.actions or {},
        -- Rooms can be tagged like items and mobs can. A weather daemon asking
        -- "which rooms are outdoors" wants a reverse index rather than a walk
        -- over the whole world on every tick, which is what `tag_d` is for.
        tags        = data.tags or {},
    }

    return Room:new(room_data)
end

--- Process an array of room data tables into Room objects.
-- If the array contains a `_meta` key, it is extracted and stored
-- via DAEMON.world.set_area_meta() (keyed by _meta.name or first room's area prefix).
-- @param area_data table  Array of room data tables (may include _meta)
-- @return table           Array of Room objects
function M.load_area(area_data)
    if type(area_data) ~= "table" then
        log("error", "ROOM_D.load_area: expected table, got " .. type(area_data))
        return {}
    end

    -- Extract _meta if present (string key, skipped by ipairs)
    local meta = area_data._meta

    local rooms = {}
    for i, data in ipairs(area_data) do
        local room = M.from_data(data)
        if room then
            rooms[#rooms + 1] = room
        else
            log("warn", "ROOM_D.load_area: skipped invalid room at index " .. i)
        end
    end

    -- Store area metadata in world_d if available
    if meta and DAEMON and DAEMON.world then
        local area_name = meta.name
        if not area_name and rooms[1] then
            -- Derive area name from first room's ID prefix
            area_name = rooms[1].id:match("^([^%.]+)") or "unknown"
        end
        if area_name then
            DAEMON.world.set_area_meta(area_name, meta)
        end
    end

    return rooms
end

--- Merge multiple room data arrays into one.
-- Useful for multi-file areas where each sub-file returns a data array.
-- Also merges _meta from the first source that has one.
-- @param ...  One or more room data arrays
-- @return table  Combined array suitable for load_area()
function M.merge(...)
    local result = {}
    local sources = {...}
    for _, source in ipairs(sources) do
        if type(source) == "table" then
            -- Preserve _meta from the first source that has one
            if source._meta and not result._meta then
                result._meta = source._meta
            end
            for _, room_data in ipairs(source) do
                result[#result + 1] = room_data
            end
        end
    end
    return result
end

-- ─── Builder "class" ─────────────────────────────────────────────────────────

local Builder = {}
Builder.__index = Builder

--- Begin building a new room. Returns a chainable Builder object.
-- @param id string  Unique room identifier (e.g. "wizard_workshop.entrance")
-- @return Builder
function M.create(id)
    local b = setmetatable({}, Builder)
    b._data = {
        id = id,
        short = "A Room",
        long = "You are in a room.",
        light_level = 2,
        smell = nil,
        sound = nil,
        exits = {},
        actions = {},
        items = {},
    }
    return b
end

--- Set the short description (room title shown in bold at the top of look).
function Builder:set_short(text)
    self._data.short = text
    return self
end

--- Set the long description (full prose shown when looking at the room).
function Builder:set_description(text)
    self._data.long = text
    return self
end

--- Set the light level. 0 = pitch dark, 1 = dim, 2 = normal, 3 = bright.
function Builder:set_light(level)
    self._data.light_level = level
    return self
end

--- Set the ambient smell description.
function Builder:set_smell(text)
    self._data.smell = text
    return self
end

--- Set the ambient sound description.
function Builder:set_sound(text)
    self._data.sound = text
    return self
end

--- Add an exit from this room.
-- @param direction string  e.g. "north", "south", "up", "in"
-- @param target_id string  The room ID the exit leads to
function Builder:add_exit(direction, target_id)
    self._data.exits[direction] = target_id
    return self
end

--- Add a room-scoped command action.
-- When a player types this verb while in the room, the function is called
-- instead of looking up a system command.
-- @param verb string       The command verb (e.g. "search", "pull")
-- @param func function     Callback: func(session_id, args_str, args)
-- @param hint string|nil   Optional hint shown in room description
function Builder:add_action(verb, func, hint)
    self._data.actions[verb] = {
        func = func,
        hint = hint,
    }
    return self
end

--- Add an examinable item to the room.
-- Players can "examine <keyword>" to see the description.
-- @param keyword string      The keyword to examine (e.g. "statue", "painting")
-- @param description string  What the player sees when they examine it
function Builder:add_item(keyword, description)
    self._data.items[keyword] = description
    return self
end

--- Finalize the room and return the completed Room object.
-- Resets builder state. The returned Room is ready for registration.
-- @return Room
function Builder:finish()
    local room = Room:new(self._data)
    self._data = nil  -- invalidate builder
    return room
end

log("debug", "ROOM_D: daemon loaded (builder + data-oriented modes available)")

return M
