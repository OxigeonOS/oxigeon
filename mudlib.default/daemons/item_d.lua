-- mudlib/daemons/item_d.lua — Item templates, and the instances that exist.
--
-- Two halves, and the difference matters:
--
--   TEMPLATE   `M.register` / `M.get`. Shared, never mutated, one per item id.
--              "What is a purple potion?"
--   INSTANCE   `M.spawn` / `M.move`. A particular one, with its own id, its own
--              location, and whatever makes it different from its template.
--              "Which purple potion, and where is it?"
--
-- Only templates existed before. Items lived as entries in `player.inventory`
-- and as rows in this registry, and **nothing could put one on a floor** —
-- there was no `get`, no `drop`, no `put`, no `give`. Combat loot "went
-- straight to the killer" because there was nowhere else for it to go.
--
-- The instance model mirrors `mob_d`, which already solved this shape. The one
-- deliberate difference is the id: a mob is `"mob:" .. seq`, because a mob is
-- never saved and a counter is enough. An item instance is `"item:" .. uuid()`,
-- because a container in somebody's inventory **is** saved — and a counter that
-- restarts at zero on every boot would hand out an id that already means
-- something else in a save file.
--
-- Locations are scheme-prefixed strings, so one field answers "where is this"
-- for every kind of somewhere:
--
--   "room:thornhollow.square"   on the ground
--   "char:42"                   carried
--   "item:<uuid>"               inside a container
--
-- Ground items are memory-tier state by the rule in state-cache.md: if the
-- server restarts, the sword someone left in the square is gone, and an area
-- reset puts the world back. Items inside a character are saved by
-- CHARACTER_D as part of `inventory`, which is where they always lived.
--
-- Exposes:
--   DAEMON.items.register(template) / register_all(list) / get(id) / all()
--   DAEMON.items.resolve(entry)             template + instance overrides
--   DAEMON.items.find_by_name(name, inv)    search an inventory array
--   DAEMON.items.spawn(template_id, location, overrides) -> instance | nil
--   DAEMON.items.move(instance, location)
--   DAEMON.items.destroy(instance)
--   DAEMON.items.get_instance(instance_id)
--   DAEMON.items.in_room(room_id) / find_in_room(room_id, name)
--   DAEMON.items.contents(container_id) / find_in_container(container_id, name)
--   DAEMON.items.count()

local matching = require('lib.matching')

local M = {}

-- Registry: item_id → Item object (the template).
M._items = {}

-- Live instances: instance_id → instance table.
M._instances = {}

-- location string → { instance_id = true }. An index, not the truth: an
-- instance's `location` field is. Rebuilt from it by `move`, so the two cannot
-- drift without a bug in exactly one place.
M._by_location = {}

--- Helper: log errors to both log() and journal_d.
local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then
        pcall(DAEMON.journal.error, message)
    end
end

local function log_warn(message)
    log("warn", message)
    if DAEMON and DAEMON.journal then
        pcall(DAEMON.journal.warn, message)
    end
end

-- ─── Templates ───────────────────────────────────────────────────────────────

--- Register an Item definition in the registry.
-- @param item table  An Item object (must have item.id)
function M.register(item)
    if not item or not item.id then
        log_error("ITEM_D: Attempted to register item without an id")
        return
    end
    M._items[item.id] = item

    -- Feed the tag index, which `room_d` and `mob_d` have always done and this
    -- never did — so `DAEMON.tag.find("item", "weapon")` came back empty for
    -- every item in the game, while `Item.tags` was widely authored and
    -- `Item:has_tag` worked. Two ways to ask one question, one of which was
    -- always wrong.
    if DAEMON and DAEMON.tag and type(item.tags) == "table" then
        pcall(DAEMON.tag.index, "item", item.id, item.tags)
    end

    log("debug", "ITEM_D: Registered item '" .. item.id .. "'")
end

--- Flat authoring data in, a registered-ready Item out.
---
--- `Weapon{...}` is the hand-authoring door and is a one-way function: an Item
--- cannot be written back to a file. Everything OLC reads and writes is the flat
--- form this takes, so this is the loader's door.
--- @param data table
--- @return table|nil item, string|nil err
function M.from_data(data)
    local components = require('components')
    return components.build(data)
end

--- An array of flat data or built Items in, an array of Items out.
---
--- An entry that already carries a metatable is passed through untouched.
--- `Object.new` sets one on everything it builds, so that is a reliable "has
--- this been constructed already" test — and it is what lets a hand-authored
--- `Weapon{...}` file and a generated flat file go through the same loader
--- without either having to declare which it is.
--- @param list table
--- @return table  array of Items
function M.build_all(list)
    local out = {}
    for _, entry in ipairs(list or {}) do
        if type(entry) ~= "table" then
            log_error("ITEM_D: build_all found a " .. type(entry) .. " in the list")
        elseif getmetatable(entry) ~= nil then
            out[#out + 1] = entry
        else
            local item, err = M.from_data(entry)
            if item then
                out[#out + 1] = item
            else
                log_error("ITEM_D: could not build item '"
                    .. tostring(entry.id) .. "': " .. tostring(err))
            end
        end
    end
    return out
end

--- Register multiple items at once.
---
--- Takes either shape — built Items or flat authoring data — because
--- `build_all` can tell them apart. Registration is not the place to decide
--- whether something is an Item: a flat table has an `id` and would register
--- perfectly happily, then quietly lack `display_name`, `has_tag` and every
--- `Item:new` default until something asked for one.
-- @param items table  Array of Item objects or flat authoring tables
function M.register_all(items)
    for _, item in ipairs(M.build_all(items)) do
        M.register(item)
    end
end

--- Get an Item definition by ID.
-- @param item_id string  The item ID
-- @return table|nil      The Item object, or nil
function M.get(item_id)
    return M._items[item_id]
end

--- Resolve an inventory entry (instance table) against its template.
-- Merges instance overrides onto the template, producing a full item view.
-- For pristine items (no overrides), returns the template directly.
-- @param entry table  An inventory entry: { template = "id", ... }
-- @return table|nil   The resolved Item (template + overrides), or nil
function M.resolve(entry)
    if type(entry) == "string" then
        -- Legacy string format
        return M._items[entry]
    end
    if type(entry) ~= "table" or not entry.template then
        return nil
    end
    local template = M._items[entry.template]
    if not template then return nil end

    -- Check for any overrides beyond the bookkeeping keys. `id`, `location`
    -- and `template` describe *which* item this is, not what it is like, so
    -- carrying them does not make an instance modified — otherwise every
    -- spawned instance would allocate a merged copy on every read.
    local has_overrides = false
    for k, _ in pairs(entry) do
        if k ~= "template" and k ~= "id" and k ~= "location" then
            has_overrides = true
            break
        end
    end

    -- Pristine item — just return the template
    if not has_overrides then return template end

    -- Modified item — shallow overlay: instance fields win
    local resolved = {}
    for k, v in pairs(template) do resolved[k] = v end
    for k, v in pairs(entry) do
        if k ~= "template" then
            resolved[k] = v
        end
    end
    -- Preserve Item metatable for method access
    setmetatable(resolved, getmetatable(template))
    return resolved
end

--- Search a player's inventory for an item matching a name string.
-- Matches against item.short and item.id using case-insensitive substring matching.
-- Supports both instance tables and legacy string entries.
-- @param name      string  The search string (e.g. "purple potion")
-- @param inventory table   Array of inventory entries
-- @return string|nil, table|nil, number|nil  template_id, resolved Item, index
function M.find_by_name(name, inventory)
    if not name or not inventory then return nil, nil, nil end
    -- Underscores out of the *needle* as well as the haystack. Only the
    -- haystack was normalized, so `get apprentice_dagger` — the id a player
    -- reads off `spawn` or off an area file — matched nothing at all, while
    -- `get apprentice dagger` worked. The two spellings have to mean the same
    -- thing or the id is not a name anyone can use.
    name = name:lower():gsub("_", " ")

    for i, entry in ipairs(inventory) do
        -- Extract template ID from entry
        local template_id
        if type(entry) == "string" then
            template_id = entry
        elseif type(entry) == "table" then
            template_id = entry.template
        end

        if template_id then
            local item = M.resolve(entry)
            if item then
                -- Match against short description (resolved, may have instance override)
                local short = item.short
                if type(short) == "string"
                    and short:lower():gsub("_", " "):find(name, 1, true) then
                    return template_id, item, i
                end
                -- Match against template ID (e.g. "purple_potion" matches "purple")
                if template_id:lower():gsub("_", " "):find(name, 1, true) then
                    return template_id, item, i
                end
            end
        end
    end
    return nil, nil, nil
end

--- Get all registered items.
-- @return table  item_id → Item mapping
function M.all()
    return M._items
end

-- ─── Locations ───────────────────────────────────────────────────────────────

--- Build a location string. Exposed so callers never hand-concatenate one and
--- get the scheme wrong; a typo'd prefix produces an item nobody can find and
--- no error anywhere.
--- @param kind string   "room" | "char" | "item"
--- @param id any
--- @return string|nil
function M.location(kind, id)
    if type(kind) ~= "string" or id == nil then return nil end
    if kind ~= "room" and kind ~= "char" and kind ~= "item" then
        log_warn("ITEM_D.location: unknown location kind '" .. kind .. "'")
        return nil
    end
    return kind .. ":" .. tostring(id)
end

--- Split a location back into its parts.
--- @param location string
--- @return string|nil kind, string|nil id
function M.split_location(location)
    if type(location) ~= "string" then return nil, nil end
    local kind, id = location:match("^(%a+):(.+)$")
    return kind, id
end

local function location_set(location)
    M._by_location[location] = M._by_location[location] or {}
    return M._by_location[location]
end

local function unindex(instance)
    local set = instance.location and M._by_location[instance.location]
    if set then
        set[instance.id] = nil
        -- An empty set is a table nothing will ever read again, and locations
        -- are unbounded (every virtual room coordinate is one). Dropping it is
        -- the same lesson as clearing a despawned mob's object state.
        if next(set) == nil then M._by_location[instance.location] = nil end
    end
end

-- ─── Instances ───────────────────────────────────────────────────────────────

--- Create a live item from a template and put it somewhere.
---
--- @param template_id string
--- @param location string|nil  a `M.location()` string; nil means nowhere yet
--- @param overrides table|nil  per-instance differences from the template
--- @return table|nil  the instance entry
function M.spawn(template_id, location, overrides)
    local template = M._items[template_id]
    if not template then
        log_warn("ITEM_D.spawn: no such item template '" .. tostring(template_id) .. "'")
        return nil
    end

    local instance = { template = template_id }
    if type(overrides) == "table" then
        for k, v in pairs(overrides) do
            if k ~= "template" and k ~= "id" and k ~= "location" then
                instance[k] = v
            end
        end
    end

    -- uuid rather than a counter: an instance inside a container inside a
    -- player's inventory is written to the save file, and a counter that
    -- restarts at zero on the next boot would collide with it.
    instance.id = "item:" .. (type(uuid) == "function" and uuid() or tostring({}):sub(8))

    M._instances[instance.id] = instance
    if location then
        instance.location = location
        location_set(location)[instance.id] = true
    end
    return instance
end

--- Move an instance somewhere else. `nil` takes it out of the world's index
--- without destroying it — which is what happens when it goes into a player's
--- inventory array, where CHARACTER_D owns it from then on.
--- @param instance table
--- @param location string|nil
--- @return boolean
function M.move(instance, location)
    if type(instance) ~= "table" or type(instance.id) ~= "string" then
        log_warn("ITEM_D.move: expected an item instance")
        return false
    end
    if not M._instances[instance.id] then
        log_warn("ITEM_D.move: instance '" .. instance.id .. "' is not registered")
        return false
    end
    if location ~= nil and type(location) ~= "string" then
        log_warn("ITEM_D.move: a location is a string or nil")
        return false
    end

    -- A container cannot be put inside itself, directly or at any depth. The
    -- check is a walk up the chain rather than one comparison, because
    -- `put bag in box; put box in bag` is two legal-looking moves that between
    -- them make a cycle nothing can ever reach again.
    if location then
        local kind, id = M.split_location(location)
        if kind == "item" then
            local seen, cursor = {}, id
            while cursor do
                if cursor == instance.id then
                    log_warn("ITEM_D.move: refusing to put '" .. instance.id
                        .. "' inside itself")
                    return false
                end
                if seen[cursor] then break end
                seen[cursor] = true
                local parent = M._instances[cursor]
                local pkind, pid = M.split_location(parent and parent.location)
                cursor = (pkind == "item") and pid or nil
            end
        end
    end

    unindex(instance)
    instance.location = location
    if location then location_set(location)[instance.id] = true end
    return true
end

--- Take an instance out of the world for good, and everything inside it.
---
--- Clears object state, for the reason `mob_d.despawn` does: the store is keyed
--- by object id, ids are never reused, and nothing else prunes it.
--- @param instance table
--- @return boolean
function M.destroy(instance)
    if type(instance) ~= "table" or not M._instances[instance.id] then return false end

    -- Depth-first, so a chest full of bags full of coins does not leave the
    -- coins addressable by an id whose container no longer exists.
    for _, child in ipairs(M.contents(instance.id)) do
        M.destroy(child)
    end

    unindex(instance)
    M._instances[instance.id] = nil
    if type(clear_object_state) == "function" then
        pcall(clear_object_state, instance.id)
    end
    return true
end

--- @param instance_id string
--- @return table|nil
function M.get_instance(instance_id)
    return M._instances[instance_id]
end

--- Every instance at one location, in a stable order.
---
--- Sorted by id, not `pairs` order: this reaches a player through `look`, and a
--- room whose contents reshuffle between two looks is a bug report.
--- @param location string
--- @return table  array of instances
function M.at(location)
    local set = M._by_location[location]
    if not set then return {} end

    local ids = {}
    for id in pairs(set) do ids[#ids + 1] = id end
    table.sort(ids)

    local out = {}
    for _, id in ipairs(ids) do
        local inst = M._instances[id]
        if inst then out[#out + 1] = inst end
    end
    return out
end

--- Items lying on the floor of a room.
--- @param room_id string
--- @return table  array of instances
function M.in_room(room_id)
    return M.at(M.location("room", room_id) or "")
end

--- What is inside a container instance.
--- @param container_id string  an instance id
--- @return table  array of instances
function M.contents(container_id)
    return M.at(M.location("item", container_id) or "")
end

--- Match a name against a list of instances, by prefix on the display name or
--- the template id. Shared by every "find the thing they meant" call site so
--- `get lantern` and `examine lantern` cannot disagree about which one.
--- @param list table   array of instances
--- @param name string
--- @return table|nil instance, table|nil resolved item
local function match_in(list, name)
    if type(name) ~= "string" or #name == 0 then return nil, nil, nil end

    local inst, why = matching.choose(
        list, name,
        function(entry)
            local item = M.resolve(entry)
            return { item and item.short, entry.template }
        end,
        function(entry)
            local item = M.resolve(entry)
            return (item and item.short) or entry.template or "something"
        end,
        function(entry)
            local item = M.resolve(entry)
            return (item and item.stackable) and entry.template or nil
        end)

    if not inst then return nil, nil, why end
    return inst, M.resolve(inst), nil
end

--- **Returns `nil, nil, <listing>` when several match** and no ordinal was
--- given, which is the same failure shape as finding nothing.
--- @param room_id string
--- @param name string
--- @return table|nil instance, table|nil resolved item, string|nil why
function M.find_in_room(room_id, name)
    return match_in(M.in_room(room_id), name)
end

--- @param container_id string
--- @param name string
--- @return table|nil instance, table|nil resolved item
function M.find_in_container(container_id, name)
    return match_in(M.contents(container_id), name)
end

--- How many instances exist anywhere. For `mudstatus`, and for a test asking
--- whether destroying things actually destroys them.
--- @return number
function M.count()
    local n = 0
    for _ in pairs(M._instances) do n = n + 1 end
    return n
end

log("info", "item_d daemon loaded")

return M
