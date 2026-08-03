-- mudlib/lib/jsonsafe.lua — Can this value survive a trip to the database?
--
-- The driver's `lua_to_json` refuses six kinds of value, and every one of them
-- is decidable from Lua. Asking here, at the call site, turns "the shutdown
-- flush raised for player 42 and we do not know why" into "this write was
-- refused, and the field is named".
--
-- The six rules mirrored, in the order `lua_to_json` applies them:
--   1. a key that is not a string or an integer
--   2. a table that is both a list and a map
--   3. a function, userdata or thread anywhere in the value
--   4. NaN or infinity
--   5. nesting deeper than 64 (which is also how the driver catches cycles)
--   6. more than 100000 values
--
-- These are a *reimplementation* of Rust logic, so they can drift. That is
-- what `tests/state_cache.rs::the_validator_and_lua_to_json_agree` exists to
-- prevent — it feeds the same hostile values to both and demands they agree.
--
-- Exposes:
--   jsonsafe.check(value)          -> true | false, reason
--   jsonsafe.estimate_bytes(value) -> approximate serialized size

local M = {}

-- Both taken from src/core/scripting/efuns.rs.
M.MAX_DEPTH = 64
M.MAX_NODES = 100000

--- Render a breadcrumb the same way the driver does, so the two error
--- messages point at the same place.
local function join(path, key)
    if path == "" then return tostring(key) end
    if type(key) == "number" then return path .. "[" .. tostring(key) .. "]" end
    return path .. "." .. tostring(key)
end

--- Is this number one JSON can represent? NaN and the infinities are not.
local function finite(n)
    -- NaN is the only value that is not equal to itself; the infinities are
    -- caught by comparing against themselves doubled.
    return n == n and n ~= math.huge and n ~= -math.huge
end

local function is_index(k)
    return type(k) == "number" and k >= 1 and k == math.floor(k)
end

--- Walk `value`, returning false and a reason at the first thing that could
--- not be written. `budget` is a one-element table so the node count is shared
--- across the whole walk rather than per branch.
local function walk(value, depth, budget, path)
    budget[1] = budget[1] - 1
    if budget[1] < 0 then
        return false, "more than " .. M.MAX_NODES .. " values, giving up at "
            .. (path == "" and "the top level" or path)
    end

    local t = type(value)
    if t == "nil" or t == "boolean" or t == "string" then
        return true
    end
    if t == "number" then
        if not finite(value) then
            return false, (path == "" and "the value" or path)
                .. " is " .. tostring(value)
                .. ", which JSON cannot represent (NaN and infinity do not)"
        end
        return true
    end
    if t ~= "table" then
        return false, (path == "" and "the value" or path)
            .. " is a " .. t .. ", which cannot be saved"
    end

    if depth > M.MAX_DEPTH then
        return false, "nesting is deeper than " .. M.MAX_DEPTH .. " at " .. path
            .. " — a table that refers to itself will always hit this"
    end

    -- Classify before descending: a table that is both a list and a map has no
    -- JSON counterpart, and saying so is more useful than a type error further
    -- down.
    local indexed, named = 0, nil
    for k in pairs(value) do
        if is_index(k) then
            indexed = indexed + 1
        elseif type(k) == "string" then
            named = named or {}
            named[#named + 1] = k
        else
            return false, (path == "" and "the value" or path)
                .. " has a key of type '" .. type(k)
                .. "', and JSON keys can only be strings or integers"
        end
    end
    if indexed > 0 and named then
        table.sort(named)
        local shown = {}
        for i = 1, math.min(4, #named) do shown[i] = "'" .. named[i] .. "'" end
        local list = table.concat(shown, ", ")
        if #named > 4 then list = list .. " and " .. (#named - 4) .. " more" end
        return false, (path == "" and "the value" or path)
            .. " has " .. indexed .. " list entries and also the key(s) " .. list
            .. " — JSON has no type that is both a list and a map"
    end

    for k, v in pairs(value) do
        local ok, err = walk(v, depth + 1, budget, join(path, k))
        if not ok then return false, err end
    end
    return true
end

--- Could this value be written to the document store?
--- @param value any
--- @return boolean ok
--- @return string|nil reason  naming the offending field, when not
function M.check(value)
    return walk(value, 0, { M.MAX_NODES }, "")
end

--- Roughly how many bytes this value would serialize to.
---
--- Deliberately an estimate: there is no JSON encoder exposed to Lua, so the
--- only exact answer costs a `db_put` that raises. This is used to warn before
--- a scope approaches `documents.max_bytes` and to refuse the write that would
--- cross it, so it only has to be conservative and cheap — never exact.
--- @param value any
--- @return number
function M.estimate_bytes(value)
    local t = type(value)
    if t == "nil" then return 4 end
    if t == "boolean" then return 5 end
    if t == "number" then return 8 end
    if t == "string" then return #value + 3 end
    if t ~= "table" then return 8 end

    local total = 2 -- the braces
    for k, v in pairs(value) do
        total = total + (type(k) == "string" and (#k + 4) or 6)
        total = total + M.estimate_bytes(v)
    end
    return total
end

return M
