-- mudlib/daemons/trait_d.lua — Character attributes, and where their numbers come from.
--
-- A trait is a named number on an entity. What makes it more than a table
-- field is that its *effective* value is computed rather than stored: it can be
-- derived from other traits, and it is filtered through whatever effects the
-- entity is under at the moment you ask.
--
-- Four kinds, and the difference that matters is what is stored:
--
--   attribute   stores a base            strength, wisdom, level
--   derived     stores NOTHING           max_hp (from constitution), willpower (from wisdom)
--   gauge       stores current + anchor   hp, mp — depletable, and may regenerate
--   counter     stores current            xp, gold
--
-- Deliberately, a trait has no `mod` field. Evennia's Traits contrib stores one
-- on every trait, which makes a buff a write to the thing it buffs — so buffs
-- have to be applied and unapplied symmetrically, and any bug in that leaves
-- the number permanently wrong. Here a modifier is never stored anywhere: it is
-- an Effect, and the value is recomputed from the base every time the set of
-- effects changes. Nothing to unapply, nothing to drift.
--
-- The corollary is a rule worth stating outright:
--
--   Effects modify `attribute` and `derived` traits. They never modify a
--   `gauge` or a `counter`.
--
-- A buff does not modify your current HP — it raises max_hp (derived), or it
-- heals you (an event). Stored-current traits are changed by events, not by
-- modifiers. This is what keeps value resolution from recursing, and it is why
-- a gauge's maximum is just another trait rather than a special field.
--
-- State lives on `entity.stats`, which CHARACTER_D already saves — so a trait
-- costs no new persistence path. Regeneration anchors live in `stats._at`.
--
-- Exposes:
--   DAEMON.trait.define(spec) / define_all(list) / seal() / defs() / errors()
--   DAEMON.trait.value(entity, id)     effective, after effects
--   DAEMON.trait.base(entity, id)      what is stored
--   DAEMON.trait.set_base / set_cur / adjust
--   DAEMON.trait.touch(entity)         settle regeneration
--   DAEMON.trait.all(entity)           everything, for `score`
--   DAEMON.trait.attach / detach / bump / bump_all
--
-- See docs/src/lua-api/traits.md.

local traitlib = require('lib.traits')
local persist  = require('lib.persist')

local M = {}

local ROOT_KEY = "trait_d"
local KINDS = { attribute = true, derived = true, gauge = true, counter = true }

-- Traits whose value is computed from a base, and which effects may therefore
-- modify. The other two store what they are.
local MODIFIABLE = { attribute = true, derived = true }

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

local function log_warn(message)
    log("warn", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.warn, message) end
end

--- Definitions survive a hot reload of this file, because they were registered
--- by the *game* layer and this daemon cannot re-derive them on its own.
---
--- Held in an upvalue after the first fetch: `get_persistent` is an efun, so
--- every call crosses into Rust, and this is read several times per trait per
--- resolution — on a path the prompt walks on every command. A hot reload
--- makes a new module table and a new upvalue, so it cannot go stale.
local S = nil

local function root()
    if S then return S end
    S = persist.root(ROOT_KEY, 1, function()
        return { defs = {}, order = nil, failed = {}, regen = {}, gen = 1 }
    end)
    return S
end

-- Memoized effective values. Module-level and weak-keyed: derived from the
-- definitions and the entity, so losing it on a reload costs one recompute,
-- and a mob that stops existing must not be kept alive by its own cache.
local _memo = setmetatable({}, { __mode = "k" })
local _entity_gen = setmetatable({}, { __mode = "k" })

-- Guards `touch` re-entering itself through `value`.
local _settling = false

-- ─── Registration ────────────────────────────────────────────────────────────

--- Declare one trait.
--- @param spec table  { id, kind, label, group, depends, formula, min, max,
---                      round, default, regen, hidden }
--- @return boolean
function M.define(spec)
    if type(spec) ~= "table" or type(spec.id) ~= "string" or #spec.id == 0 then
        log_warn("TRAIT_D.define: a trait needs a string id")
        return false
    end
    local kind = spec.kind or "attribute"
    if not KINDS[kind] then
        log_warn("TRAIT_D.define('" .. spec.id .. "'): unknown kind '" .. tostring(kind)
            .. "' — expected attribute, derived, gauge or counter")
        return false
    end
    if kind == "derived" and type(spec.formula) ~= "function" then
        log_warn("TRAIT_D.define('" .. spec.id .. "'): a derived trait needs a formula")
        return false
    end
    if kind == "derived" and type(spec.depends) ~= "table" then
        log_warn("TRAIT_D.define('" .. spec.id .. "'): a derived trait must declare `depends`")
        return false
    end
    if spec.id:sub(1, 1) == "_" then
        log_warn("TRAIT_D.define('" .. spec.id .. "'): ids beginning with _ are reserved "
            .. "(stats._at holds regeneration anchors)")
        return false
    end

    local r = root()
    r.defs[spec.id] = {
        id      = spec.id,
        kind    = kind,
        label   = spec.label or spec.id,
        group   = spec.group or "general",
        depends = spec.depends or {},
        formula = spec.formula,
        min     = spec.min,
        max     = spec.max,
        round   = spec.round or (kind == "derived" and "floor" or "none"),
        default = spec.default or 0,
        regen   = spec.regen,
        hidden  = spec.hidden or false,
    }
    r.order = nil          -- the graph changed; it must be sealed again
    r.gen = r.gen + 1
    return true
end

--- Register an array of specs, the way areas register rooms.
function M.define_all(list)
    if type(list) ~= "table" then
        log_warn("TRAIT_D.define_all: expected an array of specs")
        return 0
    end
    local n = 0
    for _, spec in ipairs(list) do
        if M.define(spec) then n = n + 1 end
    end
    return n
end

--- Work out the evaluation order and report anything broken.
---
--- A bad trait file must not take the server down — the whole game layer is
--- loaded inside pcalls for the same reason. So a cycle or a dangling
--- dependency marks those traits failed (they answer with their default) and
--- everything else keeps working.
--- @return boolean ok
function M.seal()
    local r = root()
    r.failed = {}

    -- A trait bounded by another trait depends on it, whether or not the
    -- author thought to say so. Folding that in here means the topological
    -- order is right without asking anyone to remember it.
    local graph = {}
    for id, def in pairs(r.defs) do
        local deps = {}
        for _, d in ipairs(def.depends) do deps[#deps + 1] = d end
        for _, bound in ipairs({ def.min, def.max }) do
            if type(bound) == "string" then deps[#deps + 1] = bound end
        end
        if def.regen and type(def.regen.target) == "string"
            and def.regen.target ~= "max" and def.regen.target ~= "min" then
            deps[#deps + 1] = def.regen.target
        end
        graph[id] = { depends = deps }

        for _, d in ipairs(deps) do
            if not r.defs[d] then
                r.failed[id] = "depends on '" .. d .. "', which is not defined"
                log_error("TRAIT_D: trait '" .. id .. "' " .. r.failed[id])
            end
        end
    end

    local order, cycle = traitlib.topo_sort(graph)
    if not order then
        local path = table.concat(cycle, " -> ")
        log_error("TRAIT_D: dependency cycle: " .. path
            .. " — these traits will answer with their defaults")
        for _, id in ipairs(cycle) do
            r.failed[id] = "in a dependency cycle: " .. path
        end
        -- Everything not in the cycle can still be ordered; drop the cycle and
        -- try again so one bad trait does not disable the other thirty.
        local pruned = {}
        for id, node in pairs(graph) do
            if not r.failed[id] then pruned[id] = node end
        end
        order = traitlib.topo_sort(pruned) or {}
    end

    r.order = order
    r.regen = {}
    for _, id in ipairs(order) do
        local def = r.defs[id]
        if def and def.kind == "gauge" and def.regen then
            r.regen[#r.regen + 1] = id
        end
    end
    r.gen = r.gen + 1
    M.bump_all()
    return next(r.failed) == nil
end

function M.defs()          return root().defs end
function M.get_def(id)     return root().defs[id] end
function M.errors()        return root().failed end

--- The evaluation order, sealing first if a definition changed since.
local function order_of(r)
    if not r.order then M.seal() end
    return r.order or {}
end

-- ─── Memo ────────────────────────────────────────────────────────────────────

--- Mark one entity's values stale.
function M.bump(entity)
    if type(entity) ~= "table" then return end
    _entity_gen[entity] = (_entity_gen[entity] or 0) + 1
end

--- Mark every entity's values stale. Definitions or effect definitions changed.
function M.bump_all()
    local r = root()
    r.gen = r.gen + 1
    for entity in pairs(_memo) do _memo[entity] = nil end
end

-- ─── Resolution ──────────────────────────────────────────────────────────────

local function stats_of(entity)
    if type(entity) ~= "table" then return nil end
    entity.stats = entity.stats or {}
    return entity.stats
end

--- Resolve a bound that may be a number, another trait, or nothing.
local function bound_value(values, spec)
    if type(spec) == "number" then return spec end
    if type(spec) == "string" then return values[spec] end
    return nil
end

--- Run a raw number through whatever effects modify this trait.
---
--- `DAEMON.effect.modify` returns the number unchanged, without allocating,
--- when nothing is listening — which is the case for almost every trait of
--- almost every entity, and is why this can sit on the read path at all.
local function modified(entity, id, raw)
    if not (DAEMON and DAEMON.effect and DAEMON.effect.modify) then return raw end
    local ok, result = pcall(DAEMON.effect.modify, entity, "trait:" .. id, raw)
    if not ok then
        log_error("TRAIT_D: effects for trait '" .. id .. "' raised: " .. tostring(result))
        return raw
    end
    return result
end

--- Compute every trait for one entity, in dependency order.
---
--- Whole-entity rather than per-trait: an entity has a couple of dozen traits,
--- recomputing all of them costs a couple of dozen arithmetic expressions, and
--- it only happens when something actually changed. Per-trait invalidation
--- would be a great deal more code to save nothing measurable.
local function recompute(entity)
    local r = root()
    local stats = stats_of(entity)
    local values = {}

    -- One proxy per recompute, not per trait. Reading a dependency the trait
    -- did not declare raises, which is what keeps `depends` honest — and
    -- `depends` is what the cycle detector reasons about, so a stale one would
    -- make its answer a lie.
    local current, allowed = nil, nil
    local proxy = setmetatable({}, {
        __index = function(_, key)
            if not (allowed and allowed[key]) then
                error("trait '" .. tostring(current) .. "' read undeclared dependency '"
                    .. tostring(key) .. "' (add it to depends)", 0)
            end
            return values[key]
        end,
        __newindex = function()
            error("a trait formula must not assign to its dependencies", 0)
        end,
    })

    for _, id in ipairs(order_of(r)) do
        local def = r.defs[id]
        if def then
            local raw

            if r.failed[id] then
                raw = def.default
            elseif def.kind == "derived" then
                current = id
                allowed = {}
                for _, d in ipairs(def.depends) do allowed[d] = true end
                local ok, result = pcall(def.formula, proxy, entity)
                if ok and type(result) == "number" then
                    raw = result
                else
                    if not ok then
                        log_error("TRAIT_D: formula for '" .. id .. "' failed: " .. tostring(result))
                    else
                        log_error("TRAIT_D: formula for '" .. id .. "' returned "
                            .. type(result) .. ", expected a number")
                    end
                    raw = def.default
                end
            else
                local stored = stats[id]
                raw = type(stored) == "number" and stored or def.default
            end

            if MODIFIABLE[def.kind] then
                raw = modified(entity, id, raw)
            end

            raw = traitlib.clamp(raw, bound_value(values, def.min), bound_value(values, def.max))
            values[id] = traitlib.round(raw, def.round)
        end
    end

    local next_expiry = nil
    if DAEMON and DAEMON.effect and DAEMON.effect.next_expiry then
        local ok, exp = pcall(DAEMON.effect.next_expiry, entity)
        if ok then next_expiry = exp end
    end

    _memo[entity] = {
        gen = r.gen,
        egen = _entity_gen[entity] or 0,
        expiry = next_expiry,
        values = values,
    }
    return values
end

--- Are the memoized values still the right answer?
local function fresh(entity)
    local cached = _memo[entity]
    if not cached then return nil end
    if cached.gen ~= root().gen then return nil end
    if cached.egen ~= (_entity_gen[entity] or 0) then return nil end
    -- Time does not change a value directly. The only way it can is by expiring
    -- an effect, so one comparison covers the whole clock.
    if cached.expiry and os_time() >= cached.expiry then return nil end
    return cached.values
end

--- The effective value: base (or formula), then effects, then bounds.
function M.value(entity, id)
    if type(entity) ~= "table" or type(id) ~= "string" then return 0 end
    local def = root().defs[id]
    if not def then return 0 end

    -- A regenerating gauge is a function of the clock, so settle it before
    -- reading. `_settling` stops the settle's own reads coming back here.
    if def.regen and not _settling then M.touch(entity) end

    local values = fresh(entity) or recompute(entity)
    return values[id] or def.default
end

--- What is stored, before any effect touches it. Derived traits store nothing,
--- so this is their computed value.
function M.base(entity, id)
    local def = root().defs[id]
    if not def then return 0 end
    if def.kind == "derived" then return M.value(entity, id) end
    local stats = stats_of(entity)
    local stored = stats and stats[id]
    return type(stored) == "number" and stored or def.default
end

--- Every trait at once — for `score`, and for a test that wants to compare the
--- whole picture.
--- @return table  array of { id, label, group, kind, base, value, hidden, max }
function M.all(entity)
    local r = root()
    if not r.order then M.seal() end
    M.touch(entity)
    local values = fresh(entity) or recompute(entity)
    local out = {}
    for _, id in ipairs(r.order or {}) do
        local def = r.defs[id]
        if def then
            out[#out + 1] = {
                id = id, label = def.label, group = def.group, kind = def.kind,
                base = M.base(entity, id), value = values[id] or def.default,
                hidden = def.hidden,
                max = type(def.max) == "string" and values[def.max] or def.max,
                failed = r.failed[id],
            }
        end
    end
    return out
end

-- ─── Mutation ────────────────────────────────────────────────────────────────

--- Change a stored base. Attributes and counters only — a derived trait has no
--- base to set, and a gauge's current value goes through set_cur.
function M.set_base(entity, id, value)
    local def = root().defs[id]
    if not def then
        log_warn("TRAIT_D.set_base: no such trait '" .. tostring(id) .. "'")
        return false
    end
    if def.kind == "derived" then
        log_warn("TRAIT_D.set_base('" .. id .. "'): derived traits are computed, not stored")
        return false
    end
    if type(value) ~= "number" then return false end
    local stats = stats_of(entity); if not stats then return false end
    stats[id] = value
    M.bump(entity)
    return true
end

--- Resolve a gauge's bounds and regeneration target for the current moment.
local function gauge_bounds(entity, def)
    local min = type(def.min) == "string" and M.value(entity, def.min) or def.min or 0
    local max = type(def.max) == "string" and M.value(entity, def.max) or def.max
    local target = max
    if def.regen then
        local t = def.regen.target
        if t == "min" then target = min
        elseif type(t) == "number" then target = t
        elseif type(t) == "string" and t ~= "max" then target = M.value(entity, t)
        end
    end
    return min, max, target
end

--- Set a gauge or counter's current value, clamped.
function M.set_cur(entity, id, value)
    local def = root().defs[id]
    if not def or def.kind == "derived" then return false end
    if type(value) ~= "number" then return false end
    local stats = stats_of(entity); if not stats then return false end

    if def.kind == "gauge" then
        local was_settled = _settling
        _settling = true
        local min, max, target = gauge_bounds(entity, def)
        local before = stats[id]
        value = traitlib.clamp(value, min, max)
        stats[id] = traitlib.round(value, def.round == "none" and "floor" or def.round)

        -- Leaving the target starts the regeneration clock. Doing it here
        -- rather than in `settle` is what lets a gauge sit at full without
        -- writing an anchor on every read — and stops it banking credit while
        -- it does.
        if def.regen and before == target and stats[id] ~= target then
            stats._at = stats._at or {}
            stats._at[id] = os_time()
        end
        _settling = was_settled
    else
        stats[id] = traitlib.clamp(value, def.min, def.max)
    end

    M.bump(entity)
    return true
end

--- Add to a gauge or counter, settling regeneration first so the delta applies
--- to the value as it is *now*, not as it was when it was last read.
--- @return number  the new value
function M.adjust(entity, id, delta)
    local def = root().defs[id]
    if not def or def.kind == "derived" then return 0 end
    if type(delta) ~= "number" then return M.value(entity, id) end
    if def.regen then M.touch(entity) end
    local current = M.base(entity, id)
    M.set_cur(entity, id, current + delta)
    return M.value(entity, id)
end

-- ─── Regeneration ────────────────────────────────────────────────────────────

--- Bring every regenerating gauge up to date.
---
--- Cheap enough to call on the prompt, which is where it is called from: for an
--- entity whose gauges are full, or whose last settle earned less than one
--- point, this changes nothing and — critically — writes nothing. A settle that
--- always reported a change would dirty every online player's state several
--- times a second and undo the entire point of the write-behind tier.
function M.touch(entity)
    if _settling then return false end
    local r = root()
    if #(r.regen or {}) == 0 then return false end
    local stats = stats_of(entity); if not stats then return false end

    _settling = true
    local now = os_time()
    local changed = false

    for _, id in ipairs(r.regen) do
        local def = r.defs[id]
        local cur = stats[id]
        if type(cur) == "number" then
            local min, max, target = gauge_bounds(entity, def)
            if type(target) == "number" then
                stats._at = stats._at or {}
                -- The rate is an ordinary event, so "+50% healing rate" is an
                -- ordinary effect and needs nothing new here.
                local rate = def.regen.rate or 0
                if DAEMON and DAEMON.effect and DAEMON.effect.modify then
                    local ok, scaled = pcall(DAEMON.effect.modify, entity, "regen_rate", rate)
                    if ok and type(scaled) == "number" then rate = scaled end
                end

                local new_cur, new_at = traitlib.settle(
                    cur, stats._at[id], now, rate, def.regen.per or 1, target, min, max)
                if new_cur then
                    stats[id] = new_cur
                    stats._at[id] = new_at
                    changed = true
                end
            end
        end
    end

    _settling = false
    if changed then M.bump(entity) end
    return changed
end

-- ─── Lifecycle ───────────────────────────────────────────────────────────────

--- Prepare an entity's stats for use: fill in defaults, drop anything stored
--- for a trait that is now derived, and clamp gauges into their current bounds.
---
--- The middle one is a real migration. `max_hp` used to be a stored number; it
--- is a derived trait now, so a saved value would shadow the formula forever.
function M.attach(entity)
    local r = root()
    if not r.order then M.seal() end
    local stats = stats_of(entity); if not stats then return false end
    stats._at = stats._at or {}

    for id, def in pairs(r.defs) do
        if def.kind == "derived" then
            if stats[id] ~= nil then stats[id] = nil end
        elseif type(stats[id]) ~= "number" then
            stats[id] = def.default
        end
    end

    M.bump(entity)

    -- Bounds may have moved while the character was away — a level-up, an item
    -- gone from a slot — so clamp after the defaults are in place.
    _settling = true
    local now = os_time()
    for _, id in ipairs(r.order or {}) do
        local def = r.defs[id]
        if def and def.kind == "gauge" then
            local min, max = gauge_bounds(entity, def)
            stats[id] = traitlib.clamp(stats[id] or def.default, min, max)
            if def.regen and not def.regen.offline then
                -- Otherwise three days offline arrive as a full bar the moment
                -- they log in, which is not what "regenerates in play" means.
                stats._at[id] = now
            end
        end
    end
    _settling = false

    M.bump(entity)
    return true
end

--- Forget an entity's memo. The stats themselves belong to the entity and are
--- saved by CHARACTER_D; nothing to write here.
function M.detach(entity)
    if type(entity) == "table" then
        _memo[entity] = nil
        _entity_gen[entity] = nil
    end
    return true
end

log("info", "trait_d daemon loaded")

return M
