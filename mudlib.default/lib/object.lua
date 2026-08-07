-- mudlib/lib/object.lua — Base class for all MUD objects
-- Provides shared fields and methods inherited by Room, Item, Mobile, etc.
-- Every MUD object has: id, short, description, and access to driver state.
--
-- Properties (short, description, smell, sound) can be either strings or
-- functions that return strings (lfun pattern). Use resolve() to read them.

--- @class Object
--- @field id string
--- @field short string
--- @field description string
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
    -- Traits live on `stats`, and any object can hold one — a sword's
    -- durability, a room's corruption. Copied rather than aliased so two items
    -- built from one template do not share a table and wear out together.
    -- Left nil when nothing was authored, because storage is what decides which
    -- traits an entity has: an empty table would be indistinguishable from an
    -- object that deliberately holds none, and materialising one on every
    -- object is the bloat sparse traits exist to avoid.
    if type(data.stats) == "table" then
        obj.stats = {}
        for k, v in pairs(data.stats) do obj.stats[k] = v end
    end
    setmetatable(obj, self)
    return obj
end

-- ─── Traits ──────────────────────────────────────────────────────────────────

--- The effective value of a trait, after the trait graph and every effect on
--- this object have had their say.
---
--- `self.stats[id]` is what is *stored*; this is what is *true*. A ring of
--- strength does not change the stored number — it is an effect, and only this
--- accessor knows about it.
---
--- It lives on `Object` rather than on `Mobile` because a trait is any numeric
--- datum any entity can hold: a sword's durability and a room's corruption are
--- traits in exactly the sense a mob's strength is. Falls back to the raw field
--- when TRAIT_D is not loaded, so a bare object still answers.
--- @param id string
--- @return number
function Object:trait(id)
    if DAEMON and DAEMON.trait and DAEMON.trait.get_def and DAEMON.trait.get_def(id) then
        return DAEMON.trait.value(self, id)
    end
    local raw = self.stats and self.stats[id]
    return type(raw) == "number" and raw or 0
end

--- Does this object hold this trait at all? A different question from what it
--- is worth — `trait` answers an absent trait with its default so arithmetic
--- stays safe, and this is how you ask whether it was ever learned.
--- @param id string
--- @return boolean
function Object:has_trait(id)
    if DAEMON and DAEMON.trait and DAEMON.trait.has then
        return DAEMON.trait.has(self, id)
    end
    return type(self.stats and self.stats[id]) == "number"
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
