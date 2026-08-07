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
local proto      = require('lib.prototype')

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

--- The *authored* form of something: what the file says, not what the world has.
---
--- For anything with a prototype these are different, and only the first can be
--- written back. Flattening cannot be run backwards: subtracting the prototype
--- from a live object cannot tell "inherited" from "deliberately set to the same
--- value", and those mean opposite things when the prototype later moves.
---
--- Seeding a draft from `live_data` instead is not a small bug. The live object
--- is the **flattened** template, so the draft would hold every inherited value
--- and the first `olc save` would write them all out — destroying the entire
--- point of the feature on the first edit.
---
--- This revises what `olc_d`'s header documents ("reading the live object rather
--- than the file is deliberate"), and it is worth being precise about which half
--- is revised. That argument is about *values* — a builder must see what is
--- actually in front of them — and it survives intact: `M.show` still prints
--- effective values. What changes is what the draft *stores*.
---
--- It also fixes a defect that predates prototypes: `custom.lua` patches are
--- applied before registration, so they were in the live object, so they were in
--- the draft, so `olc save` copied them into the generated file and they existed
--- twice. Only function-valued patches escaped, because `schema.lossy` caught
--- those.
--- @param session_id string
--- @param kind string
--- @param id string
--- @return table|nil data
function M.authored_data(session_id, kind, id)
    local state = DAEMON.olc and DAEMON.olc.get_state(session_id)
    local area  = state and state.area_name
    if not area then return M.live_data(kind, id) end

    local file
    for _, spec in ipairs(DAEMON.codegen.GENERATED) do
        if spec.kind == kind then file = spec.file end
    end

    local on_disk = file and DAEMON.codegen.read(area, file)
    for _, data in ipairs(on_disk or {}) do
        if type(data) == "table" and data.id == id then
            local copy = {}
            for k, v in pairs(data) do copy[k] = v end
            return copy
        end
    end

    -- Not on disk yet: a room `dig` just made, or an area that has never been
    -- saved. The live object is the only form there is, and it has no prototype
    -- to have been flattened from.
    return M.live_data(kind, id)
end

--- The draft a session is editing, loaded on first touch.
--- @param session_id string
--- @param kind string
--- @param id string
--- @return table|nil draft, string|nil err
function M.draft(session_id, kind, id)
    local existing = DAEMON.olc.draft(session_id, kind, id)
    if existing then return existing end

    local data = M.authored_data(session_id, kind, id)
    if not data then return nil, "no " .. kind .. " '" .. tostring(id) .. "'" end
    return DAEMON.olc.put_draft(session_id, kind, id, data)
end

--- What a draft would become once its prototype chain is folded in.
---
--- What `olc show` prints and what `apply_live` builds from. The draft itself
--- stays the override set — see `authored_data` for why that distinction is the
--- feature.
--- @param kind string
--- @param data table
--- @return table
function M.effective(kind, data)
    if type(data) ~= "table" or data.prototype == nil then return data end
    local merged = proto.resolve(kind, data)
    return type(merged) == "table" and merged or data
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
    -- The world gets the *resolved* form, because that is what `areaload` would
    -- register. A draft holds only the overrides, and registering those would
    -- give the builder a creature with none of its prototype's stat block —
    -- which looks exactly like the prototype not working.
    data = M.effective(kind, data)

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

--- Which prototype `from <base>` means, if it means one at all.
---
--- Precedence, and it is chosen so nothing that worked before can change:
---
---   `from weapon`        a component first, then a prototype of that name
---   `from comp:weapon`   always a component
---   `from proto:beast`   always a prototype
---
--- `from` already meant "a component" and is already a reserved OLC keyword for
--- it. Silently preferring a prototype would change what an existing typed
--- command does, which is the one thing a grammar may never do.
--- @param kind string
--- @param base string|nil
--- @return string|nil prototype_id
function M.base_prototype(kind, base)
    if type(base) ~= "string" or base == "" then return nil end

    local explicit = base:match("^proto:(.+)$")
    if explicit then return explicit end
    if base:match("^comp:") then return nil end

    for _, name in ipairs(components.names()) do
        if name == base then return nil end
    end

    local ok, protos = pcall(require, 'prototypes')
    if ok and protos and protos.get(kind, base) then return base end
    return nil
end

--- The component half of the same question.
--- @param base string|nil
--- @return string|nil component_name
function M.base_component(base)
    if type(base) ~= "string" or base == "" then return nil end
    if base:match("^proto:") then return nil end
    return base:match("^comp:(.+)$") or base
end

--- A fresh datum of a kind, from the schema's defaults.
--- @param session_id string
--- @param kind string
--- @param id string
--- @param base string|nil  a component, for `olc new item x from weapon`, or a
---                         prototype id, for `olc new mob x from proto:beast`
--- @return table|nil draft, string|nil err
function M.create(session_id, kind, id, base)
    if not schema.of(kind) then return nil, "no such kind '" .. tostring(kind) .. "'" end

    local data
    local prototype = M.base_prototype(kind, base)
    if prototype then
        -- **Nothing is copied.** A new record from a prototype is two keys, and
        -- that is the whole point: the moment `create` seeded it with the
        -- prototype's values, the first save would write them all back out and
        -- the child would stop tracking its parent for ever.
        data = { prototype = prototype }
    else
        data = schema.defaults(kind, base)
    end
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

-- ─── Prototypes ──────────────────────────────────────────────────────────────

--- Drop everything in a draft that only restates its prototype.
---
--- The safe form of subtraction — a human asked for it and sees the result.
--- `olc save` must never do this on its own: a builder who deliberately sets a
--- value equal to the inherited one means "this is mine now, and it must not
--- move when the prototype moves", and subtracting deletes that intent with
--- nothing in the diff to show for it.
--- @return table removed, string|nil err
function M.thin(session_id, kind, id)
    local draft, err = M.draft(session_id, kind, id)
    if not draft then return {}, err end
    if draft.prototype == nil then
        return {}, "'" .. tostring(id) .. "' has no prototype, so nothing restates one"
    end

    local removed = proto.thin(kind, draft)
    if #removed > 0 then
        DAEMON.olc.touch(session_id, kind, id)
        M.apply_live(kind, draft)
    end
    return removed
end

--- Every prototype of a kind, with its parent and how many records name it.
---
--- The use count is over what is *registered*, which is the resolved world — so
--- it answers "what would I break", which is the question somebody about to edit
--- a prototype is actually asking.
--- @param kind string
--- @return table  array of { id, parent, uses }
function M.protos(kind)
    local ok, protos = pcall(require, 'prototypes')
    if not ok or not protos then return {} end

    local uses = {}
    local registered = {}
    if kind == "mob" and DAEMON.mobs then
        for _, id in ipairs(DAEMON.mobs.all()) do registered[#registered + 1] = DAEMON.mobs.get(id) end
    elseif kind == "item" and DAEMON.items then
        for _, id in ipairs(DAEMON.items.all()) do registered[#registered + 1] = DAEMON.items.get(id) end
    elseif kind == "room" and DAEMON.world then
        for _, room in ipairs(DAEMON.world.all_rooms and DAEMON.world.all_rooms() or {}) do
            registered[#registered + 1] = room
        end
    end
    for _, r in ipairs(registered) do
        local p = type(r) == "table" and r.prototype
        if type(p) == "string" then uses[p] = (uses[p] or 0) + 1 end
    end

    local out = {}
    for _, id in ipairs(protos.ids(kind)) do
        local data = protos.get(kind, id)
        out[#out + 1] = { id = id, parent = data and data.prototype, uses = uses[id] or 0 }
    end
    return out
end

-- ─── Describing, for the builder ─────────────────────────────────────────────

--- Every field of a datum, with its value, marked for editability and origin.
---
--- **Effective values, override storage.** The draft holds only what differs
--- from the prototype, and a builder who could not see the rest would be editing
--- blind — so what is printed is the resolved form, with a mark saying where
--- each value came from.
--- @param kind string
--- @param data table
--- @return table  array of display lines
function M.show(kind, data)
    local lines = {}
    local last_component = nil

    local effective = M.effective(kind, data)
    local inherited = 0
    local total     = 0

    for _, f in ipairs(schema.fields_for(kind, data)) do
        if f.component and f.component ~= last_component then
            last_component = f.component
            lines[#lines + 1] = "  {cyan}── " .. f.component .. " ──{/}"
        end

        local value = effective[f.name]
        local mark, source = " ", nil
        if f.editable == false then
            mark = "{dim}#{/}"
        elseif data.prototype ~= nil then
            local origin, from = proto.origin(kind, data, f.name)
            if origin == "inherited" then
                mark, source = "{dim}~{/}", from
                inherited = inherited + 1
                total = total + 1
            elseif origin == "struck" then
                mark = "{red}-{/}"
            elseif origin == "self" then
                total = total + 1
            end
        end

        local shown
        if mark == "{red}-{/}" then
            shown = "{red}(removed here){/}"
        else
            shown = schema.render(f, value)
            if value == nil then shown = "{dim}" .. shown .. "{/}" end
            if source then shown = "{dim}" .. shown .. "   [" .. source .. "]{/}" end
        end

        lines[#lines + 1] = string.format("  %s %-16s %s", mark, f.name, shown)
    end

    if data.prototype ~= nil then
        local chain = proto.chain(kind, data.prototype, data.id)
        local names = {}
        for i = #(chain or {}), 1, -1 do names[#names + 1] = chain[i].id end
        lines[#lines + 1] = ""
        if #names > 0 then
            lines[#lines + 1] = "  {dim}prototype " .. table.concat(names, " -> ") .. "{/}"
        else
            lines[#lines + 1] = "  {red}prototype " .. tostring(data.prototype)
                .. " does not resolve.{/}"
        end
        lines[#lines + 1] = "  {dim}· set here   ~ inherited   - struck   # hand-code only{/}"
        if inherited > 0 then
            lines[#lines + 1] = "  {dim}" .. inherited .. " of " .. total
                .. " values are inherited. `olc thin` drops what only restates them.{/}"
        end
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
