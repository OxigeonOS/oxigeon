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
---
--- `seed` is the datum the question is asked *about*, and it has to be, because
--- `fields_for` is data-dependent: an item's component fields appear only when
--- the datum names the component. Asking with `{}` — which this did — means
--- `armour.resist` and `armour.stat_bonus` are not in the list, so a patch of
--- `resist = { fire = 2 }` **replaced** the whole resist table instead of merging
--- one key into it. Silently, and only for component fields, which is why it
--- survived: the room and mob cases it was written against have no components.
local function map_fields(kind, seed)
    local out = {}
    for _, f in ipairs(schema.fields_for(kind, seed or {})) do
        if f.type == "map" then out[f.name] = true end
    end
    return out
end

--- One patch over one datum. The patch side wins on every key it names.
---
--- Shared with `lib/prototype.lua`, which folds a chain of these. One merge
--- algorithm rather than two that will disagree about `exits` in six months.
--- @param kind string
--- @param data table
--- @param patch table
--- @param opts table|nil  { seed = <datum for field lookup>, none = <sentinel> }
--- @return table  the same `data`, mutated
function M.merge_one(kind, data, patch, opts)
    if type(patch) ~= "table" then return data end
    opts = opts or {}
    local maps = map_fields(kind, opts.seed or data)

    -- The delete sentinel is **off unless a caller asks for it**. `custom.lua`
    -- has no way to delete a generated key and deliberately keeps none: there
    -- the generated file is the whole truth, so "take it out in OLC" is always
    -- available, and a deletion visible only in the patch file would not be.
    -- A prototyped record is incomplete by construction — the value to remove
    -- lives in the parent's file — so that argument does not carry across, and
    -- only the prototype resolver passes `none`.
    local none = opts.none

    for key, value in pairs(patch) do
        if none ~= nil and value == none then
            data[key] = nil
        elseif maps[key] and type(value) == "table" then
            -- A fresh table rather than mutating `data[key]` in place: the
            -- patch side may be a prototype's own table, shared by every
            -- template that inherits it, and one of them growing a stat would
            -- give it to all of them.
            local merged = {}
            if type(data[key]) == "table" then
                for k, v in pairs(data[key]) do merged[k] = v end
            end
            for k, v in pairs(value) do
                if none ~= nil and v == none then merged[k] = nil else merged[k] = v end
            end
            data[key] = merged
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
