-- mudlib/schema/init.lua — The index of authorable kinds.
--
-- Discovery, the same shape as `components/init.lua`: `list_dir` over this
-- directory, `require` each module, keep the ones that look like a schema. A
-- schema is any module here exposing `kind` (its name) and `fields` (an ordered
-- array of descriptors).
--
-- So a game layer that invents a fourth authorable kind drops in
-- `game/schema/npc.lua` and it appears — `list_dir` searches both roots. There
-- is no central list of kinds to keep in step, for the same reason there is no
-- central list of component fields: see the trait rules in CLAUDE.md.
--
-- See mudlib/lib/schema.lua for what the descriptors mean.

local M = {}

local DEFAULT_ORDER = 50

local _list = nil

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then
        pcall(DAEMON.journal.error, message)
    end
end

local function discover()
    if _list then return _list end
    _list = {}

    if type(list_dir) ~= "function" then
        log_error("SCHEMA: list_dir is unavailable; no schemas loaded")
        return _list
    end

    local ok, entries = pcall(list_dir, "schema")
    if not ok or type(entries) ~= "table" then
        log_error("SCHEMA: list_dir('schema') failed: " .. tostring(entries))
        return _list
    end

    for _, entry in ipairs(entries) do
        local file = type(entry) == "table" and entry.name
        local is_dir = type(entry) == "table" and entry.is_dir
        local name = (not is_dir) and file and file:match("^(.+)%.lua$")
        if name and name ~= "init" then
            local rok, mod = pcall(require, "schema." .. name)
            if not rok then
                log_error("SCHEMA: failed to load '" .. name .. "': " .. tostring(mod))
            elseif type(mod) == "table" and type(mod.fields) == "table" then
                mod.kind = mod.kind or name
                _list[#_list + 1] = mod
            end
        end
    end

    table.sort(_list, function(a, b)
        local ao, bo = a.order or DEFAULT_ORDER, b.order or DEFAULT_ORDER
        if ao ~= bo then return ao < bo end
        return tostring(a.kind) < tostring(b.kind)
    end)

    return _list
end

--- Every loaded schema module, in declared order.
--- @return table  array of modules
function M.all()
    return discover()
end

--- One schema by kind name, or nil.
--- @param kind string
--- @return table|nil
function M.get(kind)
    for _, mod in ipairs(discover()) do
        if mod.kind == kind then return mod end
    end
    return nil
end

--- Drop the cache so a hot reload picks up a new or edited schema.
--- Called from `mudlib/init.lua`'s `on_load`, beside `components.flush_cache`.
function M.flush_cache()
    _list = nil
end

return M
