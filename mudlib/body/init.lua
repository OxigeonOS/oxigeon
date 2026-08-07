-- mudlib/body/init.lua — The index of body layouts.
--
-- Discovery, the same shape as `prototypes/init.lua` and `schema/init.lua`:
-- `list_dir` over this directory across both jail roots, `require` each module,
-- keep what looks like a layout. So `game/body/creatures.lua` appears here with
-- no central list to keep in step.
--
-- **The mudlib ships no layouts.** A humanoid is game content, and a mudlib that
-- shipped one would be asserting that its creatures have hands.
--
--     -- game/body/creatures.lua
--     return {
--         layouts = {
--             humanoid = {
--                 features = { "hands", "feet", "eyes" },
--                 parts = {
--                     { id = "head",  size = 8,  height = 95, slot = "head",
--                       vulnerable = { piercing = 0.25 } },
--                     { id = "chest", size = 30, height = 70, slot = "chest" },
--                     { id = "legs",  size = 20, height = 30, slot = "legs" },
--                 },
--             },
--         },
--     }
--
-- `size` need not sum to anything — `lib/body.lua` normalises. `height` is a
-- percentage of the creature's own height, so one layout fits any size of it.
-- **A field this index does not know is kept and rides through onto the hit
-- result**, so there is no closed part-field list to rot.
--
-- Exposes:
--   body.all() / body.get(id) / body.ids()
--   body.problems() / body.flush_cache()

local M = {}

local _index    = nil
local _problems = nil

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

local function discover()
    if _index then return _index end
    _index, _problems = {}, {}

    if type(list_dir) ~= "function" then return _index end

    local ok, entries = pcall(list_dir, "body")
    if not ok or type(entries) ~= "table" then
        -- Not an error: a game with no layouts is the ordinary state, and it is
        -- the whole backwards-compatible path.
        return _index
    end

    local Body = require('lib.body')

    local files = {}
    for _, entry in ipairs(entries) do
        local file   = type(entry) == "table" and entry.name
        local is_dir = type(entry) == "table" and entry.is_dir
        local name   = (not is_dir) and file and file:match("^(.+)%.lua$")
        if name and name ~= "init" then files[#files + 1] = name end
    end
    table.sort(files)   -- so "declared twice" always names the same winner

    local n = 0
    for _, name in ipairs(files) do
        local rok, mod = pcall(require, "body." .. name)
        if not rok then
            log_error("BODY: failed to load '" .. name .. "': " .. tostring(mod))
            _problems[#_problems + 1] = "body/" .. name .. " does not load: " .. tostring(mod)
        elseif type(mod) == "table" and type(mod.layouts) == "table" then
            for id, layout in pairs(mod.layouts) do
                if _index[id] then
                    _problems[#_problems + 1] = "layout '" .. tostring(id)
                        .. "' is declared twice; body/" .. name .. " wins"
                end
                local normalised, problems = Body.normalise(id, layout)
                for _, p in ipairs(problems or {}) do
                    _problems[#_problems + 1] = p
                end
                if normalised and #normalised.parts > 0 then
                    _index[id] = normalised
                    n = n + 1
                end
            end
        end
    end

    for _, p in ipairs(_problems) do log_error("BODY: " .. p) end
    if n > 0 then log("info", "BODY: " .. n .. " layout(s)") end
    return _index
end

function M.all() return discover() end

function M.get(id)
    if type(id) ~= "string" then return nil end
    return discover()[id]
end

function M.ids()
    local out = {}
    for id in pairs(discover()) do out[#out + 1] = id end
    table.sort(out)
    return out
end

--- Everything wrong with the layout library, for `verify`.
function M.problems()
    discover()
    return _problems or {}
end

--- Drop the cache so a reload picks up an edit. Called from `mudlib/init.lua`'s
--- `on_load`, beside the schema and prototype indexes.
function M.flush_cache()
    if type(_index) == "table" then
        for name in pairs(package.loaded) do
            if type(name) == "string" and name:match("^body%.") then
                package.loaded[name] = nil
            end
        end
    end
    _index, _problems = nil, nil
end

return M
