-- mudlib/lib/body.lua — What a creature is made of, and where a blow lands.
--
-- A layout is a list of parts, each with a **size** (how likely it is to be hit)
-- and a **height** (where it is, as a percentage of this creature's own height).
-- Height as a percentage rather than centimetres is what lets one `humanoid`
-- layout serve a halfling and a giant.
--
-- ─── Optional, by absence ────────────────────────────────────────────────────
--
-- `M.of(entity)` returns nil for anything with no layout, and nil is the entire
-- backwards-compatible path: no layout means no hit location, which means the
-- `hit_slot` guard in armour never fires and combat resolves exactly as it did.
-- There is no `if layouts_enabled` anywhere.
--
-- ─── Not a daemon ────────────────────────────────────────────────────────────
--
-- The pair is `body/init.lua` (a discovered index) and this (the arithmetic),
-- which is `prototypes/init.lua` and `lib/prototype.lua` line for line — and
-- there is no `prototype_d` either. Nothing here holds state.
--
-- > Requires `body` **inside functions**, so the pure half loads in a VM with no
-- > `list_dir`. Same rule, same reason, as `lib/prototype.lua`.
--
-- Exposes:
--   Body.normalise(id, layout)             -> layout, problems
--   Body.of(entity)                        -> layout | nil
--   Body.parts(layout)                     -> array
--   Body.has_feature(entity, feature)      -> boolean
--   Body.window(attacker_h, defender_h, weapon_length, opts) -> low, high
--   Body.candidates(layout, low, high)     -> array
--   Body.pick(candidates, rng)             -> part | nil
--   Body.locate(attacker, defender, weapon, opts) -> part | nil
--
-- See docs/src/lua-api/bodies.md.

local M = {}

--- How far past their own height somebody reaches, before the weapon.
M.REACH_RATIO = 1.15

-- ─── Layouts ─────────────────────────────────────────────────────────────────

--- Check a layout over and fill it out. Reports rather than raises — one bad
--- layout must not take the others with it.
--- @return table layout, table problems
function M.normalise(id, layout)
    local problems = {}
    if type(layout) ~= "table" then
        return nil, { "layout '" .. tostring(id) .. "' is not a table" }
    end

    layout.id = id
    layout.parts = type(layout.parts) == "table" and layout.parts or {}
    layout.features = type(layout.features) == "table" and layout.features or {}

    local seen, kept = {}, {}
    for _, part in ipairs(layout.parts) do
        if type(part) ~= "table" or type(part.id) ~= "string" or part.id == "" then
            problems[#problems + 1] = "'" .. tostring(id) .. "' has a part with no id"
        elseif seen[part.id] then
            problems[#problems + 1] = "'" .. tostring(id) .. "' declares '"
                .. part.id .. "' twice"
        else
            local size = tonumber(part.size) or 0
            local height = tonumber(part.height)
            if size <= 0 then
                problems[#problems + 1] = "'" .. id .. "." .. part.id
                    .. "' has no size, so it can never be hit"
            elseif height == nil or height < 0 or height > 100 then
                problems[#problems + 1] = "'" .. id .. "." .. part.id
                    .. "' needs a height between 0 and 100 (percent of the creature)"
            else
                seen[part.id] = true
                part.size, part.height = size, height
                kept[#kept + 1] = part
            end
        end
    end
    layout.parts = kept

    if #kept == 0 then
        problems[#problems + 1] = "'" .. tostring(id) .. "' has no usable parts"
    end
    return layout, problems
end

--- Which layout this entity has, or nil.
---
--- Three rungs: what it declares, then its race, then a configured default.
--- The `race` rung is what finally makes that field mean something; it is
--- documented, and `body` overrides it.
--- @return table|nil
function M.of(entity)
    if type(entity) ~= "table" then return nil end

    local ok, index = pcall(require, 'body')
    if not ok or type(index) ~= "table" then return nil end

    if type(entity.body) == "string" then
        local named = index.get(entity.body)
        if named then return named end
    end
    if type(entity.race) == "string" then
        local raced = index.get(entity.race)
        if raced then return raced end
    end

    local cok, default = pcall(config, "game.combat_default_body")
    if cok and type(default) == "string" then return index.get(default) end
    return nil
end

function M.parts(layout)
    return (type(layout) == "table" and layout.parts) or {}
end

--- Does this creature have the feature an ability asks for?
---
--- Read from the parts *and* from the layout's own list, so "this thing has
--- hands" can be said once at the top or attached to the hands.
function M.has_feature(entity, feature)
    local layout = M.of(entity)
    if not layout then return false end
    for _, f in ipairs(layout.features or {}) do
        if f == feature then return true end
    end
    for _, part in ipairs(layout.parts) do
        for _, f in ipairs(part.features or {}) do
            if f == feature then return true end
        end
    end
    return false
end

-- ─── Where a blow can land ───────────────────────────────────────────────────

--- The band of the defender a swing can reach, as percentages of the defender.
---
--- Either height missing disables the filter entirely, which is the ordinary
--- case for a game that has defined no `height` trait.
--- @return number low, number high
function M.window(attacker_height, defender_height, weapon_length, opts)
    opts = opts or {}
    local a = tonumber(attacker_height) or 0
    local d = tonumber(defender_height) or 0
    if a <= 0 or d <= 0 then return 0, 100 end

    local reach = a * (tonumber(opts.reach_ratio) or M.REACH_RATIO)
        + (tonumber(weapon_length) or 0)
    local high = (reach / d) * 100
    if high > 100 then high = 100 end
    return 0, high
end

--- Which parts are inside the window.
---
--- **If nothing is, the lowest part or parts are returned rather than an empty
--- set.** A halfling with a dagger against a giant hits its shins; it does not
--- miss the whole creature, and it certainly does not reach its head.
--- @return table  array of parts
function M.candidates(layout, low, high)
    local parts = M.parts(layout)
    local out = {}
    for _, part in ipairs(parts) do
        if part.height >= (low or 0) and part.height <= (high or 100) then
            out[#out + 1] = part
        end
    end
    if #out > 0 then return out end

    local lowest = nil
    for _, part in ipairs(parts) do
        if lowest == nil or part.height < lowest then lowest = part.height end
    end
    for _, part in ipairs(parts) do
        if part.height == lowest then out[#out + 1] = part end
    end
    return out
end

--- Weighted by size. `rng` is injected so a test can force the location.
--- @return table|nil
function M.pick(candidates, rng)
    local total = 0
    for _, part in ipairs(candidates or {}) do total = total + part.size end
    if total <= 0 then return nil end

    local roll = (rng or function(n) return math.random(n) end)(math.floor(total))
    local seen = 0
    for _, part in ipairs(candidates) do
        seen = seen + part.size
        if roll <= seen then return part end
    end
    return candidates[#candidates]
end

--- Where this swing lands on this defender, or nil when there is no layout.
--- @return table|nil part
function M.locate(attacker, defender, weapon, opts)
    opts = opts or {}
    local layout = M.of(defender)
    if not layout then return nil end

    if type(opts.force) == "string" then
        for _, part in ipairs(M.parts(layout)) do
            if part.id == opts.force then return part end
        end
    end

    local function height_of(entity)
        if type(entity) ~= "table" or type(entity.trait) ~= "function" then return 0 end
        if DAEMON and DAEMON.trait and DAEMON.trait.has
            and DAEMON.trait.has(entity, "height") then
            local h = entity:trait("height")
            return type(h) == "number" and h or 0
        end
        return 0
    end

    local length = 0
    if type(weapon) == "table" and type(weapon.weapon) == "table" then
        length = tonumber(weapon.weapon.length) or 0
        -- An arrow reaches the head of anything, so a ranged weapon skips the
        -- reach window rather than needing a flag of its own.
        if weapon.weapon.range == "ranged" then
            return M.pick(M.parts(layout), opts.rng)
        end
    end

    local low, high = M.window(height_of(attacker), height_of(defender), length, opts)
    return M.pick(M.candidates(layout, low, high), opts.rng)
end

return M
