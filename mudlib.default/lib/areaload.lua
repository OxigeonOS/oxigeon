-- mudlib/lib/areaload.lua — Finding areas, and loading them in the right order.
--
-- `game/init.lua` used to name every area explicitly: a `pcall` block per area,
-- each requiring its rooms, items, mobs and shops by hand and calling
-- `register_area_source` afterwards. An area OLC created was therefore invisible
-- until somebody edited that file — and `olc` never called
-- `register_area_source` at all, so `areas reset <new_area>` answered "No
-- registered source" for every area it had ever made.
--
-- Both stop being possible here. Discovery finds the area; `M.load` registers
-- the reset spec as its last act, so a working `areas reset` is not something
-- anybody has to remember.
--
-- ─── A lib, not a daemon ─────────────────────────────────────────────────────
--
-- It holds no state, and `world_d` calls it during a reset. A daemon that
-- `world_d` depends on and that depends on `world_d` is a load-order problem
-- waiting to happen.
--
-- ─── The five entry files ────────────────────────────────────────────────────
--
--   rooms.lua | init.lua   the rooms. `init.lua` wins, for an area assembled
--                          from several files with `ROOM_D.merge` — thornhollow
--                          does exactly that and is unchanged by any of this.
--   items.lua              item templates
--   mobs.lua               creature templates
--   shops.lua              shops
--   custom.lua             hand-written patches
--
-- Anything else in the directory is *included by one of those*. That one
-- sentence is why `legacy_rooms.lua` — which `olc adopt` writes — needs no
-- special case, and why `_drafts/` is skipped without a rule.

local patch = require('lib.patch')
local proto = require('lib.prototype')

local M = {}

--- Directories under `areas/` that are not areas.
local SKIP = { _drafts = true }

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

-- ─── Discovery ───────────────────────────────────────────────────────────────

--- Every area directory, sorted.
---
--- Both roots, because `list_dir` searches both and a mudlib shipping a starter
--- area is a thing that should work.
--- @return table  array of area names
function M.discover()
    if type(list_dir) ~= "function" then return {} end

    local ok, entries = pcall(list_dir, "areas")
    if not ok or type(entries) ~= "table" then return {} end

    local out = {}
    for _, e in ipairs(entries) do
        if type(e) == "table" and e.is_dir and e.name and not SKIP[e.name] then
            out[#out + 1] = e.name
        end
    end
    table.sort(out)
    return out
end

--- What an area is made of, without loading it.
--- @param area_name string
--- @return table|nil {
---   name, meta, managed, entry, has = {...}, modules = { "areas.x.rooms", ... } }
function M.inspect(area_name)
    if type(list_dir) ~= "function" then return nil end

    local ok, entries = pcall(list_dir, "areas/" .. area_name)
    if not ok or type(entries) ~= "table" then return nil end

    local files, modules = {}, {}
    for _, e in ipairs(entries) do
        if type(e) == "table" and not e.is_dir and e.name then
            local stem = e.name:match("^(.+)%.lua$")
            if stem then
                files[stem] = true
                modules[#modules + 1] = "areas." .. area_name .. "." .. stem
            end
        end
    end
    table.sort(modules)

    local meta
    if DAEMON and DAEMON.codegen then
        meta = DAEMON.codegen.read(area_name, "_meta")
    end

    return {
        name    = area_name,
        meta    = type(meta) == "table" and meta or nil,
        managed = type(meta) == "table" and meta.managed or nil,
        -- `init.lua` wins: an area assembled from several room files needs
        -- somewhere to do the assembling, and that is what thornhollow's does.
        entry   = files.init and "init" or (files.rooms and "rooms" or nil),
        has     = files,
        modules = modules,
    }
end

-- ─── Loading ─────────────────────────────────────────────────────────────────

--- Re-read the prototype library before every load.
---
--- This is what makes "editing a prototype takes effect on area reload" a
--- property of the system rather than something each reload path has to
--- remember — and it is why `world_d.reset_area`, which calls `M.load`, needed
--- no change at all to pick prototype edits up.
local function flush_prototypes()
    local ok, protos = pcall(require, 'prototypes')
    if ok and protos and protos.flush_cache then
        local fok, err = pcall(protos.flush_cache)
        if not fok then log_error("AREALOAD: prototype flush failed: " .. tostring(err)) end
    end
end

local function fresh_require(module_path)
    package.loaded[module_path] = nil
    local ok, value = pcall(require, module_path)
    if not ok then return nil, tostring(value) end
    return value
end

local function require_area(area_name, file)
    local ok, value = pcall(require, "areas." .. area_name .. "." .. file)
    if not ok then return nil, tostring(value) end
    return value
end

--- The `custom.lua` patches for an area, or nil.
local function custom_of(area_name, spec)
    if not (spec and spec.has and spec.has.custom) then return nil end
    local custom, err = require_area(area_name, "custom")
    if err then
        log_error("AREALOAD: " .. area_name .. "/custom.lua failed to load: " .. err)
        return nil
    end
    return type(custom) == "table" and custom or nil
end

--- Register item templates.
local function load_items(area_name, spec, custom)
    if not (spec.has.items and DAEMON and DAEMON.items) then return end

    local list, err = require_area(area_name, "items")
    if err then return log_error("AREALOAD: " .. area_name .. "/items.lua: " .. err) end
    if type(list) ~= "table" then return end

    -- Prototype first, `custom.lua` second, both before construction. Before
    -- construction so a patched `damage` reaches `weapon.from_data` rather than
    -- an already-built component; prototype first because `components` has to be
    -- resolved before the patch merge can know that `resist` is a map, and
    -- because a strike the patch does not mention has to be *consumed* rather
    -- than left sitting in the datum as an uninterpreted "@none".
    proto.resolve_list("item", list)
    patch.apply("item", list, patch.for_kind(custom, "item"))
    DAEMON.items.register_all(list)
end

--- Build and register an area's rooms.
local function load_rooms(area_name, spec, custom)
    if not (spec.entry and DAEMON and DAEMON.room and DAEMON.world) then return end

    local data, err = require_area(area_name, spec.entry)
    if err then return log_error("AREALOAD: " .. area_name .. "/" .. spec.entry .. ".lua: " .. err) end
    if type(data) ~= "table" then return end

    -- `_meta` is a string key, so `resolve_list`'s ipairs walk steps over it.
    proto.resolve_list("room", data)
    patch.apply("room", data, patch.for_kind(custom, "room"))

    -- A generated `rooms.lua` never carries an inline `_meta`; a hand-authored
    -- one may. Injecting it here means there is one source of area metadata per
    -- area whichever shape the area is, and `ROOM_D.load_area` keeps taking it
    -- the way it always has.
    if spec.meta and data._meta == nil then
        data._meta = spec.meta
    end

    local rooms = DAEMON.room.load_area(data)
    DAEMON.world.register_area(rooms)
end

local function load_mobs(area_name, spec, custom)
    if not (spec.has.mobs and DAEMON and DAEMON.mobs) then return end

    local list, err = require_area(area_name, "mobs")
    if err then return log_error("AREALOAD: " .. area_name .. "/mobs.lua: " .. err) end
    if type(list) ~= "table" then return end

    proto.resolve_list("mob", list)
    patch.apply("mob", list, patch.for_kind(custom, "mob"))
    DAEMON.mobs.register_all(list)
end

local function load_shops(area_name, spec)
    if not (spec.has.shops and DAEMON and DAEMON.shop) then return end

    local list, err = require_area(area_name, "shops")
    if err then return log_error("AREALOAD: " .. area_name .. "/shops.lua: " .. err) end
    if type(list) == "table" then DAEMON.shop.register_all(list) end
end

--- Register how this area is rebuilt, so `areas reset` works.
---
--- The last act of every load, which is what makes a working reset structural
--- rather than something each caller has to remember. `olc` forgot, so every
--- area it created answered "No registered source" for ever.
local function register_spec(area_name, spec)
    if not (DAEMON and DAEMON.world and DAEMON.world.register_area_spec) then return end
    DAEMON.world.register_area_spec(area_name, {
        modules = spec.modules,
        load    = function(name) return M.load(name) end,
    })
end

--- Load one area completely.
---
--- Used by `load_all` and by `world_d.reset_area`. Every pass runs for this one
--- area, which is right for a reset: by then every *other* area already exists.
--- @param area_name string
--- @return boolean ok, string|nil err
function M.load(area_name)
    flush_prototypes()

    local spec = M.inspect(area_name)
    if not spec then return false, "no such area directory" end
    if not spec.entry then
        return false, "no rooms.lua or init.lua"
    end

    local custom = custom_of(area_name, spec)

    local ok, err = pcall(function()
        load_items(area_name, spec, custom)
        load_rooms(area_name, spec, custom)
        load_mobs(area_name, spec, custom)
        load_shops(area_name, spec)
    end)
    if not ok then return false, tostring(err) end

    -- Last, and idempotent: it runs again on every reset, so anything it does
    -- has to be safe to do twice.
    if custom and type(custom.on_load) == "function" then
        local hook_ok, hook_err = pcall(custom.on_load, area_name)
        if not hook_ok then
            log_error("AREALOAD: " .. area_name .. "/custom.lua on_load raised: "
                .. tostring(hook_err))
        end
    end

    register_spec(area_name, spec)
    return true
end

--- Load every discovered area, in passes across all of them.
---
--- Passes rather than area-by-area, and that removes a whole class of hazard at
--- once: an exit from one area into another, a mob whose `spawn_room` is
--- elsewhere, a shop standing in a room another area declares. The tree already
--- has one — `thornhollow.smithy` has a `down` exit into `collapsed_mine.adit`
--- — which worked only because `game/init.lua` happened to list them in the
--- right order.
---
--- Mobs are registered but **not populated**: the caller does that once, at the
--- end, when every room in every area exists to stand in.
--- @return number loaded, table failures  failures = { { area, err }, ... }
function M.load_all()
    flush_prototypes()

    local names = M.discover()
    local specs, customs, failures = {}, {}, {}

    for _, name in ipairs(names) do
        local spec = M.inspect(name)
        if not spec then
            failures[#failures + 1] = { area = name, err = "could not be inspected" }
        elseif not spec.entry then
            failures[#failures + 1] = { area = name, err = "no rooms.lua or init.lua" }
        else
            specs[name] = spec
            customs[name] = custom_of(name, spec)
        end
    end

    local function pass(what, fn)
        for _, name in ipairs(names) do
            local spec = specs[name]
            if spec then
                local ok, err = pcall(fn, name, spec, customs[name])
                if not ok then
                    failures[#failures + 1] = { area = name, err = what .. ": " .. tostring(err) }
                    specs[name] = nil
                end
            end
        end
    end

    pass("items", load_items)
    pass("rooms", load_rooms)
    pass("mobs",  load_mobs)
    pass("shops", function(name, spec) load_shops(name, spec) end)

    -- Content hooks after every area's data, for the same reason as the passes:
    -- thornhollow's vault chest is spawned into a room, and the room has to be
    -- there whichever order the areas were discovered in.
    for _, name in ipairs(names) do
        local custom = customs[name]
        if specs[name] and custom and type(custom.on_load) == "function" then
            local ok, err = pcall(custom.on_load, name)
            if not ok then
                log_error("AREALOAD: " .. name .. "/custom.lua on_load raised: " .. tostring(err))
            end
        end
    end

    local loaded = 0
    for _, name in ipairs(names) do
        if specs[name] then
            register_spec(name, specs[name])
            loaded = loaded + 1
        end
    end

    return loaded, failures
end

--- Purge an area's modules from the require cache.
--- @param modules table  array of module paths
function M.purge(modules)
    for _, path in ipairs(modules or {}) do
        package.loaded[path] = nil
    end
end

return M
