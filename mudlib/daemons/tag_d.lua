-- mudlib/daemons/tag_d.lua — A reverse index over the tags things already have.
--
-- `tags` is on `Item`, `Mobile` and `Room`, and every use of it so far has been
-- a forward question: *does this thing have that tag?* — a linear scan over one
-- object's short list, which is fine.
--
-- The backward question is the expensive one. *What is tagged `outdoor`?* means
-- walking every room in the world, and a weather daemon asking it on a tick
-- walks them all again every tick. *Which mobs are in the `town_guard`
-- faction?* is the same shape.
--
-- So: one index, maintained where things are registered, keyed
-- `kind -> tag -> id`. Built here rather than in each daemon because three
-- copies of an index is three places for it to go stale, and a stale index is
-- worse than no index — it answers confidently and wrongly.
--
--   DAEMON.tag.index("room", room.id, room.tags)
--   DAEMON.tag.find("room", "outdoor")     -> { "thornhollow.square", ... }
--   DAEMON.tag.has("room", "thornhollow.square", "outdoor")
--   DAEMON.tag.forget("mob", instance_id)
--
-- Deliberately **not** persisted. Everything it indexes is registered from a
-- file on every boot, so the index is rebuilt as a side effect of loading the
-- world; writing it would create a second copy that can disagree with the
-- first.

local M = {}

--- kind -> tag -> { id -> true }
M._by_tag = {}
--- kind -> id -> { tag -> true }, so `forget` does not have to scan every tag.
M._by_id = {}

local function log_warn(message)
    log("warn", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.warn, message) end
end

local function kind_table(store, kind)
    store[kind] = store[kind] or {}
    return store[kind]
end

--- Record what this thing is tagged with, replacing whatever it said before.
---
--- Replacing rather than adding: an object being re-registered — an area
--- reload, a mob respawning under a template someone just edited — must not
--- leave its old tags behind. That is the failure mode an index has that a
--- linear scan does not.
--- @param kind string   "room" | "mob" | "item" | anything a game invents
--- @param id string
--- @param tags table|nil  array of tag strings
--- @return number  how many tags were recorded
function M.index(kind, id, tags)
    if type(kind) ~= "string" or type(id) ~= "string" then
        log_warn("TAG_D.index: needs a kind and an id")
        return 0
    end

    M.forget(kind, id)
    if type(tags) ~= "table" then return 0 end

    local by_tag = kind_table(M._by_tag, kind)
    local by_id  = kind_table(M._by_id, kind)

    local mine = {}
    local n = 0
    for _, tag in ipairs(tags) do
        if type(tag) == "string" and #tag > 0 then
            by_tag[tag] = by_tag[tag] or {}
            by_tag[tag][id] = true
            mine[tag] = true
            n = n + 1
        end
    end

    if n > 0 then by_id[id] = mine end
    return n
end

--- Take one thing out of the index entirely. Called on despawn and on area
--- reload, for the reason `mob_d` clears object state on despawn: an index
--- keyed on ids that are never reused grows forever otherwise.
--- @return boolean  whether anything was removed
function M.forget(kind, id)
    local by_id = M._by_id[kind]
    local mine = by_id and by_id[id]
    if not mine then return false end

    local by_tag = M._by_tag[kind] or {}
    for tag in pairs(mine) do
        local set = by_tag[tag]
        if set then
            set[id] = nil
            -- An empty set is a table nothing will read again, and tags are
            -- unbounded because a game invents them freely.
            if next(set) == nil then by_tag[tag] = nil end
        end
    end
    by_id[id] = nil
    return true
end

--- Everything of one kind carrying one tag, in a stable order.
---
--- Sorted, not `pairs` order: a weather daemon iterating rooms must visit them
--- in the same order twice, or a bug that only shows on the third room is
--- impossible to reproduce.
--- @return table  array of ids
function M.find(kind, tag)
    local set = (M._by_tag[kind] or {})[tag]
    if not set then return {} end

    local out = {}
    for id in pairs(set) do out[#out + 1] = id end
    table.sort(out)
    return out
end

--- Everything carrying *all* of these tags.
--- @param tags table  array of tag strings
--- @return table  array of ids
function M.find_all(kind, tags)
    if type(tags) ~= "table" or #tags == 0 then return {} end

    -- Start from the smallest set, so the intersection walks as few candidates
    -- as possible. With one common tag and one rare one that is the difference
    -- between checking three things and checking three hundred.
    local smallest, smallest_n = nil, math.huge
    for _, tag in ipairs(tags) do
        local set = (M._by_tag[kind] or {})[tag]
        if not set then return {} end
        local n = 0
        for _ in pairs(set) do n = n + 1 end
        if n < smallest_n then smallest, smallest_n = set, n end
    end

    local out = {}
    for id in pairs(smallest) do
        local all = true
        for _, tag in ipairs(tags) do
            local set = (M._by_tag[kind] or {})[tag]
            if not (set and set[id]) then all = false break end
        end
        if all then out[#out + 1] = id end
    end
    table.sort(out)
    return out
end

--- @return boolean
function M.has(kind, id, tag)
    local set = (M._by_tag[kind] or {})[tag]
    return (set and set[id]) == true
end

--- What this thing is tagged with, sorted.
--- @return table  array of tag strings
function M.tags_of(kind, id)
    local mine = (M._by_id[kind] or {})[id]
    if not mine then return {} end
    local out = {}
    for tag in pairs(mine) do out[#out + 1] = tag end
    table.sort(out)
    return out
end

--- Every tag in use for one kind, with counts. For an admin command, and for
--- finding the typo that made `outdoors` and `outdoor` two different tags.
--- @return table  array of { tag, count }
function M.all(kind)
    local by_tag = M._by_tag[kind] or {}
    local out = {}
    for tag, set in pairs(by_tag) do
        local n = 0
        for _ in pairs(set) do n = n + 1 end
        out[#out + 1] = { tag = tag, count = n }
    end
    table.sort(out, function(a, b) return a.tag < b.tag end)
    return out
end

--- Every kind that has anything indexed.
--- @return table  array of kind names
function M.kinds()
    local out = {}
    for kind in pairs(M._by_tag) do out[#out + 1] = kind end
    table.sort(out)
    return out
end

--- Throw the whole index away. For a full world reload, where rebuilding is
--- cheaper and more honest than reconciling.
function M.clear(kind)
    if kind then
        M._by_tag[kind] = nil
        M._by_id[kind] = nil
    else
        M._by_tag = {}
        M._by_id = {}
    end
end

log("info", "tag_d daemon loaded")

return M
