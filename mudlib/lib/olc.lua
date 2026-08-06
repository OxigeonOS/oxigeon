-- mudlib/lib/olc.lua — What the `olc` verb actually does.
--
-- The command file is the grammar; this is the work. Split because the grammar
-- is thirty branches of argument shuffling and every one of them would otherwise
-- have a little bit of world-mutation buried in it — which is how `dig` came to
-- mutate a live Room and then discard the return value of the write that was
-- supposed to persist it.
--
-- Everything here goes through `lib/schema.lua` for what a field is,
-- `daemons/codegen_d.lua` for what a file is, and `daemons/olc_d.lua` for what
-- the session is holding. None of it parses a value itself: `schema.set` is the
-- only string-to-value converter in the system, and a second one would disagree
-- with the first eventually and silently.

local schema     = require('lib.schema')
local components = require('components')

local M = {}

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

-- ─── Reading what exists ─────────────────────────────────────────────────────

--- The live authoring data for something, as flat data.
---
--- A Room and an Item are *built* objects; what OLC edits is the flat form they
--- were built from. For a room that is nearly the object itself; for an item it
--- is `components.to_data`, which is the only direction that can be written back
--- to a file.
--- @param kind string
--- @param id string
--- @return table|nil data, table lossy
function M.live_data(kind, id)
    if kind == "room" then
        local room = DAEMON.world and DAEMON.world.get_room(id)
        if not room then return nil, {} end

        local data = { id = room.id }
        for _, f in ipairs(schema.fields_for("room", {})) do
            local key = f.name
            -- `Room` spells two of them differently: `description` is stored as
            -- `long`, `light` as `light_level`. The schema is the authoring
            -- vocabulary and the class is the runtime one; this is the seam.
            if key == "description" then
                data.description = room.long
            elseif key == "light" then
                data.light = room.light_level
            elseif key ~= "id" then
                data[key] = room[key]
            end
        end
        return data, schema.lossy("room", room)

    elseif kind == "item" then
        local item = DAEMON.items and DAEMON.items.get(id)
        if not item then return nil, {} end
        local data, lossy = components.to_data(item)
        data.id = item.id
        for _, f in ipairs(schema.fields_for("item", data)) do
            if data[f.name] == nil and item[f.name] ~= nil and f.name ~= "components" then
                data[f.name] = item[f.name]
            end
        end
        return data, lossy

    elseif kind == "mob" then
        local t = DAEMON.mobs and DAEMON.mobs.get(id)
        if not t then return nil, {} end
        -- Mob templates are already flat: `mob_d` stores what was authored and
        -- builds at spawn time.
        local data = {}
        for k, v in pairs(t) do data[k] = v end
        return data, schema.lossy("mob", t)
    end

    return nil, {}
end

--- The draft a session is editing, loaded from the live world on first touch.
--- @param session_id string
--- @param kind string
--- @param id string
--- @return table|nil draft, string|nil err
function M.draft(session_id, kind, id)
    local existing = DAEMON.olc.draft(session_id, kind, id)
    if existing then return existing end

    local data = M.live_data(kind, id)
    if not data then return nil, "no " .. kind .. " '" .. tostring(id) .. "'" end
    return DAEMON.olc.put_draft(session_id, kind, id, data)
end

-- ─── Writing back to the live world ──────────────────────────────────────────

--- Rebuild the live object from a draft, so the world changes as you type.
---
--- The demo-able half, and it costs nothing: the alternative is a builder
--- setting a description and having to reset the area to see it.
--- @param kind string
--- @param data table
--- @return boolean ok, string|nil err
function M.apply_live(kind, data)
    if kind == "room" then
        if not (DAEMON.room and DAEMON.world) then return false, "no world" end
        local room = DAEMON.room.from_data(data)
        if not room then return false, "the room data is not loadable" end

        -- Characters standing in the old Room object have to move to the new
        -- one, or they are in a room the world no longer holds. `register_room`
        -- replaces by id; it does not carry anybody across.
        local old = DAEMON.world.get_room(data.id)
        if old and old.get_characters then
            local here = old:get_characters()
            DAEMON.world.register_room(room)
            for _, char_id in ipairs(here) do room:add_character(char_id) end
        else
            DAEMON.world.register_room(room)
        end
        return true

    elseif kind == "item" then
        if not DAEMON.items then return false, "no item daemon" end
        local item, err = components.build(data)
        if not item then return false, err end
        DAEMON.items.register(item)
        return true

    elseif kind == "mob" then
        if not DAEMON.mobs then return false, "no mob daemon" end
        DAEMON.mobs.register(data)
        return true
    end
    return false, "no such kind"
end

-- ─── Creating ────────────────────────────────────────────────────────────────

--- A fresh datum of a kind, from the schema's defaults.
--- @param session_id string
--- @param kind string
--- @param id string
--- @param base string|nil  a component, for `olc new item x from weapon`
--- @return table|nil draft, string|nil err
function M.create(session_id, kind, id, base)
    if not schema.of(kind) then return nil, "no such kind '" .. tostring(kind) .. "'" end

    local data = schema.defaults(kind, base)
    data.id = id

    local draft = DAEMON.olc.put_draft(session_id, kind, id, data)
    DAEMON.olc.touch(session_id, kind, id)

    local ok, err = M.apply_live(kind, draft)
    if not ok then return nil, err end
    return draft
end

-- ─── Saving ──────────────────────────────────────────────────────────────────

--- Everything of a kind in an area: the drafts, merged over what is on disk.
---
--- Merged rather than replaced, because a session only holds drafts for what it
--- has actually touched. Writing just those would delete every room the builder
--- did not open this evening.
--- @param session_id string
--- @param area_name string
--- @param kind string
--- @return table  array of data
function M.merged(session_id, area_name, kind)
    local file
    for _, spec in ipairs(DAEMON.codegen.GENERATED) do
        if spec.kind == kind then file = spec.file end
    end

    local on_disk = (file and DAEMON.codegen.read(area_name, file)) or {}
    local by_id, order = {}, {}
    for _, data in ipairs(on_disk) do
        if type(data) == "table" and data.id then
            by_id[data.id] = data
            order[#order + 1] = data.id
        end
    end

    for _, data in ipairs(DAEMON.olc.drafts_of(session_id, kind)) do
        if data.id then
            if not by_id[data.id] then order[#order + 1] = data.id end
            by_id[data.id] = data
        end
    end

    -- Sorted, so a file's record order does not depend on the order somebody
    -- happened to edit them in — the same reasoning as `serialize`'s key order.
    table.sort(order)
    local out = {}
    for _, id in ipairs(order) do out[#out + 1] = by_id[id] end
    return out
end

--- Write every changed kind for an area.
--- @param session_id string
--- @param area_name string
--- @return table  array of { file, ok, err }
function M.save(session_id, area_name)
    local results = {}
    for _, spec in ipairs(DAEMON.codegen.GENERATED) do
        local list = M.merged(session_id, area_name, spec.kind)
        -- An empty file is not written. A brand-new area has no items and no
        -- mobs, and an `items.lua` holding `{}` is a file somebody has to
        -- wonder about.
        if #list > 0 then
            local ok, err = DAEMON.codegen.write_kind(area_name, spec.kind, list)
            results[#results + 1] = { file = spec.file, ok = ok, err = err, count = #list }
        end
    end
    return results
end

-- ─── Describing, for the builder ─────────────────────────────────────────────

--- Every field of a datum, with its value, marked for editability.
--- @param kind string
--- @param data table
--- @return table  array of display lines
function M.show(kind, data)
    local lines = {}
    local last_component = nil

    for _, f in ipairs(schema.fields_for(kind, data)) do
        if f.component and f.component ~= last_component then
            last_component = f.component
            lines[#lines + 1] = "  {cyan}── " .. f.component .. " ──{/}"
        end

        local value = data[f.name]
        local mark = (f.editable == false) and "{dim}#{/}" or " "
        local shown = schema.render(f, value)
        if value == nil then shown = "{dim}" .. shown .. "{/}" end

        lines[#lines + 1] = string.format("  %s %-16s %s", mark, f.name, shown)
    end

    local unknown = schema.unknown(kind, data)
    if #unknown > 0 then
        lines[#lines + 1] = "  {yellow}Not in the schema, kept as-is: "
            .. table.concat(unknown, ", ") .. "{/}"
    end

    local lossy = schema.lossy(kind, data)
    if #lossy > 0 then
        local names = {}
        for _, l in ipairs(lossy) do names[#names + 1] = l.path .. " (" .. l.why .. ")" end
        lines[#lines + 1] = "  {red}Would not survive a save, move to custom.lua: "
            .. table.concat(names, ", ") .. "{/}"
    end

    return lines
end

--- What `olc fields` prints: everything that *could* exist, with its type.
--- @param kind string
--- @param base string|nil
--- @return table  array of display lines
function M.fields(kind, base)
    local seed = base and { components = { base } } or {}
    local lines = {}
    local last_component = nil

    for _, f in ipairs(schema.fields_for(kind, seed)) do
        if f.component and f.component ~= last_component then
            last_component = f.component
            lines[#lines + 1] = "  {cyan}── " .. f.component .. " ──{/}"
        end
        local mark = (f.editable == false) and "{dim}#{/}" or "·"
        lines[#lines + 1] = string.format("  %s %-16s {yellow}%-13s{/} %s",
            mark, f.name, f.type or "string", f.help or "")
    end
    lines[#lines + 1] = "  {dim}· editable   # hand-code only (see custom.lua){/}"
    return lines
end

return M
