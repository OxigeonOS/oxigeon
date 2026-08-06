-- mudlib/daemons/adopt_d.lua — Bringing a hand-authored area under OLC.
--
-- OLC regenerates `rooms.lua`, `items.lua` and `mobs.lua` wholesale, which is
-- only safe for an area whose data is data. A hand-authored one is not:
-- thornhollow's square carries two inline action functions, greywater_marsh's
-- descriptions are lfuns keyed on the weather. Regenerating either would delete
-- them, and the file would still compile.
--
-- So `_meta.managed` gates every OLC write, and this is the only thing that sets
-- it. In two steps, deliberately:
--
--   olc adopt <area>            report what would change. Writes nothing.
--   olc adopt <area> --confirm  do it.
--
-- ─── No Lua source is ever parsed ────────────────────────────────────────────
--
-- The obvious implementation lifts each function body out of the old file and
-- into the new `custom.lua`. That is a *source transformation*, it would fail
-- subtly rather than loudly, and the failure would be somebody's room action
-- quietly not working.
--
-- Instead the original file is copied beside the new one as `legacy_<name>.lua`
-- and the generated `custom.lua` **references** it. Mechanical, lossless, and it
-- leaves an obvious tidying job with a stated end condition. Nothing is deleted,
-- ever.

local schema     = require('lib.schema')
local components = require('components')
local serialize  = require('lib.serialize')

local M = {}

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

-- ─── Reading what is there ───────────────────────────────────────────────────

--- The area's current content, in *authoring* form.
---
--- Through `require` rather than `load(read_file(...))`: `require` keeps chunk
--- names, so a compile error still names the file and the debugger and `reload`
--- keep working. Items come back as built `Item`s, so `components.to_data` is
--- what gets them back to a form that can be written — the only direction that
--- exists, and why every component has a `to_data`.
--- @param area_name string
--- @return table|nil { rooms, items, mobs }, string|nil err
function M.read_current(area_name)
    local areaload = require('lib.areaload')
    local spec = areaload.inspect(area_name)
    if not spec then return nil, "no such area" end
    if not spec.entry then return nil, "no rooms.lua or init.lua" end

    local out = { rooms = {}, items = {}, mobs = {}, lossy = {} }

    local function load_one(file, into)
        if file ~= spec.entry and not spec.has[file] then return end
        local ok, value = pcall(require, "areas." .. area_name .. "." .. file)
        if not ok then
            log_error("ADOPT: " .. area_name .. "/" .. file .. ".lua: " .. tostring(value))
            return
        end
        if type(value) == "table" then out[into] = value end
    end

    load_one(spec.entry, "rooms")
    load_one("items", "items")
    load_one("mobs", "mobs")

    -- A hand-authored `rooms.lua` may carry an inline `_meta`, which is area
    -- metadata rather than a room. `room_d.load_area` already knows that; this
    -- has to as well, or `_meta` is reported as a room with no id.
    if out.rooms._meta then
        out.meta = out.rooms._meta
        out.rooms._meta = nil
    end

    return out
end

--- Items are built objects; everything else is already flat.
---
--- Always a **copy**. `require` caches the module, so mutating what it returned
--- would edit the live area — and the next step deletes every field that is
--- moving to `custom.lua`.
--- @param kind string
--- @param entry table
--- @return table data, table lossy
local function to_authoring(kind, entry)
    local data, lossy

    if kind == "item" then
        data, lossy = components.to_data(entry)
        data.id = entry.id
        for _, f in ipairs(schema.fields_for("item", data)) do
            if data[f.name] == nil and entry[f.name] ~= nil and f.name ~= "components" then
                data[f.name] = entry[f.name]
            end
        end
        for _, l in ipairs(schema.lossy("item", entry)) do lossy[#lossy + 1] = l end
    else
        data = {}
        for k, v in pairs(entry) do data[k] = v end
        lossy = schema.lossy(kind, entry)
    end

    -- **Remove what is moving.** Classifying a field as lossy and then writing
    -- it anyway is the worst of both: the data file would hold a function, so
    -- the write is refused and the adoption stops halfway with the copies
    -- already made. The value is not lost — it is in `legacy_*.lua`, which
    -- `custom.lua` references.
    for _, l in ipairs(lossy) do
        data[l.path:match("^([^%.]+)")] = nil
    end

    return data, lossy
end

-- ─── The dry run ─────────────────────────────────────────────────────────────

--- What adopting this area would change.
---
--- Three outcomes per field, and the third is the one that matters:
---
---   kept     in a schema, writable
---   lossy    a function, or a field the kind lists as hand-written. Moves.
---   unknown  named by no schema but writable. **Kept verbatim** and reported —
---            dropping it silently is the bug class this whole design ends.
--- @param area_name string
--- @return table|nil plan, string|nil err
function M.plan(area_name)
    local managed = DAEMON.codegen.is_managed(area_name)
    if managed then
        return nil, "'" .. area_name .. "' is already OLC-managed."
    end

    local current, err = M.read_current(area_name)
    if not current then return nil, err end

    local plan = {
        area    = area_name,
        meta    = current.meta,
        kinds   = {},
        lossy   = {},
        unknown = {},
        legacy  = {},
    }

    for _, spec in ipairs(DAEMON.codegen.GENERATED) do
        local entries = current[spec.file] or {}
        local converted = {}

        for _, entry in ipairs(entries) do
            if type(entry) == "table" and entry.id then
                local data, lossy = to_authoring(spec.kind, entry)
                converted[#converted + 1] = data

                for _, l in ipairs(lossy) do
                    plan.lossy[#plan.lossy + 1] = {
                        kind = spec.kind, id = entry.id, path = l.path, why = l.why,
                    }
                end
                for _, path in ipairs(schema.unknown(spec.kind, data)) do
                    plan.unknown[#plan.unknown + 1] = {
                        kind = spec.kind, id = entry.id, path = path,
                    }
                end
            end
        end

        plan.kinds[spec.kind] = converted
        if #converted > 0 then
            plan.legacy[#plan.legacy + 1] = spec.file
        end
    end

    -- Anything in the directory that is not one of the five entry names. Its
    -- content has already been adopted (its entry file required it), but the
    -- file itself will be left behind, unloaded, and that is worth saying.
    local areaload = require('lib.areaload')
    local spec = areaload.inspect(area_name)
    local entries = { rooms = true, init = true, items = true, mobs = true,
                      shops = true, custom = true, _meta = true }
    plan.strays = {}
    for name in pairs(spec.has or {}) do
        if not entries[name] then plan.strays[#plan.strays + 1] = name end
    end
    table.sort(plan.strays)

    table.sort(plan.lossy, function(a, b)
        if a.kind ~= b.kind then return a.kind < b.kind end
        if a.id ~= b.id then return a.id < b.id end
        return a.path < b.path
    end)
    table.sort(plan.unknown, function(a, b)
        if a.kind ~= b.kind then return a.kind < b.kind end
        if a.id ~= b.id then return a.id < b.id end
        return a.path < b.path
    end)

    return plan
end

-- ─── Doing it ────────────────────────────────────────────────────────────────

--- The `custom.lua` an adoption writes.
---
--- It references the legacy copies rather than containing the function source,
--- because extracting a function body from one file into another is a source
--- transformation and would fail silently. Mechanical and lossless, and the
--- header says what "done" looks like so the reference is not permanent.
local function custom_source(plan)
    local lines = {
        "-- game/areas/" .. plan.area .. "/custom.lua — written once by `olc adopt`.",
        "--",
        "-- From here on this file is yours: OLC never reads or writes it.",
        "--",
        "-- Adoption moved the behaviour OLC cannot author out of the generated",
        "-- files and left the originals beside them as legacy_*.lua. Nothing was",
        "-- deleted. When you have moved what you want to keep into this file",
        "-- properly, delete the legacy files and the requires below.",
        "",
    }

    local needed = {}
    for _, l in ipairs(plan.lossy) do
        for _, spec in ipairs(DAEMON.codegen.GENERATED) do
            if spec.kind == l.kind then needed[spec.file] = true end
        end
    end

    local files = {}
    for file in pairs(needed) do files[#files + 1] = file end
    table.sort(files)

    for _, file in ipairs(files) do
        lines[#lines + 1] = "local legacy_" .. file .. " = require('areas."
            .. plan.area .. ".legacy_" .. file .. "')"
    end
    lines[#lines + 1] = ""
    lines[#lines + 1] = "local function by_id(list, id)"
    lines[#lines + 1] = "    for _, d in ipairs(list) do if d.id == id then return d end end"
    lines[#lines + 1] = "    return {}"
    lines[#lines + 1] = "end"
    lines[#lines + 1] = ""
    lines[#lines + 1] = "return {"

    for _, spec in ipairs(DAEMON.codegen.GENERATED) do
        local by_id = {}
        local ids = {}
        for _, l in ipairs(plan.lossy) do
            if l.kind == spec.kind then
                if not by_id[l.id] then by_id[l.id] = {} ; ids[#ids + 1] = l.id end
                by_id[l.id][#by_id[l.id] + 1] = l.path
            end
        end
        if #ids > 0 then
            table.sort(ids)
            lines[#lines + 1] = "    " .. spec.file .. " = {"
            for _, id in ipairs(ids) do
                lines[#lines + 1] = "        [" .. serialize.quote(id) .. "] = {"
                local paths = by_id[id]
                table.sort(paths)
                for _, path in ipairs(paths) do
                    -- A dotted path is inside a component and cannot be patched
                    -- as a top-level key; the whole component field moves.
                    local key = path:match("^([^%.]+)")
                    lines[#lines + 1] = "            " .. key .. " = by_id(legacy_"
                        .. spec.file .. ", " .. serialize.quote(id) .. ")." .. key .. ","
                end
                lines[#lines + 1] = "        },"
            end
            lines[#lines + 1] = "    },"
        end
    end

    lines[#lines + 1] = "}"
    lines[#lines + 1] = ""
    return table.concat(lines, "\n")
end

--- Adopt the area. Order matters; see the comments.
--- @param area_name string
--- @return table lines, boolean ok
function M.confirm(area_name)
    local plan, err = M.plan(area_name)
    if not plan then return { "{red}" .. tostring(err) .. "{/}" }, false end

    local out = {}
    local function say(text) out[#out + 1] = text end

    -- 1. Copy every entry file aside, BEFORE anything is overwritten: writing
    --    `rooms.lua` destroys the source of the very functions `custom.lua` is
    --    about to reference. Refuse rather than overwrite — a `legacy_` file
    --    that already exists is a previous adoption somebody may be mid-way
    --    through tidying.
    for _, file in ipairs(plan.legacy) do
        local target = "legacy_" .. file
        if DAEMON.codegen.read(area_name, target) ~= nil then
            say("{red}" .. target .. ".lua already exists. A previous adoption "
                .. "left it; move it aside first.{/}")
            return out, false
        end
        local source = read_file(DAEMON.codegen.path(area_name, file))
        if not source then
            say("{red}Could not read " .. file .. ".lua.{/}")
            return out, false
        end
        local ok, werr = write_file(DAEMON.codegen.path(area_name, target), source)
        if not ok then
            say("{red}Could not write " .. target .. ".lua: " .. tostring(werr) .. "{/}")
            return out, false
        end
        -- Compile-check the copy, so a broken one is found now rather than at
        -- the next boot when the original is gone.
        local compiles, cerr = verify_file(DAEMON.codegen.path(area_name, target))
        if not compiles then
            say("{red}" .. target .. ".lua does not compile: " .. tostring(cerr) .. "{/}")
            return out, false
        end
        say("  copied  " .. file .. ".lua → " .. target .. ".lua")
    end

    -- 2. `custom.lua`, and never over an existing one. Hand-written code is the
    --    one thing here that cannot be regenerated.
    if #plan.lossy > 0 then
        if DAEMON.codegen.read(area_name, "custom") ~= nil then
            say("{red}custom.lua already exists. Adoption will not overwrite "
                .. "hand-written code — merge it by hand and re-run.{/}")
            return out, false
        end
        local ok, werr = write_file(DAEMON.codegen.path(area_name, "custom"),
            custom_source(plan))
        if not ok then
            say("{red}Could not write custom.lua: " .. tostring(werr) .. "{/}")
            return out, false
        end
        say("  wrote   custom.lua  (" .. #plan.lossy .. " field(s) moved)")
    end

    -- 3. The data files.
    for _, spec in ipairs(DAEMON.codegen.GENERATED) do
        local list = plan.kinds[spec.kind] or {}
        if #list > 0 then
            local ok, werr = DAEMON.codegen.write_kind(area_name, spec.kind, list)
            if not ok then
                say("{red}Could not write " .. spec.file .. ".lua: " .. tostring(werr) .. "{/}")
                return out, false
            end
            say("  wrote   " .. spec.file .. ".lua  (" .. #list .. " record(s))")
        end
    end

    -- 4. `_meta.lua` **last**, so a failure part-way leaves the area unmanaged
    --    and OLC still refuses to touch it. The gate is the last thing set,
    --    which makes a half-finished adoption safe rather than a trap.
    local meta = plan.meta or {}
    local ok, werr = DAEMON.codegen.write_meta(area_name, {
        title  = meta.title,
        author = meta.author,
        level  = meta.level,
        status = meta.status,
    })
    if not ok then
        say("{red}Could not write _meta.lua: " .. tostring(werr) .. "{/}")
        return out, false
    end
    say("  wrote   _meta.lua  {green}(managed){/}")

    if DAEMON.audit then pcall(DAEMON.audit.log, "olc.adopt", true, area_name) end

    say("")
    say("{green}'" .. area_name .. "' is now OLC-managed.{/}")
    say("  {dim}Nothing was deleted. `areas reset " .. area_name
        .. "` to load what was written.{/}")
    return out, true
end

-- ─── The report ──────────────────────────────────────────────────────────────

--- Run the dry run, or the adoption, and describe it.
--- @param player table
--- @param area_name string
--- @param confirm boolean
--- @return table  array of lines
function M.run(player, area_name, confirm)
    local plan, err = M.plan(area_name)
    if not plan then return { "{red}" .. tostring(err) .. "{/}" } end

    local lines = {
        "{cyan}" .. area_name .. "{/} is not OLC-managed. Adopting it would "
            .. "rewrite its data files.",
        "",
    }

    for _, spec in ipairs(DAEMON.codegen.GENERATED) do
        local n = #(plan.kinds[spec.kind] or {})
        if n > 0 then
            lines[#lines + 1] = string.format("  %-12s %d %s%s",
                spec.file .. ".lua", n, spec.kind, n == 1 and "" or "s")
        end
    end

    if #plan.lossy > 0 then
        lines[#lines + 1] = ""
        lines[#lines + 1] = "{yellow}Moves to custom.lua{/} — OLC cannot author these:"
        for _, l in ipairs(plan.lossy) do
            lines[#lines + 1] = string.format("  %-6s %-28s %-20s %s",
                l.kind, l.id, l.path, l.why)
        end
    end

    if #plan.unknown > 0 then
        lines[#lines + 1] = ""
        lines[#lines + 1] = "{cyan}Named by no schema{/} — kept verbatim, not editable in OLC:"
        for _, u in ipairs(plan.unknown) do
            lines[#lines + 1] = string.format("  %-6s %-28s %s", u.kind, u.id, u.path)
        end
    end

    if #plan.strays > 0 then
        lines[#lines + 1] = ""
        lines[#lines + 1] = "{yellow}Left in place, unloaded{/} — their content has been"
        lines[#lines + 1] = "adopted through the file that requires them:"
        for _, name in ipairs(plan.strays) do
            lines[#lines + 1] = "  " .. name .. ".lua"
        end
    end

    if not confirm then
        lines[#lines + 1] = ""
        lines[#lines + 1] = "{dim}Nothing has been written. Re-run with --confirm to adopt.{/}"
        lines[#lines + 1] = "{dim}Originals are copied to legacy_*.lua; nothing is deleted.{/}"
        return lines
    end

    lines[#lines + 1] = ""
    local done, ok = M.confirm(area_name)
    for _, line in ipairs(done) do lines[#lines + 1] = line end
    if not ok then
        lines[#lines + 1] = "{red}Adoption stopped. The area is still unmanaged.{/}"
    end
    return lines
end

log("debug", "ADOPT_D: daemon loaded")

return M
