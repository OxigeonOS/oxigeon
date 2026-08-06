-- mudlib/lib/patch.lua — The half of an area that is code.
--
-- OLC regenerates `rooms.lua`, `items.lua` and `mobs.lua` wholesale. That is
-- only safe if the things it cannot express live somewhere it never touches, and
-- `custom.lua` is that somewhere:
--
--   -- game/areas/sunken_reach/custom.lua — hand-written. OLC never reads or
--   -- writes this file.
--   local function pull_chain(session_id, args_str, args) ... end
--
--   return {
--       rooms = {
--           ["sunken_reach.cistern"] = {
--               actions     = { pull = { func = pull_chain, hint = "pull the chain" } },
--               description = function(room) ... end,
--           },
--       },
--       items = { ["reach_lantern"] = { on_use = function(item, user_id) ... end } },
--       mobs  = { ["reach_eel"]     = { on_death = function(mob) ... end } },
--       on_load = function(area_name) ... end,
--   }
--
-- ─── Merged before construction, not after ───────────────────────────────────
--
-- A patched `damage` has to reach `weapon.from_data`, not a weapon component
-- that has already been built from the unpatched value. So the merge happens on
-- the flat authoring data, which is also why `custom.lua` can patch a field the
-- generated file does not mention at all.
--
-- ─── There is no way to delete a generated key ───────────────────────────────
--
-- Deliberately. A `REMOVE` sentinel is four lines and I am not adding it: if a
-- field should not be there, take it out in OLC, where it shows up in the file
-- and in the diff. A deletion that only exists in the patch file is invisible
-- in the data file everybody reads.

local schema = require('lib.schema')

local M = {}

local function log_warn(message)
    log("warn", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.warn, message) end
end

--- Which of a kind's fields merge key-by-key rather than being replaced.
---
--- The schema decides, not a guess about shape. "Does it look like an array"
--- gets it wrong the first time somebody patches an *empty* `exits`, and gets it
--- wrong silently.
local function map_fields(kind)
    local out = {}
    for _, f in ipairs(schema.fields_for(kind, {})) do
        if f.type == "map" then out[f.name] = true end
    end
    return out
end

--- One patch over one datum. The patch side wins on every key it names.
--- @param kind string
--- @param data table
--- @param patch table
--- @return table  the same `data`, mutated
function M.merge_one(kind, data, patch)
    if type(patch) ~= "table" then return data end
    local maps = map_fields(kind)

    for key, value in pairs(patch) do
        if maps[key] and type(value) == "table" and type(data[key]) == "table" then
            for k, v in pairs(value) do data[key][k] = v end
        else
            data[key] = value
        end
    end
    return data
end

--- Merge every patch into its generated datum.
---
--- An id in the patch that matches nothing generated is a **warning**, not a
--- silence. It is how a room you renamed in OLC gets noticed — the patch that
--- carried its actions is now pointing at nothing, and the room has quietly lost
--- its behaviour.
--- @param kind string       "room" | "item" | "mob"
--- @param list table        array of flat data tables, each with an `id`
--- @param patches table|nil { [id] = { field = value } }
--- @return table list, table report  report = { applied = n, orphans = { id, ... } }
function M.apply(kind, list, patches)
    local report = { applied = 0, orphans = {} }
    if type(patches) ~= "table" or type(list) ~= "table" then return list, report end

    local by_id = {}
    for _, data in ipairs(list) do
        if type(data) == "table" and data.id then by_id[data.id] = data end
    end

    for id, patch in pairs(patches) do
        local target = by_id[id]
        if target then
            M.merge_one(kind, target, patch)
            report.applied = report.applied + 1
        else
            report.orphans[#report.orphans + 1] = tostring(id)
        end
    end

    table.sort(report.orphans)
    for _, id in ipairs(report.orphans) do
        log_warn("PATCH: custom.lua patches " .. kind .. " '" .. id
            .. "', which no generated file declares. Its behaviour is not applied "
            .. "to anything — was the id renamed?")
    end

    return list, report
end

--- The patch table for one kind, out of a whole `custom.lua`.
--- @param custom table|nil
--- @param kind string
--- @return table|nil
function M.for_kind(custom, kind)
    if type(custom) ~= "table" then return nil end
    -- Keyed by the plural, matching the file names: `rooms`, `items`, `mobs`.
    local plural = kind .. "s"
    local t = custom[plural]
    return type(t) == "table" and t or nil
end

return M
