-- mudlib/daemons/verify_d.lua — Does this area actually work?
--
-- `verify` was "does this one file parse". That is worth having and is kept, but
-- it is not the question a builder has: a file can compile perfectly and still
-- describe a room with an exit into nothing, a creature whose loot names an item
-- that does not exist, or a passage you can walk down and not back.
--
-- ─── Read from disk, not from the registry ───────────────────────────────────
--
-- The single most important choice here. By the time content is in the registry
-- it has already had its duplicate ids collapsed, its unknown fields dropped by
-- the loader, and `custom.lua` applied over the top. What a builder about to
-- save needs to know is *what the next reload will do*, and only the files can
-- answer that.
--
-- ─── It reports; it does not refuse ──────────────────────────────────────────
--
-- No `--fix`, and nothing here changes anything. An auto-fixer over generated
-- content is how somebody loses a description to a linter's idea of tidy, with
-- no undo. The gate lives at `olc save` and `olc adopt`, which is where a write
-- is actually about to happen.

local schema    = require('lib.schema')
local movement  = require('lib.movement')
local serialize = require('lib.serialize')

local M = {}

--- The four severities, in report order.
---
--- `lossy` is its own level rather than an error because it is a different kind
--- of statement: the others are about whether the area *works*, and this one is
--- about whether saving it would destroy something. That is the only category
--- concerned with data loss, and burying it among the warnings is how it gets
--- skimmed past.
M.LEVELS = { "error", "warn", "note", "lossy" }

local function finding(level, where, what, message)
    return { level = level, where = where, what = what, message = message }
end

-- ─── The checks ──────────────────────────────────────────────────────────────

--- Every id declared, and every duplicate.
local function check_ids(out, kind, list, file)
    local seen = {}
    for _, data in ipairs(list or {}) do
        if type(data) ~= "table" then
            out[#out + 1] = finding("error", file, "?", "a non-table entry")
        elseif not data.id then
            out[#out + 1] = finding("error", file, "?", "an entry with no id")
        elseif seen[data.id] then
            out[#out + 1] = finding("error", file, data.id,
                "duplicate id — the later one wins and the earlier is lost")
        else
            seen[data.id] = true
        end
    end
    return seen
end

--- Schema validation, plus what a save would drop.
local function check_schema(out, kind, list, file)
    for _, data in ipairs(list or {}) do
        if type(data) == "table" and data.id then
            local _, errors = schema.validate(kind, data)
            for _, e in ipairs(errors) do
                out[#out + 1] = finding("error", file, data.id, e.path .. " " .. e.message)
            end

            local lossy = schema.lossy(kind, data)
            for _, l in ipairs(lossy) do
                out[#out + 1] = finding("lossy", file, data.id,
                    l.path .. " is a " .. l.why .. " — it belongs in custom.lua")
            end

            for _, path in ipairs(schema.unknown(kind, data)) do
                out[#out + 1] = finding("note", file, data.id,
                    "'" .. path .. "' is in no schema. It is kept as-is, but "
                    .. "nothing validates it and `olc set` cannot reach it.")
            end

            -- The record-level check, only when the per-field walk found
            -- nothing. Both would otherwise report the same function twice —
            -- once by name and once as "this record cannot be written" — and a
            -- linter that says a thing twice is a linter people stop reading.
            if #lossy == 0 then
                local ok, why = serialize.check(data)
                if not ok then
                    out[#out + 1] = finding("lossy", file, data.id,
                        "cannot be written: " .. tostring(why)
                        .. ". It belongs in custom.lua.")
                end
            end
        end
    end
end

--- Exits: do they go somewhere, and can you come back?
local function check_exits(out, rooms, known_rooms)
    for _, room in ipairs(rooms or {}) do
        if type(room) == "table" and room.id and type(room.exits) == "table" then
            for direction, target in pairs(room.exits) do
                local id = type(target) == "table" and target.target or target

                if not movement.OPPOSITES[direction] then
                    out[#out + 1] = finding("warn", "rooms.lua", room.id,
                        "'" .. tostring(direction) .. "' is not a direction, so "
                        .. "nothing can walk it")
                end

                if type(id) ~= "string" then
                    out[#out + 1] = finding("error", "rooms.lua", room.id,
                        direction .. " has no target")
                else
                    local here = known_rooms[id]
                    local elsewhere = DAEMON.world and DAEMON.world.get_room(id)
                    if not here and not elsewhere then
                        out[#out + 1] = finding("error", "rooms.lua", room.id,
                            direction .. " leads to '" .. id .. "', which does not exist")
                    elseif here then
                        -- One-way is legal and sometimes deliberate — a chute, a
                        -- trapdoor — so it is a warning. It is also the single
                        -- easiest thing to do by accident.
                        local back = movement.OPPOSITES[direction]
                        local other = nil
                        for _, r in ipairs(rooms) do if r.id == id then other = r end end
                        local returns = other and type(other.exits) == "table"
                            and other.exits[back]
                        if back and not returns then
                            out[#out + 1] = finding("warn", "rooms.lua", room.id,
                                direction .. " → " .. id .. " is one-way ("
                                .. id .. " has no " .. back .. ")")
                        end
                    end
                end
            end
        end
    end
end

--- Rooms you cannot reach.
---
--- From the declared entrance, and refusing to guess one. Picking "the room with
--- no inbound edges" would choose a different room after every edit and make the
--- orphan list flap between runs, which is how a check stops being read.
local function check_reachable(out, area_name, rooms, meta)
    if #(rooms or {}) == 0 then return end

    local entrance = (meta and meta.entrance) or (area_name .. ".entrance")
    local by_id = {}
    for _, r in ipairs(rooms) do if r.id then by_id[r.id] = r end end

    if not by_id[entrance] then
        out[#out + 1] = finding("warn", "_meta.lua", area_name,
            "no entrance: '" .. entrance .. "' does not exist, so unreachable "
            .. "rooms cannot be detected. Set `entrance` in _meta.lua.")
        return
    end

    local seen, queue = { [entrance] = true }, { entrance }
    while #queue > 0 do
        local id = table.remove(queue)
        local room = by_id[id]
        for _, target in pairs((room and room.exits) or {}) do
            local to = type(target) == "table" and target.target or target
            if type(to) == "string" and by_id[to] and not seen[to] then
                seen[to] = true
                queue[#queue + 1] = to
            end
        end
    end

    for _, r in ipairs(rooms) do
        if r.id and not seen[r.id] then
            out[#out + 1] = finding("warn", "rooms.lua", r.id,
                "unreachable from " .. entrance)
        end
    end
end

--- References to things that have to exist by the time somebody plays.
local function check_references(out, area)
    local function item_exists(id)
        for _, i in ipairs(area.items or {}) do if i.id == id then return true end end
        return DAEMON.items and DAEMON.items.get(id) ~= nil
    end
    local function room_exists(id)
        for _, r in ipairs(area.rooms or {}) do if r.id == id then return true end end
        return DAEMON.world and DAEMON.world.get_room(id) ~= nil
    end

    for _, mob in ipairs(area.mobs or {}) do
        if type(mob) == "table" and mob.id then
            if mob.spawn_room and not room_exists(mob.spawn_room) then
                out[#out + 1] = finding("warn", "mobs.lua", mob.id,
                    "spawn_room '" .. mob.spawn_room .. "' does not exist, so it will "
                    .. "never be placed")
            end
            for i, entry in ipairs(mob.loot_table or {}) do
                if type(entry) == "table" and entry.item_id and not item_exists(entry.item_id) then
                    out[#out + 1] = finding("warn", "mobs.lua", mob.id,
                        "loot[" .. i .. "] names item '" .. entry.item_id
                        .. "', which does not exist")
                end
            end
            for i, id in ipairs(mob.inventory or {}) do
                if type(id) == "string" and not item_exists(id) then
                    out[#out + 1] = finding("warn", "mobs.lua", mob.id,
                        "inventory[" .. i .. "] names item '" .. id
                        .. "', which does not exist")
                end
            end
            for _, id in pairs(mob.patrol or {}) do
                if type(id) == "string" and not room_exists(id) then
                    out[#out + 1] = finding("warn", "mobs.lua", mob.id,
                        "patrols to '" .. id .. "', which does not exist")
                end
            end
        end
    end

    for _, item in ipairs(area.items or {}) do
        if type(item) == "table" and item.key and not item_exists(item.key) then
            out[#out + 1] = finding("warn", "items.lua", item.id,
                "its key '" .. item.key .. "' does not exist, so it can never be unlocked")
        end
    end
end

--- Trait ids, against what `trait_d` actually defines.
local function check_traits(out, kind, list, file)
    if not (DAEMON.trait and DAEMON.trait.get_def) then return end

    for _, data in ipairs(list or {}) do
        if type(data) == "table" and type(data.stats) == "table" then
            local ids = {}
            for id in pairs(data.stats) do ids[#ids + 1] = tostring(id) end
            table.sort(ids)
            for _, id in ipairs(ids) do
                local ok, def = pcall(DAEMON.trait.get_def, id)
                if not ok or not def then
                    out[#out + 1] = finding("warn", file, data.id,
                        "stats." .. id .. " is not a trait trait_d defines. It is "
                        .. "stored and ignored.")
                end
            end
        end
    end
end

--- Components, and whether each has what it needs.
local function check_components(out, items)
    local components = require('components')
    local known = {}
    for _, name in ipairs(components.names()) do known[name] = true end

    for _, item in ipairs(items or {}) do
        if type(item) == "table" and item.id then
            for _, name in ipairs(item.components or {}) do
                if not known[name] then
                    out[#out + 1] = finding("error", "items.lua", item.id,
                        "component '" .. tostring(name) .. "' does not exist. Known: "
                        .. table.concat(components.names(), " "))
                end
            end
        end
    end
end

--- `custom.lua` patches that name nothing.
---
--- The way a rename gets noticed: the patch that carried a room's actions is now
--- pointing at an id no file declares, and the room has quietly lost its
--- behaviour with nothing else to show for it.
local function check_custom(out, area)
    if type(area.custom) ~= "table" then return end

    for _, spec in ipairs(DAEMON.codegen.GENERATED) do
        local patches = area.custom[spec.file]
        if type(patches) == "table" then
            local by_id = {}
            for _, data in ipairs(area[spec.file] or {}) do
                if type(data) == "table" and data.id then by_id[data.id] = true end
            end
            local ids = {}
            for id in pairs(patches) do ids[#ids + 1] = tostring(id) end
            table.sort(ids)
            for _, id in ipairs(ids) do
                if not by_id[id] then
                    out[#out + 1] = finding("error", "custom.lua", id,
                        "patches a " .. spec.kind .. " that " .. spec.file
                        .. ".lua does not declare — was it renamed?")
                end
            end
        end
    end
end

--- Things that are merely unfinished.
local function check_style(out, area_name, area)
    if not area.meta then
        out[#out + 1] = finding("note", "_meta.lua", area_name,
            "missing, so OLC does not manage this area (olc adopt " .. area_name .. ")")
    elseif not area.meta.author or area.meta.author == "Unknown" then
        out[#out + 1] = finding("note", "_meta.lua", area_name, "no author")
    end

    local defaults = schema.defaults("room")
    for _, room in ipairs(area.rooms or {}) do
        if type(room) == "table" and room.id then
            if room.description == defaults.description then
                out[#out + 1] = finding("note", "rooms.lua", room.id,
                    "the description is still the placeholder")
            end
            if not room.tags or #room.tags == 0 then
                out[#out + 1] = finding("note", "rooms.lua", room.id, "no tags")
            end
        end
    end
end

-- ─── The report ──────────────────────────────────────────────────────────────

--- Lint one area.
---
--- `override` lets `olc save` verify what it is *about* to write rather than
--- what is on disk — which is the only useful moment to run it.
--- @param area_name string
--- @param override table|nil  { rooms, items, mobs }
--- @return table  { findings, counts, area, files }
function M.area(area_name, override)
    local area = DAEMON.codegen.read_area(area_name)
    if override then
        for _, key in ipairs({ "rooms", "items", "mobs" }) do
            if override[key] then area[key] = override[key] end
        end
    end

    local out = {}
    for _, e in ipairs(area.errors or {}) do
        out[#out + 1] = finding("error", e.file, area_name, e.message)
    end

    local rooms = check_ids(out, "room", area.rooms, "rooms.lua")
    check_ids(out, "item", area.items, "items.lua")
    check_ids(out, "mob",  area.mobs,  "mobs.lua")

    check_schema(out, "room", area.rooms, "rooms.lua")
    check_schema(out, "item", area.items, "items.lua")
    check_schema(out, "mob",  area.mobs,  "mobs.lua")

    check_exits(out, area.rooms, rooms)
    check_reachable(out, area_name, area.rooms, area.meta)
    check_references(out, area)
    check_traits(out, "mob", area.mobs, "mobs.lua")
    check_traits(out, "item", area.items, "items.lua")
    check_components(out, area.items)
    check_custom(out, area)
    check_style(out, area_name, area)

    local counts = { error = 0, warn = 0, note = 0, lossy = 0 }
    for _, f in ipairs(out) do counts[f.level] = (counts[f.level] or 0) + 1 end

    table.sort(out, function(a, b)
        if a.where ~= b.where then return a.where < b.where end
        if a.what ~= b.what then return tostring(a.what) < tostring(b.what) end
        return a.message < b.message
    end)

    return {
        area     = area_name,
        findings = out,
        counts   = counts,
        rooms    = #(area.rooms or {}),
        items    = #(area.items or {}),
        mobs     = #(area.mobs or {}),
        has_custom = area.custom ~= nil,
    }
end

local HEADINGS = {
    error = "{red}ERRORS{/}",
    warn  = "{yellow}WARNINGS{/}",
    note  = "{cyan}NOTES{/}",
    lossy = "{red}LOSSY{/} — on disk, not owned by the schema. `olc save` would drop these.",
}

--- A report, as lines a builder reads.
--- @param report table
--- @return table  array of strings
function M.render(report)
    local lines = {
        "{cyan}" .. report.area .. "{/} — " .. report.rooms .. " room"
            .. (report.rooms == 1 and "" or "s") .. ", "
            .. report.items .. " item" .. (report.items == 1 and "" or "s") .. ", "
            .. report.mobs .. " creature" .. (report.mobs == 1 and "" or "s")
            .. (report.has_custom and ", custom.lua" or ""),
    }

    for _, level in ipairs(M.LEVELS) do
        local n = report.counts[level] or 0
        if n > 0 then
            lines[#lines + 1] = ""
            lines[#lines + 1] = HEADINGS[level] .. " (" .. n .. ")"
            for _, f in ipairs(report.findings) do
                if f.level == level then
                    lines[#lines + 1] = string.format("  %-12s %-28s %s",
                        f.where, tostring(f.what), f.message)
                end
            end
        end
    end

    local total = report.counts.error + report.counts.warn
        + report.counts.note + report.counts.lossy
    lines[#lines + 1] = ""
    if total == 0 then
        lines[#lines + 1] = "  {green}ok{/}   no findings."
    else
        lines[#lines + 1] = string.format(
            "  %d error%s · %d warning%s · %d note%s · %d lossy",
            report.counts.error, report.counts.error == 1 and "" or "s",
            report.counts.warn,  report.counts.warn == 1 and "" or "s",
            report.counts.note,  report.counts.note == 1 and "" or "s",
            report.counts.lossy)
    end
    return lines
end

log("debug", "VERIFY_D: daemon loaded")

return M
