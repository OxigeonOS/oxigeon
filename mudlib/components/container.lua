-- mudlib/components/container.lua — The `container` component, its archetype, and its
-- system.
--
-- Same three parts as `weapon` and `armour`, for the same reason: an item that
-- is a container *and* a light source *and* a quest token has no single class
-- it could be.
--
--   Container{...}                the ARCHETYPE — flat data in, an Item out
--   item.container = {...}        the COMPONENT — data, no functions
--   container.can_accept(...)     the SYSTEM   — module functions taking the item
--
-- What is *inside* a container is not part of the component. Contents live in
-- `item_d`'s location index, keyed `"item:<instance id>"`, so a container holds
-- its contents the same way a room does — one mechanism, and `put`/`get from`
-- are the same code path as `drop`/`get`.
--
--   local Container = require('components.container')
--   Container{ id = "backpack", short = "a leather backpack", slot = "back",
--              capacity = 20, capacity_weight = 40 }
--
-- Open/closed and locked state is *per instance* and lives in object state, not
-- in the component: two backpacks built from one template must be able to
-- disagree about whether they are open.

local Item = require('lib.item')

local M = {}

--- Component identity, for `components/init.lua`.
--- `component` is the field this owns on an item; `order` is where its
--- lines sort in `examine`.
M.component = "container"
M.order = 30


-- ─── The component ───────────────────────────────────────────────────────────

--- Build a `container` component from flat authoring data.
--- @param data table
--- @return table
function M.from_data(data)
    data = type(data) == "table" and data or {}

    return {
        -- How many items fit. 0 means unlimited, which a corpse wants: a boss
        -- that dropped eleven things should not silently lose the eleventh.
        capacity        = tonumber(data.capacity) or 0,
        -- Total weight it will hold. 0 means unlimited.
        capacity_weight = tonumber(data.capacity_weight) or 0,
        -- Whether it can be shut at all. A corpse cannot; a chest can.
        closeable       = data.closeable == true,
        -- The state a fresh instance starts in.
        starts_closed   = data.starts_closed == true,
        -- Template id of the key that opens it, or nil for "not lockable".
        key             = type(data.key) == "string" and data.key or nil,
        starts_locked   = data.starts_locked == true,
    }
end

-- ─── The system ──────────────────────────────────────────────────────────────

--- Does this item carry a container component?
--- @param item any
--- @return boolean
function M.is(item)
    return type(item) == "table" and type(item.container) == "table"
end

--- Read a per-instance flag, falling back to the component's starting value.
---
--- Object state rather than a field on the component, because the component is
--- shared with the template: writing `closed` onto it would shut every chest in
--- the game at once. `instance_id` is the *instance's* id, which is what makes
--- two backpacks able to disagree.
local function flag(item, instance_id, key, default)
    if type(instance_id) ~= "string" or type(get_object_state) ~= "function" then
        return default
    end
    local ok, stored = pcall(get_object_state, instance_id, key)
    if ok and stored ~= nil then return stored == true end
    return default
end

--- @param item table          the resolved item (template + overrides)
--- @param instance_id string  the instance's id
--- @return boolean
function M.is_closed(item, instance_id)
    if not M.is(item) or not item.container.closeable then return false end
    return flag(item, instance_id, "closed", item.container.starts_closed)
end

--- @return boolean
function M.is_locked(item, instance_id)
    if not M.is(item) or not item.container.key then return false end
    return flag(item, instance_id, "locked", item.container.starts_locked)
end

--- Open or shut it. Refuses rather than pretends when it is locked, because a
--- silent no-op on a locked chest is indistinguishable from an empty one.
--- @return boolean ok, string|nil why
function M.set_closed(item, instance_id, closed)
    if not M.is(item) then return false, "That is not a container." end
    if not item.container.closeable then
        return false, "It does not close."
    end
    if not closed and M.is_locked(item, instance_id) then
        return false, "It is locked."
    end
    if type(set_object_state) == "function" then
        set_object_state(instance_id, "closed", closed == true)
    end
    return true
end

--- @param key_template string|nil  the template id of the key being used
--- @return boolean ok, string|nil why
function M.set_locked(item, instance_id, locked, key_template)
    if not M.is(item) then return false, "That is not a container." end
    if not item.container.key then
        return false, "It has no lock."
    end
    if key_template ~= item.container.key then
        return false, "That key does not fit."
    end
    if locked and not M.is_closed(item, instance_id) then
        return false, "You would have to close it first."
    end
    if type(set_object_state) == "function" then
        set_object_state(instance_id, "locked", locked == true)
    end
    return true
end

--- Would one more item fit?
---
--- Answers with a reason, not just a boolean: "the backpack is full" and "that
--- is too heavy for the backpack" are different problems with different fixes,
--- and a player told only "you can't" will try the same thing again.
--- @param item table          the resolved container
--- @param instance_id string
--- @param incoming table|nil  the resolved item going in
--- @return boolean ok, string|nil why
function M.can_accept(item, instance_id, incoming)
    if not M.is(item) then return false, "That is not a container." end
    if M.is_closed(item, instance_id) then return false, "It is closed." end

    local c = item.container
    local contents = DAEMON and DAEMON.items and DAEMON.items.contents(instance_id) or {}

    if c.capacity > 0 and #contents >= c.capacity then
        return false, "There is no room left in it."
    end

    if c.capacity_weight > 0 then
        local carried = 0
        for _, entry in ipairs(contents) do
            local resolved = DAEMON.items.resolve(entry)
            carried = carried + ((resolved and resolved.weight) or 0)
        end
        local adding = (incoming and incoming.weight) or 0
        if carried + adding > c.capacity_weight then
            return false, "It will not take the weight."
        end
    end

    return true
end

--- The total weight of a container and everything in it, recursively.
---
--- Recursive because a backpack inside a chest inside a cart is a thing players
--- will build the moment containers exist, and a carry-capacity check that only
--- looked one level down would let them carry the world in a satchel.
--- @param item table
--- @param instance_id string
--- @param seen table|nil  cycle guard; `item_d.move` prevents cycles, but a
---                        corrupted save should not hang the game thread
--- @return number
function M.total_weight(item, instance_id, seen)
    local own = (type(item) == "table" and item.weight) or 0
    if not M.is(item) or type(instance_id) ~= "string" then return own end

    seen = seen or {}
    if seen[instance_id] then return own end
    seen[instance_id] = true

    if not (DAEMON and DAEMON.items) then return own end

    local total = own
    for _, entry in ipairs(DAEMON.items.contents(instance_id)) do
        local resolved = DAEMON.items.resolve(entry)
        if resolved then
            total = total + M.total_weight(resolved, entry.id, seen)
        end
    end
    return total
end

--- The `examine` lines for the container component.
---
--- Unlike weapon and armour, this needs the *instance*: whether a chest is open
--- and what is inside it are per-instance, not properties of the template.
--- @param item table
--- @param ctx table|nil  { instance_id = string|nil }
--- @return table  array of strings, empty when the item is not a container
function M.describe(item, ctx)
    if not M.is(item) then return {} end
    local instance_id = type(ctx) == "table" and ctx.instance_id or nil
    local c = item.container

    local lines = {}
    if M.is_locked(item, instance_id) then
        lines[#lines + 1] = "It is locked."
        return lines
    end
    if M.is_closed(item, instance_id) then
        lines[#lines + 1] = "It is closed."
        return lines
    end

    local contents = DAEMON and DAEMON.items and DAEMON.items.contents(instance_id) or {}
    if #contents == 0 then
        lines[#lines + 1] = "It is empty."
    else
        lines[#lines + 1] = "It contains:"
        for _, entry in ipairs(contents) do
            local resolved = DAEMON.items.resolve(entry)
            if resolved then
                lines[#lines + 1] = "  " .. (resolved.display_name
                    and resolved:display_name() or resolved.short or entry.template)
            end
        end
    end

    if c.capacity > 0 then
        lines[#lines + 1] = "Capacity: " .. #contents .. " / " .. c.capacity
    end
    return lines
end

-- ─── The archetype ───────────────────────────────────────────────────────────

--- Create an Item carrying a `container` component.
--- @param data table  flat authoring data
--- @return table      an Item
function M.new(data)
    data = type(data) == "table" and data or {}
    local item = Item:new(data)
    item.container = M.from_data(data)
    return item
end

return setmetatable(M, { __call = function(_, data) return M.new(data) end })
