-- mudlib/daemons/effect_d.lua — Buffs, debuffs, and the events that run through them.
--
-- An Effect is a temporary thing attached to an entity that gets a say in what
-- happens to it. When the game is about to do something — deal damage, award
-- experience, work out how fast health returns — it hands the numbers to
-- `run`, every effect that cares gets to change them, and the game uses what
-- comes back.
--
--   local ev = DAEMON.effect.run(target, "damage_taken", { amount = 30 })
--   if not ev.cancelled then target:take_damage(ev.amount) end
--
-- That is the whole idea. It replaces storing a modifier on the thing being
-- modified, which is the design this is a reaction to: a stored `mod` has to be
-- added when the buff lands and subtracted when it leaves, and any path that
-- misses the subtraction leaves the character permanently wrong. Here nothing
-- is ever stored on the trait — remove the effect and the number simply stops
-- being computed that way.
--
-- Two halves, and keeping them apart is what makes the system saveable:
--
--   definitions   live in code (game/effects/*.lua). They hold functions.
--                 Never written anywhere.
--   instances     live in the cache. Plain data — nine fields, no functions,
--                 no metatables — so they survive both `Player._deep_copy`
--                 and the trip through JSON.
--
-- Ordering is by phase, not by registration, because "-15% damage" and
-- "-5 damage" give different answers depending on which goes first and nobody
-- should have to know which effect landed first to predict their own damage.
-- See mudlib/lib/effects.lua for the phases.
--
-- Exposes:
--   DAEMON.effect.define(spec) / define_all(list) / defs() / get_def(id)
--   DAEMON.effect.apply(entity, def_id, opts)   -> instance | false
--   DAEMON.effect.remove(entity, key_or_def, opts)
--   DAEMON.effect.clear(entity, opts)
--   DAEMON.effect.active(entity)                -> array of live instances
--   DAEMON.effect.has(entity, def_id)
--   DAEMON.effect.run(entity, hook, ev)         -> ev
--   DAEMON.effect.modify(entity, hook, amount)  -> number
--   DAEMON.effect.set_source_effects(entity, source, specs)
--   DAEMON.effect.sweep() / heartbeat()
--   DAEMON.effect.attach(entity) / detach(entity)
--
-- See docs/src/lua-api/effects.md.

local efflib  = require('lib.effects')
local persist = require('lib.persist')

local M = {}

local ROOT_KEY  = "effect_d"
local NS_SAVED  = "effects"
local NS_FAST   = "effects_fast"

local STACK_MODES = {
    refresh = true, stack = true, independent = true, ignore = true, replace = true,
}

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

local function log_warn(message)
    log("warn", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.warn, message) end
end

--- Definitions come from the game layer, so like traits they have to outlive a
--- reload of this file. Cached in an upvalue for the same reason TRAIT_D caches
--- its own: `get_persistent` crosses into Rust, and the pipeline consults this
--- on every event.
local S = nil

local function root()
    if S then return S end
    S = persist.root(ROOT_KEY, 1, function()
        return { defs = {}, gen = 1, seq = 0 }
    end)
    return S
end

-- Per-scope handler index, rebuilt on demand. Module-level: it is derived from
-- the definitions and the live instances, so losing it costs one rebuild.
local _index = {}
-- scope -> entity, so the sweep and the heartbeat can reach the objects they
-- have to act on. Weak values: a despawned mob must not be kept alive by this.
local _entities = setmetatable({}, { __mode = "v" })
--- Pipelines in flight, keyed `"<scope>|<hook>"`. Already per-entity, because
--- the scope is `char:<id>` or `obj:<id>` — two players running the same hook do
--- not collide, and never did.
local _running = {}

--- Pipeline nesting depth, **per entity scope**.
---
--- This was one global counter, which is fine while exactly one dispatch is ever
--- in flight and wrong the moment one can be suspended: a coroutine paused three
--- levels deep leaves the count at three, so the next player's pipeline trips
--- the cap after five levels instead of eight, and the decrement on resume
--- belongs to whoever happens to be running then.
---
--- Entries are removed at zero rather than left at zero, so this cannot grow
--- without bound across every entity the game has ever run an effect on.
local _depth = {}

-- ─── Storage ─────────────────────────────────────────────────────────────────

do
    if DAEMON and DAEMON.cache then
        --- Effects that should still be running after a restart.
        ---
        --- `min_lifetime` is what makes a short buff free: an effect with less
        --- than 30 seconds left is held in memory and never written, because
        --- by the time the server is back it would have expired anyway.
        --- Writing it would not be expensive so much as wrong.
        DAEMON.cache.define(NS_SAVED, {
            tier              = "write_behind",
            flush_seconds     = 30,
            min_lifetime      = 30,
            delete_when_empty = true,
            preload           = true,
            expiry_of         = function(_, v)
                return type(v) == "table" and v.expires or nil
            end,
        })
        --- Effects that are rebuilt from their source rather than saved:
        --- equipment, room auras, anything on a mob.
        DAEMON.cache.define(NS_FAST, {
            tier      = "memory",
            expiry_of = function(_, v)
                return type(v) == "table" and v.expires or nil
            end,
        })
    else
        log("error", "EFFECT_D: cache_d is not loaded — effects will not work")
    end
end

--- Where this entity's effects live. Players are keyed by character, anything
--- else by object id, so a mob's effects can never collide with a player's.
function M.scope(entity)
    if type(entity) ~= "table" then return nil end
    if entity.char_id then return "char:" .. tostring(entity.char_id) end
    if entity.id then return "obj:" .. tostring(entity.id) end
    return nil
end

local function ns_for(def)
    return def.persist == false and NS_FAST or NS_SAVED
end

--- Both namespaces, oldest-first order irrelevant — callers sort by key.
local function each_store(scope, fn)
    if not (DAEMON and DAEMON.cache) then return end
    for _, ns in ipairs({ NS_SAVED, NS_FAST }) do
        local live = DAEMON.cache.get_scope(ns, scope)
        if live then fn(ns, live) end
    end
end

-- ─── Definitions ─────────────────────────────────────────────────────────────

--- Turn `modifiers = { strength = 2, max_hp = "+10%" }` into ordinary handlers.
---
--- Authors get a table; the runtime gets one mechanism. A flat number becomes
--- an `add` handler, a percentage becomes a `mult` handler, and both scale with
--- the stack count so a stacking buff does not need to say so twice.
local function desugar_modifiers(def)
    for trait_id, amount in pairs(def.modifiers or {}) do
        local hook = "trait:" .. trait_id
        if type(amount) == "number" then
            def.hooks[hook .. "#mod"] = {
                hook = hook, phase = "add", order = 0,
                fn = function(ev, ctx)
                    ev.amount = ev.amount + amount * (ctx.stacks or 1)
                end,
            }
        elseif type(amount) == "string" then
            local pct = tonumber(amount:match("^([%+%-]?%d+%.?%d*)%%$"))
            if pct then
                def.hooks[hook .. "#mod"] = {
                    hook = hook, phase = "mult", order = 0,
                    fn = function(ev, ctx)
                        ev.scale = ev.scale + (pct / 100) * (ctx.stacks or 1)
                    end,
                }
            else
                log_warn("EFFECT_D.define('" .. def.id .. "'): modifier for '" .. trait_id
                    .. "' should be a number or a percentage like \"+10%\", got " .. amount)
            end
        end
    end
    def.modifiers = nil
end

--- A gauge or a counter stores what it is. Only computed traits — attributes
--- and derived ones — can be modified, and catching the mistake at definition
--- time is much kinder than a buff that silently does nothing.
local function reject_stored_trait_hooks(def)
    if not (DAEMON and DAEMON.trait and DAEMON.trait.get_def) then return true end
    for _, handler in pairs(def.hooks) do
        local trait_id = tostring(handler.hook or ""):match("^trait:(.+)$")
        if trait_id then
            local tdef = DAEMON.trait.get_def(trait_id)
            if tdef and (tdef.kind == "gauge" or tdef.kind == "counter") then
                log_error("EFFECT_D.define('" .. def.id .. "'): cannot modify trait '"
                    .. trait_id .. "' — it is a " .. tdef.kind
                    .. ", which is changed by events, not by modifiers. "
                    .. "To raise a gauge's ceiling, modify the trait it uses as its max.")
                return false
            end
        end
    end
    return true
end

--- @return boolean
function M.define(spec)
    if type(spec) ~= "table" or type(spec.id) ~= "string" or #spec.id == 0 then
        log_warn("EFFECT_D.define: an effect needs a string id")
        return false
    end
    local stack = spec.stack or "refresh"
    if not STACK_MODES[stack] then
        log_warn("EFFECT_D.define('" .. spec.id .. "'): unknown stack mode '" .. tostring(stack)
            .. "' — expected refresh, stack, independent, ignore or replace")
        return false
    end

    local def = {
        id             = spec.id,
        label          = spec.label or spec.id,
        desc           = spec.desc,
        duration       = spec.duration,
        stack          = stack,
        max_stacks     = spec.max_stacks or 1,
        persist        = spec.persist,
        survives_death = spec.survives_death or false,
        tick           = spec.tick,
        condition      = spec.condition,
        potency        = spec.potency,
        hooks          = {},
        on_apply       = spec.on_apply,
        on_refresh     = spec.on_refresh,
        on_expire      = spec.on_expire,
        modifiers      = spec.modifiers,
    }

    -- A definition may register several handlers for the same hook in
    -- different phases; the table key is just a name, and `hook` says what it
    -- listens to. `damage_taken` and `damage_taken#flat` are one effect
    -- reducing damage twice, by a percentage and then by a flat amount.
    for name, handler in pairs(spec.hooks or {}) do
        if type(handler) == "function" then
            handler = { fn = handler }
        end
        if type(handler) ~= "table" or type(handler.fn) ~= "function" then
            log_warn("EFFECT_D.define('" .. spec.id .. "'): hook '" .. tostring(name)
                .. "' needs a function")
        else
            local hook = handler.hook or tostring(name):gsub("#.*$", "")
            if handler.phase and not efflib.valid_phase(handler.phase) then
                log_warn("EFFECT_D.define('" .. spec.id .. "'): hook '" .. tostring(name)
                    .. "' has unknown phase '" .. tostring(handler.phase) .. "'")
            end
            def.hooks[name] = {
                hook  = hook,
                phase = handler.phase or efflib.DEFAULT_PHASE,
                order = handler.order or 0,
                fn    = handler.fn,
            }
        end
    end

    desugar_modifiers(def)
    if not reject_stored_trait_hooks(def) then return false end

    local r = root()
    r.defs[spec.id] = def
    r.gen = r.gen + 1
    _index = {}
    if DAEMON and DAEMON.trait then pcall(DAEMON.trait.bump_all) end
    return true
end

function M.define_all(list)
    if type(list) ~= "table" then
        log_warn("EFFECT_D.define_all: expected an array of specs")
        return 0
    end
    local n = 0
    for _, spec in ipairs(list) do
        if M.define(spec) then n = n + 1 end
    end
    return n
end

function M.defs()       return root().defs end
function M.get_def(id)  return root().defs[id] end

-- ─── Instances ───────────────────────────────────────────────────────────────

--- What identifies "the same effect" for stacking purposes.
local function dedupe_key(def, source)
    if def.stack == "refresh" then
        return def.id .. "|" .. tostring(source or "")
    end
    if def.stack == "independent" then
        local r = root()
        r.seq = r.seq + 1
        return def.id .. "#" .. r.seq
    end
    return def.id
end

local function invalidate(entity, scope)
    _index[scope] = nil
    if DAEMON and DAEMON.trait then DAEMON.trait.bump(entity) end
end

local function fire(def, name, entity, inst, extra)
    local hook = def[name]
    if type(hook) ~= "function" then return end
    local ctx = {
        entity = entity, inst = inst, def = def,
        stacks = inst.stacks or 1,
        potency = inst.potency or def.potency,
        reason = extra and extra.reason,
        ticks = extra and extra.ticks,
    }
    local ok, err = pcall(hook, ctx)
    if not ok then
        log_error("EFFECT_D: " .. name .. " for '" .. def.id .. "' failed: " .. tostring(err))
    end
end

--- Put an effect on an entity.
--- @param opts table|nil  { source, duration, potency, stacks, caster, state }
--- @return table|false  the instance
function M.apply(entity, def_id, opts)
    opts = opts or {}
    local def = root().defs[def_id]
    if not def then
        log_warn("EFFECT_D.apply: no such effect '" .. tostring(def_id) .. "'")
        return false
    end
    local scope = M.scope(entity)
    if not scope or not (DAEMON and DAEMON.cache) then return false end

    -- A condition that is false now means the effect never lands, which is
    -- different from landing and expiring immediately.
    if def.condition then
        local ok, pass, reason = pcall(def.condition, entity)
        if ok and not pass then return false, reason end
    end

    _entities[scope] = entity
    local ns  = ns_for(def)
    local now = os_time()
    local duration = opts.duration or def.duration
    local key = dedupe_key(def, opts.source)
    local existing = DAEMON.cache.get(ns, scope, key)

    if existing and def.stack == "ignore" then
        return false, "already affected"
    end
    if existing and def.stack == "replace" then
        fire(def, "on_expire", entity, existing, { reason = "replaced" })
        DAEMON.cache.delete(ns, scope, key)
        existing = nil
    end

    local inst
    if existing then
        inst = existing
        if def.stack == "stack" then
            inst.stacks = math.min((inst.stacks or 1) + (opts.stacks or 1), def.max_stacks)
        end
        -- Never shorten an effect by re-applying it: a weaker second cast must
        -- not cut the first one short.
        if duration then
            inst.expires = math.max(inst.expires or 0, now + duration)
        else
            inst.expires = nil
        end
        if opts.potency then inst.potency = opts.potency end
        fire(def, "on_refresh", entity, inst)
    else
        inst = {
            def    = def.id,
            start  = now,
            source = opts.source,
        }
        if duration then inst.expires = now + duration end
        if opts.stacks and opts.stacks > 1 then
            inst.stacks = math.min(opts.stacks, def.max_stacks)
        end
        if opts.potency then inst.potency = opts.potency end
        if opts.caster then inst.caster = opts.caster end
        if opts.state then inst.state = opts.state end
        if def.tick then inst.last_tick = now end
    end

    local stored = DAEMON.cache.set(ns, scope, key, inst, { expires_at = inst.expires })
    if not stored then return false end

    if not existing then fire(def, "on_apply", entity, inst) end
    invalidate(entity, scope)
    return inst
end

--- Take an effect off. Accepts an instance key or a definition id (which
--- removes every instance of it, however it was stacked).
--- @return number  how many were removed
function M.remove(entity, key_or_def, opts)
    local scope = M.scope(entity)
    if not scope or not (DAEMON and DAEMON.cache) then return 0 end
    opts = opts or {}
    local removed = 0

    each_store(scope, function(ns, live)
        local doomed = {}
        for key, inst in pairs(live) do
            if key == key_or_def or inst.def == key_or_def then
                doomed[#doomed + 1] = key
            end
        end
        for _, key in ipairs(doomed) do
            local inst = live[key]
            local def = root().defs[inst.def]
            if def then fire(def, "on_expire", entity, inst, { reason = opts.reason or "removed" }) end
            DAEMON.cache.delete(ns, scope, key)
            removed = removed + 1
        end
    end)

    if removed > 0 then invalidate(entity, scope) end
    return removed
end

--- Take everything off. `opts.keep_survivors` honours `survives_death`.
function M.clear(entity, opts)
    opts = opts or {}
    local scope = M.scope(entity)
    if not scope or not (DAEMON and DAEMON.cache) then return 0 end
    local removed = 0

    each_store(scope, function(ns, live)
        local doomed = {}
        for key, inst in pairs(live) do
            local def = root().defs[inst.def]
            local survives = opts.keep_survivors and def and def.survives_death
            if not survives then doomed[#doomed + 1] = key end
        end
        for _, key in ipairs(doomed) do
            local inst = live[key]
            local def = root().defs[inst.def]
            if def then fire(def, "on_expire", entity, inst, { reason = opts.reason or "cleared" }) end
            DAEMON.cache.delete(ns, scope, key)
            removed = removed + 1
        end
    end)

    if removed > 0 then invalidate(entity, scope) end
    return removed
end

--- Everything currently running, expired ones dropped on the way past.
---
--- Expiry is checked here as well as by the sweep because the two answer
--- different questions: this one keeps a *read* honest, the sweep is what makes
--- `on_expire` fire for a player who is typing nothing.
--- @return table  array of { key, ns, inst, def }, in a stable order
function M.active(entity)
    local out = {}
    local scope = M.scope(entity)
    if not scope or not (DAEMON and DAEMON.cache) then return out end

    _entities[scope] = entity
    local now = os_time()
    local expired = {}

    each_store(scope, function(ns, live)
        for key, inst in pairs(live) do
            local def = root().defs[inst.def]
            if not def then
                -- The definition went away — a reload dropped it, or the game
                -- layer stopped defining it. Nothing can interpret this
                -- instance any more, so it goes.
                expired[#expired + 1] = { ns = ns, key = key, inst = inst, def = nil }
            elseif inst.expires and inst.expires <= now then
                expired[#expired + 1] = { ns = ns, key = key, inst = inst, def = def }
            else
                out[#out + 1] = { key = key, ns = ns, inst = inst, def = def }
            end
        end
    end)

    for _, dead in ipairs(expired) do
        if dead.def then
            fire(dead.def, "on_expire", entity, dead.inst, { reason = "timeout" })
        end
        DAEMON.cache.delete(dead.ns, scope, dead.key)
    end
    if #expired > 0 then invalidate(entity, scope) end

    -- Sorted so that two entities with the same effects always resolve them in
    -- the same order. Nothing here may depend on `pairs`.
    table.sort(out, function(a, b) return a.key < b.key end)
    return out
end

function M.has(entity, def_id)
    for _, e in ipairs(M.active(entity)) do
        if e.inst.def == def_id then return true end
    end
    return false
end

--- When does the next effect on this entity run out? nil if none ever will.
--- TRAIT_D caches this so a single comparison can tell it whether the clock
--- has invalidated its memo.
--- Is a cached index still the right answer?
---
--- Generation covers everything that was *done* to the entity. The expiry
--- comparison covers the one thing nobody does: time passing. Without it a
--- cached index keeps handing out handlers belonging to an effect that ran out
--- minutes ago, and the only way anyone notices is that the numbers are wrong.
local function index_fresh(idx)
    if not idx then return false end
    if idx.gen ~= root().gen then return false end
    if idx.next_expiry and os_time() >= idx.next_expiry then return false end
    return true
end

function M.next_expiry(entity)
    local scope = M.scope(entity)
    if not scope then return nil end
    local idx = _index[scope]
    if not index_fresh(idx) then idx = M._rebuild(entity, scope) end
    return idx and idx.next_expiry or nil
end

-- ─── The pipeline ────────────────────────────────────────────────────────────

--- Build the per-scope handler index: hook name -> sorted handlers, or false
--- for "nothing listens to this".
function M._rebuild(entity, scope)
    local hooks, next_expiry = {}, nil
    local i = 0
    for _, e in ipairs(M.active(entity)) do
        i = i + 1
        if e.inst.expires then
            next_expiry = next_expiry and math.min(next_expiry, e.inst.expires) or e.inst.expires
        end
        for _, handler in pairs(e.def.hooks) do
            local list = hooks[handler.hook]
            if not list then list = {}; hooks[handler.hook] = list end
            list[#list + 1] = {
                phase = handler.phase,
                order = handler.order,
                def   = e.def.id,
                index = i,
                fn    = handler.fn,
                ctx   = {
                    entity  = entity,
                    inst    = e.inst,
                    def     = e.def,
                    stacks  = e.inst.stacks or 1,
                    potency = e.inst.potency or e.def.potency,
                    key     = e.key,
                },
            }
        end
    end
    for _, list in pairs(hooks) do efflib.sort_handlers(list) end

    local idx = { gen = root().gen, hooks = hooks, next_expiry = next_expiry }
    _index[scope] = idx
    return idx
end

local function handlers_for(entity, hook)
    local scope = M.scope(entity)
    if not scope then return nil end
    local idx = _index[scope]
    if not index_fresh(idx) then idx = M._rebuild(entity, scope) end
    local list = idx.hooks[hook]
    if not list or #list == 0 then return nil end
    return list, scope
end

--- Run an event through this entity's effects and return it.
---
--- The event is a plain table the handlers mutate. `amount` is the working
--- number, `scale` accumulates multipliers, and setting `cancelled` stops the
--- rest of the chain. When nothing listens — the usual case — this returns the
--- table it was given, untouched and without allocating.
function M.run(entity, hook, ev)
    ev = ev or {}
    local handlers, scope = handlers_for(entity, hook)
    if not handlers then return ev end

    -- Regeneration heals, healing is an event, and an effect could listen to
    -- it and heal again. Refusing is better than a stack overflow, and much
    -- better than a silent infinite loop on the game thread.
    local guard = scope .. "|" .. hook
    if _running[guard] then
        log_warn("EFFECT_D: '" .. hook .. "' re-entered for " .. scope .. " — refusing to recurse")
        return ev
    end
    local depth = _depth[scope] or 0
    if depth > 8 then
        log_warn("EFFECT_D: effect chain deeper than 8 at '" .. hook .. "' — stopping")
        return ev
    end

    _running[guard] = true
    _depth[scope] = depth + 1

    for _, h in ipairs(handlers) do h.ctx.entity = entity end
    local ok, err = pcall(efflib.dispatch, ev, handlers, function(e, h)
        log_error("EFFECT_D: handler from '" .. tostring(h.def) .. "' on '" .. hook
            .. "' failed: " .. tostring(e))
    end)
    if not ok then
        log_error("EFFECT_D: pipeline for '" .. hook .. "' failed: " .. tostring(err))
    end

    local left = (_depth[scope] or 1) - 1
    _depth[scope] = left > 0 and left or nil
    _running[guard] = nil
    return ev
end

--- The cheap form, for a single number with no context.
---
--- This is the one TRAIT_D calls on every trait of every recompute, so the
--- no-effects path must not allocate: it returns the number it was handed.
function M.modify(entity, hook, amount)
    if type(amount) ~= "number" then return amount end
    if not handlers_for(entity, hook) then return amount end
    local ev = M.run(entity, hook, { amount = amount, scale = 0 })
    return type(ev.amount) == "number" and ev.amount or amount
end

-- ─── Sources ─────────────────────────────────────────────────────────────────

--- The effects from `source` are now exactly `specs`, and nothing else.
---
--- Idempotent by design: it adds what is new, removes what is gone, and leaves
--- what is unchanged alone. That is what lets equipment call it on every login
--- and every slot change without accumulating duplicates or having to work out
--- what it did last time.
--- @param specs table  array of { def = "id", potency = n, duration = n, ... }
--- @return number added, number removed
function M.set_source_effects(entity, source, specs)
    local scope = M.scope(entity)
    if not scope or not (DAEMON and DAEMON.cache) then return 0, 0 end
    specs = specs or {}

    local wanted = {}
    for _, spec in ipairs(specs) do wanted[spec.def] = spec end

    local removed = 0
    for _, e in ipairs(M.active(entity)) do
        if e.inst.source == source and not wanted[e.inst.def] then
            DAEMON.cache.delete(e.ns, scope, e.key)
            local def = root().defs[e.inst.def]
            if def then fire(def, "on_expire", entity, e.inst, { reason = "source removed" }) end
            removed = removed + 1
        end
    end

    local added = 0
    for _, spec in ipairs(specs) do
        local already = false
        for _, e in ipairs(M.active(entity)) do
            if e.inst.source == source and e.inst.def == spec.def then already = true end
        end
        if not already then
            local opts = {}
            for k, v in pairs(spec) do opts[k] = v end
            opts.def = nil
            opts.source = source
            if M.apply(entity, spec.def, opts) then added = added + 1 end
        end
    end

    if added > 0 or removed > 0 then invalidate(entity, scope) end
    return added, removed
end

-- ─── Tickers ─────────────────────────────────────────────────────────────────

--- Expire what has run out, for everyone.
---
--- `active` already expires lazily, so this exists for the side effects: an
--- effect that ends while its owner is standing still must still say so, and
--- still stop modifying their numbers.
--- @return number  entities swept
function M.sweep()
    local n = 0
    for scope, entity in pairs(_entities) do
        if entity then
            local ok, err = pcall(M.active, entity)
            if not ok then
                log_error("EFFECT_D: sweep failed for " .. scope .. ": " .. tostring(err))
            end
            n = n + 1
        end
    end
    return n
end

--- Drive effects that do something on a timer.
---
--- Each instance carries `last_tick` and earns whole ticks from it, advancing
--- by exactly the ticks it fired — the same carry-the-remainder rule
--- regeneration uses, so a coarse heartbeat loses no accuracy and a heartbeat
--- that skipped a beat catches up rather than losing the time.
--- @return number  handlers fired
function M.heartbeat()
    local now = os_time()
    local fired = 0

    for scope, entity in pairs(_entities) do
        if entity then
            local ok, err = pcall(function()
                for _, e in ipairs(M.active(entity)) do
                    local tick = e.def.tick
                    if tick and tick > 0 then
                        local last = e.inst.last_tick or e.inst.start or now
                        local ticks = math.floor((now - last) / tick)
                        if ticks >= 1 then
                            e.inst.last_tick = last + ticks * tick
                            M.run(entity, "heartbeat", {
                                amount = 0, scale = 0, ticks = ticks,
                                effect = e.inst.def, key = e.key,
                            })
                            fired = fired + 1
                        end
                    end
                end
            end)
            if not ok then
                log_error("EFFECT_D: heartbeat failed for " .. scope .. ": " .. tostring(err))
            end
        end
    end
    return fired
end

-- ─── Lifecycle ───────────────────────────────────────────────────────────────

--- Bring an entity's effects into memory and make it reachable from the tickers.
function M.attach(entity)
    local scope = M.scope(entity)
    if not scope then return false end
    _entities[scope] = entity
    if DAEMON and DAEMON.cache then DAEMON.cache.preload(scope) end
    _index[scope] = nil
    M.active(entity)   -- drop anything that expired while they were away
    return true
end

--- Write what should be kept, forget the rest.
function M.detach(entity)
    local scope = M.scope(entity)
    if not scope then return false end
    if DAEMON and DAEMON.cache then
        pcall(DAEMON.cache.flush, NS_SAVED, scope)
        pcall(DAEMON.cache.drop, NS_SAVED, scope)
        pcall(DAEMON.cache.drop, NS_FAST, scope)
    end
    _index[scope] = nil
    _entities[scope] = nil
    return true
end

-- ─── Ticker registration ─────────────────────────────────────────────────────

do
    local function every(key, default, id, fn)
        local ok, v = pcall(config, key)
        local interval = (ok and type(v) == "number") and v or default
        if interval > 0 and DAEMON and DAEMON.ticker then
            DAEMON.ticker.every(interval, id, fn)
        end
    end

    every("game.effect_sweep_seconds", 5, "effect.sweep", function()
        if DAEMON and DAEMON.effect then
            local ok, err = pcall(DAEMON.effect.sweep)
            if not ok then log("error", "EFFECT_D: sweep tick failed: " .. tostring(err)) end
        end
    end)

    every("game.effect_heartbeat_seconds", 3, "effect.heartbeat", function()
        if DAEMON and DAEMON.effect then
            local ok, err = pcall(DAEMON.effect.heartbeat)
            if not ok then log("error", "EFFECT_D: heartbeat tick failed: " .. tostring(err)) end
        end
    end)
end

log("info", "effect_d daemon loaded")

return M
