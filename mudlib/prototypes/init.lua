-- mudlib/prototypes/init.lua — The index of prototypes.
--
-- Discovery, the same shape as `schema/init.lua` and `components/init.lua`:
-- `list_dir` over this directory, `require` each module, keep what looks like a
-- prototype file. `list_dir` searches both jail roots, so `game/prototypes/`
-- appears here with no central list to keep in step.
--
-- **The mudlib ships no prototypes.** A `beast` is game content and belongs in
-- the game layer; this module tolerates the directory not existing at all, which
-- is the state a fresh mudlib is in.
--
-- ─── The file format ─────────────────────────────────────────────────────────
--
--     -- game/prototypes/beasts.lua — hand-written, like custom.lua, and for the
--     -- same reason: these hold functions. OLC never reads or writes this file.
--     return {
--         mobs = {
--             ["beast"]         = { aggressive = true, tags = { "beast" } },
--             ["beast.crawler"] = { prototype = "beast", name = "crawler" },
--         },
--         items = { ["reagent_potion"] = { weight = 0.5, stackable = true } },
--     }
--
-- Keyed by the plural, matching `custom.lua` and the generated file names, so a
-- builder learns one shape. Keying by kind first is also what keeps a prototype
-- *pure authoring data* — there is no `kind` or `doc` metadata key inside it that
-- could one day collide with a schema field a game layer invents.
--
-- The file name carries nothing; the id does. An area that wants a private
-- prototype calls it `collapsed_mine.crawler`.
--
-- Exposes:
--   prototypes.all()            -> { [kind] = { [id] = data } }
--   prototypes.get(kind, id)    -> data | nil
--   prototypes.ids(kind)        -> sorted array
--   prototypes.find_kind(id, except) -> kind | nil
--   prototypes.problems()       -> array of { kind, id, level, message }
--   prototypes.flush_cache()
--
-- See docs/src/lua-api/prototypes.md.

local M = {}

local _index    = nil
local _problems = nil

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

local function log_warn(message)
    log("warn", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.warn, message) end
end

--- Every authorable kind, singular, with its plural. Read from the schema index
--- rather than listed, so a game layer's fourth kind works with no edit here.
local function kinds()
    local ok, schemas = pcall(require, 'schema')
    if not ok or type(schemas) ~= "table" then return {} end
    local out = {}
    for _, mod in ipairs(schemas.all()) do
        if mod.kind then out[mod.kind .. "s"] = mod.kind end
    end
    return out
end

local function note(problems, kind, id, level, message)
    problems[#problems + 1] = {
        kind = kind, id = id, level = level, message = message,
    }
end

--- Take one module's tables into the index.
local function absorb(index, problems, file, mod)
    local plurals = kinds()

    for plural, bag in pairs(mod) do
        local kind = plurals[plural]
        if type(bag) ~= "table" then
            note(problems, "?", plural, "error",
                "prototypes/" .. file .. ": '" .. tostring(plural) .. "' is not a table")
        elseif not kind then
            note(problems, "?", plural, "error",
                "prototypes/" .. file .. ": '" .. tostring(plural)
                .. "' is not an authorable kind. Expected one of the plural forms "
                .. "of what schema/ declares.")
        else
            index[kind] = index[kind] or {}
            for id, data in pairs(bag) do
                if type(id) ~= "string" or id == "" then
                    note(problems, kind, tostring(id), "error",
                        "prototypes/" .. file .. ": a prototype id must be a non-empty string")
                elseif type(data) ~= "table" then
                    note(problems, kind, id, "error",
                        "prototypes/" .. file .. ": '" .. id .. "' is not a table")
                else
                    if index[kind][id] then
                        note(problems, kind, id, "error",
                            "prototype '" .. id .. "' is declared twice for kind '"
                            .. kind .. "'; prototypes/" .. file .. " wins")
                    end
                    -- A prototype is a template *minus its id*. One that carried
                    -- an `id` would merge it into every child and every one of
                    -- them would register under the same name — which reads, in
                    -- the log, as "the last area loaded ate the others".
                    if data.id ~= nil then
                        note(problems, kind, id, "error",
                            "prototype '" .. id .. "' declares an `id`. A prototype is a "
                            .. "template without one; dropped so it cannot overwrite a child's.")
                        data.id = nil
                    end
                    index[kind][id] = data
                end
            end
        end
    end
end

local function discover()
    if _index then return _index end
    _index, _problems = {}, {}

    if type(list_dir) ~= "function" then
        log_error("PROTOTYPES: list_dir is unavailable; no prototypes loaded")
        return _index
    end

    local ok, entries = pcall(list_dir, "prototypes")
    if not ok or type(entries) ~= "table" then
        -- Not an error. A mudlib with no `prototypes/` directory anywhere is the
        -- ordinary state of a fresh install, and `schema/init.lua` tolerates the
        -- same thing for the same reason.
        return _index
    end

    local files = {}
    for _, entry in ipairs(entries) do
        local file   = type(entry) == "table" and entry.name
        local is_dir = type(entry) == "table" and entry.is_dir
        local name   = (not is_dir) and file and file:match("^(.+)%.lua$")
        if name and name ~= "init" then files[#files + 1] = name end
    end
    -- Sorted, so "declared twice" always names the same winner.
    table.sort(files)

    local n = 0
    for _, name in ipairs(files) do
        local rok, mod = pcall(require, "prototypes." .. name)
        if not rok then
            log_error("PROTOTYPES: failed to load '" .. name .. "': " .. tostring(mod))
            note(_problems, "?", name, "error",
                "prototypes/" .. name .. " does not load: " .. tostring(mod))
        elseif type(mod) ~= "table" then
            log_warn("PROTOTYPES: '" .. name .. "' did not return a table")
        else
            absorb(_index, _problems, name, mod)
            n = n + 1
        end
    end

    for _, p in ipairs(_problems) do
        if p.level == "error" then log_error("PROTOTYPES: " .. p.message) end
    end

    if n > 0 then
        local count = 0
        for _, bag in pairs(_index) do
            for _ in pairs(bag) do count = count + 1 end
        end
        log("info", "PROTOTYPES: " .. count .. " prototype(s) from " .. n .. " file(s)")
    end

    return _index
end

--- The whole index: `{ [kind] = { [id] = data } }`.
function M.all()
    return discover()
end

--- One prototype's authoring data, or nil. **Never mutate what this returns** —
--- it is shared by every template that inherits it. `lib/prototype.lua` copies
--- before merging, which is the only reason that is safe.
--- @param kind string
--- @param id string
--- @return table|nil
function M.get(kind, id)
    if type(kind) ~= "string" or type(id) ~= "string" then return nil end
    local bag = discover()[kind]
    return bag and bag[id] or nil
end

--- Every prototype id for a kind, sorted.
--- @param kind string
--- @return table
function M.ids(kind)
    local out = {}
    for id in pairs(discover()[kind] or {}) do out[#out + 1] = id end
    table.sort(out)
    return out
end

--- Which *other* kind declares this id, if any.
---
--- So a missing parent can say "'beast' is an item prototype; this is a mob"
--- rather than "does not exist", which is the same message for a typo and for
--- a plural in the wrong place.
--- @param id string
--- @param except string|nil
--- @return string|nil
function M.find_kind(id, except)
    local hits = {}
    for kind, bag in pairs(discover()) do
        if kind ~= except and bag[id] then hits[#hits + 1] = kind end
    end
    table.sort(hits)
    return hits[1]
end

--- Everything wrong with the prototype library itself.
---
--- A broken prototype breaks every area that names it, so this is worth asking
--- once at load rather than N times as "does not exist" against each child.
--- @return table  array of { kind, id, level, message }
function M.problems()
    discover()
    return _problems or {}
end

--- Drop the cache so a reload picks up an edited or new file.
---
--- Called from `areaload.load` and `areaload.load_all`, which is what makes
--- "editing a prototype takes effect on area reload" structural rather than
--- something each reload path has to remember. `package.loaded` is purged too,
--- or `require` would hand back the old table.
function M.flush_cache()
    if type(_index) == "table" then
        for name in pairs(package.loaded) do
            if type(name) == "string" and name:match("^prototypes%.") then
                package.loaded[name] = nil
            end
        end
    end
    _index, _problems = nil, nil

    local ok, proto = pcall(require, 'lib.prototype')
    if ok and proto and proto.flush_cache then proto.flush_cache() end
end

return M
