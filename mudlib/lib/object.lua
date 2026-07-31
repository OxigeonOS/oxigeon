-- mudlib/lib/object.lua — Base class for all MUD objects
-- Provides shared fields and methods inherited by Room, Item, Mobile, etc.
-- Every MUD object has: id, short, description, and access to driver state.
--
-- Properties (short, description, smell, sound) can be either strings or
-- functions that return strings (lfun pattern). Use resolve() to read them.

local Object = {}
Object.__index = Object

--- Resolve a property value. If the value is a function, call it
--- and return its result. Mirrors LPC's lfun pattern where setters
--- can accept either a literal string or a callable.
--- @param value any       The stored value (string, function, or other)
--- @param obj   table     The object, passed as argument to function values
--- @return string|nil
function Object.resolve(value, obj)
    if value == nil then
        return nil
    end
    if type(value) == "function" then
        local ok, result = pcall(value, obj)
        if ok and type(result) == "string" then
            return result
        end
        return "<invalid lfun return>"
    end
    if type(value) == "string" then
        return value
    end
    return "<invalid lfun return>"
end

--- Constructor. Initializes common fields from a data table.
--- @param data table  Must contain at least 'id'
--- @return table      The new object
function Object:new(data)
    local obj = {
        id          = data.id or "unknown",
        short       = data.short or "Something",
        description = data.description or data.long or "You see nothing special.",
    }
    setmetatable(obj, self)
    return obj
end

-- ─── Driver state access ─────────────────────────────────────────────────────
-- Wraps the set_object_state/get_object_state efuns, scoped to this object's ID.

--- Get a state value for this object.
--- @param key string  The state key
--- @return any        The value, or nil
function Object:get_state(key)
    return get_object_state(self.id, key)
end

--- Set a state value for this object.
--- @param key string    The state key
--- @param value any     The value to store
function Object:set_state(key, value)
    set_object_state(self.id, key, value)
end

--- Get all state for this object.
--- @return table|nil
function Object:get_all_state()
    return get_all_object_state(self.id)
end

--- Clear all state for this object.
function Object:clear_state()
    clear_object_state(self.id)
end

return Object
