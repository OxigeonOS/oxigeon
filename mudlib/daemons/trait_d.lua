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
--   DAEMON.trait.all(entity, category) what the entity holds, for `score`
--   DAEMON.trait.has / forget / present / categories
--   DAEMON.trait.attach / seed / detach / bump / bump_all
--
-- Presence is decided by storage, never declared: an entity has a stored-kind
-- trait when there is a number for it, and a derived trait when everything it
-- reads is present. A sword has `dps` because it has damage and speed; it has
-- no `willpower` because it has no `wisdom`, and nothing had to say so. That is
-- what keeps a recompute proportional to what the entity holds rather than to
-- how many skills the game has ever defined.
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

--- `sets = "item"` and `sets = {"character", "mob"}` mean the same kind of
--- thing, so accept both. A map rather than an array, so `seed` tests
--- membership without a scan.
---
--- Saying nothing and saying nothing-at-all are different answers, and the
--- difference is load-bearing:
---
---   sets omitted        -> { character }   every trait written before sets
---                                          existed keeps behaving as it did
---   sets = {} / false   -> {}              seeded by nothing, deliberately
---
--- The second is what a skill is. Not having swordsmanship until you learn it
--- is the point of sparse traits, so a skill must be able to say "no set"
--- rather than be tagged into one nobody happens to call `seed` with — that
--- would make its absence an accident of which call sites exist.
local function normalize_sets(spec)
    if spec == nil then return { character = true } end
    if spec == false then return {} end

    local out = {}
    if type(spec) == "string" then
        out[spec] = true
    elseif type(spec) == "table" then
        for _, name in ipairs(spec) do
            if type(name) == "string" then out[name] = true end
        end
    else
        -- A number, a function: not a set list. Treat it as unsaid rather than
        -- as none, so a typo does not silently stop seeding a character stat.
        return { character = true }
    end
    return out
end

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

-- Which traits each entity actually holds, in evaluation order. Guarded by the
-- same two counters as `_memo` and weak-keyed for the same reason. Unlike the
-- value memo it needs no expiry check: an effect ending changes what a trait is
-- worth, never whether the entity has one.
local _present = setmetatable({}, { __mode = "k" })

-- Guards `touch` re-entering itself through `value`.
local _settling = false

-- ─── Registration ────────────────────────────────────────────────────────────

--- Declare one trait.
--- @param spec table  { id, kind, category, label, group, depends, formula,
---                      min, max, round, default, regen, hidden, always, sets }
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
        -- What this number *is* in the game's vocabulary — stat, skill,
        -- resource, condition, reputation. Freeform on purpose: the mudlib
        -- defines no closed list, so a game invents a category without touching
        -- the driver. It is a lens for commands and never changes behaviour; the
        -- moment a category is tempted to *mean* something, that belongs on the
        -- spec as its own declared field. Defaults to "stat" so every trait
        -- defined before this field existed still appears in `score`.
        category = spec.category or "stat",
        -- Which heading it sorts under *within* one command. Presentational.
        group   = spec.group or "general",
        depends = spec.depends or {},
        formula = spec.formula,
        min     = spec.min,
        max     = spec.max,
        round   = spec.round or (kind == "derived" and "floor" or "none"),
        default = spec.default or 0,
        regen   = spec.regen,
        hidden  = spec.hidden or false,
        -- Present on every entity regardless of what it stores. See build_present.
        always  = spec.always or false,
        -- Which seed sets start an entity off with this trait. Not a filter on
        -- reads — presence is decided by storage.
        sets    = normalize_sets(spec.sets),
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

    -- `rank` is a trait's position in the evaluation order. It exists so a
    -- per-entity subset can be sorted back into dependency order without
    -- walking the whole registry — which is the point of the subset.
    for _, def in pairs(r.defs) do def.rank = nil end
    for i, id in ipairs(order) do
        if r.defs[id] then r.defs[id].rank = i end
    end
    -- Failed traits were pruned out of `order` and so have no rank. Rank them
    -- after everything else, by id: without this the subset sort would see
    -- equal keys and produce a `pairs`-dependent order.
    local unranked = {}
    for id, def in pairs(r.defs) do
        if not def.rank then unranked[#unranked + 1] = id end
    end
    table.sort(unranked)
    for i, id in ipairs(unranked) do r.defs[id].rank = #order + i end

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
    for entity in pairs(_present) do _present[entity] = nil end
end

-- ─── Resolution ──────────────────────────────────────────────────────────────

local function stats_of(entity)
    if type(entity) ~= "table" then return nil end
    entity.stats = entity.stats or {}
    return entity.stats
end

--- Work out which traits this entity actually holds.
---
--- Storage decides. An entity has a stored-kind trait when there is a number
--- for it; it has a derived trait when everything that trait reads is present.
--- Applicability is therefore never declared and so can never rot — the same
--- reasoning that makes `depends` enforced rather than advisory. A sword has
--- `dps` because it has damage and speed; it has no `willpower` because it has
--- no `wisdom`, and nothing had to say so.
--- @return table  array of trait ids, in evaluation order
local function build_present(entity)
    local r = root()
    if not r.order then M.seal() end
    local stats = stats_of(entity)
    if not stats then return {} end

    local has = {}

    -- Stored kinds. `stats._at` is a table of regeneration anchors, so the
    -- number test skips it without needing to know about it. A number stored
    -- under an id no trait has claimed is inert rather than an error — a save
    -- written before its trait file loaded must not be a failure.
    for id, value in pairs(stats) do
        if type(value) == "number" then
            local def = r.defs[id]
            if def and def.kind ~= "derived" then has[id] = true end
        end
    end

    -- `always` is the escape hatch for a trait whose formula is meaningful on
    -- defaults alone. It means what it says: present on every entity. If this
    -- turns out to be common, the presence rule above is the thing that is
    -- wrong.
    for id, def in pairs(r.defs) do
        if def.always and not r.failed[id] then has[id] = true end
    end

    -- Derived kinds, to a fixpoint: one derived trait may read another, and
    -- `pairs` gives no useful order to discover that in.
    local changed = true
    while changed do
        changed = false
        for id, def in pairs(r.defs) do
            if def.kind == "derived" and not has[id] and not r.failed[id] then
                local ready = true
                for _, d in ipairs(def.depends) do
                    if not has[d] then ready = false; break end
                end
                if ready then has[id] = true; changed = true end
            end
        end
    end

    -- Bounds are dependencies too. A gauge clamped by a trait the entity does
    -- not have is a gauge with no ceiling, which is a different trait from the
    -- one that was defined — so it is absent instead. Removal cascades, because
    -- a derived trait may read the gauge that just went, so repeat until stable.
    local removed = true
    while removed do
        removed = false
        for id in pairs(has) do
            local def = r.defs[id]
            local ok = true
            if def.min ~= nil and type(def.min) == "string" and not has[def.min] then ok = false end
            if def.max ~= nil and type(def.max) == "string" and not has[def.max] then ok = false end
            if ok and def.kind == "derived" then
                for _, d in ipairs(def.depends) do
                    if not has[d] then ok = false; break end
                end
            end
            if not ok then
                has[id] = nil
                removed = true
            end
        end
    end

    local list = {}
    for id in pairs(has) do list[#list + 1] = id end
    table.sort(list, function(a, b)
        return (r.defs[a].rank or 0) < (r.defs[b].rank or 0)
    end)
    return list
end

--- The entity's traits, cached until the definitions or the entity change.
local function present_of(entity)
    local r = root()
    if not r.order then M.seal() end
    local egen = _entity_gen[entity] or 0
    local cached = _present[entity]
    if cached and cached.gen == r.gen and cached.egen == egen then
        return cached.list
    end
    local list = build_present(entity)
    _present[entity] = { gen = r.gen, egen = egen, list = list }
    return list
end

--- Which traits this entity holds, in evaluation order.
--- @param entity table
--- @return table  array of trait ids
function M.present(entity)
    if type(entity) ~= "table" then return {} end
    local list = present_of(entity)
    local out = {}
    for i, id in ipairs(list) do out[i] = id end
    return out
end

--- Does this entity have this trait at all?
---
--- A different question from what it is worth. `value` answers an absent trait
--- with its default so arithmetic stays safe; this is how you ask whether the
--- character has ever learned the skill.
--- @param entity table
--- @param id string
--- @return boolean
function M.has(entity, id)
    if type(entity) ~= "table" or type(id) ~= "string" then return false end
    for _, present in ipairs(present_of(entity)) do
        if present == id then return true end
    end
    return false
end

--- Take a trait away from an entity — unlearning a skill, an enchantment
--- stripped. Derived traits are not stored, so they cannot be forgotten
--- directly: remove what they read instead.
--- @param entity table
--- @param id string
--- @return boolean  whether anything was removed
function M.forget(entity, id)
    if type(entity) ~= "table" or type(id) ~= "string" then return false end
    local def = root().defs[id]
    if def and def.kind == "derived" then
        log_warn("TRAIT_D.forget('" .. id .. "'): a derived trait is not stored; "
            .. "remove one of the traits it depends on")
        return false
    end
    local stats = stats_of(entity); if not stats then return false end
    if stats[id] == nil then return false end

    stats[id] = nil
    if stats._at then stats._at[id] = nil end
    M.bump(entity)
    return true
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

    -- The entity's own traits, not the registry's. This is what keeps a
    -- recompute proportional to what the entity holds rather than to how many
    -- skills the game has ever defined.
    for _, id in ipairs(present_of(entity)) do
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
---
--- Commands name what they show: `score` renders `category == "stat"`, `skills`
--- renders `category == "skill"`, `traits` renders everything. Filtering here
--- rather than in each command would put the lens in the wrong place, so this
--- returns the lot and carries `category` on every row.
--- @param entity table
--- @param category string|nil  when given, only traits in that category
--- @return table  array of { id, label, group, category, kind, base, value, hidden, max }
function M.all(entity, category)
    local r = root()
    if not r.order then M.seal() end
    M.touch(entity)
    local values = fresh(entity) or recompute(entity)
    local out = {}
    -- What this entity has, not what the game defines — otherwise `score` on a
    -- sword lists its willpower.
    for _, id in ipairs(present_of(entity)) do
        local def = r.defs[id]
        if def and (category == nil or def.category == category) then
            out[#out + 1] = {
                id = id, label = def.label, group = def.group, kind = def.kind,
                category = def.category,
                base = M.base(entity, id), value = values[id] or def.default,
                hidden = def.hidden,
                max = type(def.max) == "string" and values[def.max] or def.max,
                failed = r.failed[id],
            }
        end
    end
    return out
end

--- Which categories this entity's traits fall into, sorted. What an admin
--- command lists so a mis-categorised trait has somewhere to show up.
--- @param entity table
--- @return table  array of category names
function M.categories(entity)
    local r = root()
    local seen, out = {}, {}
    for _, id in ipairs(present_of(entity)) do
        local def = r.defs[id]
        if def and not seen[def.category] then
            seen[def.category] = true
            out[#out + 1] = def.category
        end
    end
    table.sort(out)
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

--- Prepare an entity's stats for use, without deciding what it should have.
---
--- Two jobs, and neither of them is materialising a stat block: drop anything
--- stored for a trait that is now derived, and clamp the gauges the entity
--- actually holds into their current bounds. Cheap enough to run on every item
--- instance, which is why seeding is a separate call.
---
--- The first is a real migration. `max_hp` used to be a stored number; it is a
--- derived trait now, so a saved value would shadow the formula forever.
function M.attach(entity)
    local r = root()
    if not r.order then M.seal() end
    local stats = stats_of(entity); if not stats then return false end
    stats._at = stats._at or {}

    -- Over the entity's own keys, not the registry's — an item must not pay for
    -- every skill the game has ever defined. Collected first because clearing a
    -- key mid-traversal is a rule not worth relying on.
    local stale = {}
    for id in pairs(stats) do
        local def = r.defs[id]
        if def and def.kind == "derived" then stale[#stale + 1] = id end
    end
    for _, id in ipairs(stale) do stats[id] = nil end

    M.bump(entity)

    -- Bounds may have moved while the character was away — a level-up, an item
    -- gone from a slot — so clamp what is here into its current range.
    _settling = true
    local now = os_time()
    for _, id in ipairs(present_of(entity)) do
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

--- Give an entity the traits a named set says it starts with.
---
--- This is the only thing that makes a trait exist for an entity that has never
--- stored one. After it runs, storage is the truth — which is why seeding is a
--- creation-time convenience rather than a filter consulted on every read.
--- A skill is deliberately in no set: not having swordsmanship until you learn
--- it is the point.
--- @param entity table
--- @param set string|nil  defaults to "character"
--- @return number  how many traits were written
function M.seed(entity, set)
    local r = root()
    if not r.order then M.seal() end
    local stats = stats_of(entity); if not stats then return 0 end
    set = type(set) == "string" and set or "character"

    local written = 0
    for id, def in pairs(r.defs) do
        if def.kind ~= "derived" and def.sets[set] and type(stats[id]) ~= "number" then
            stats[id] = def.default
            written = written + 1
        end
    end

    M.attach(entity)
    return written
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
