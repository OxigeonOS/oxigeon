-- mudlib/daemons/ability_d.lua — What a character can *do*, as data.
--
-- `spell_d` was 177 lines arranging five things the mudlib already had — gauges,
-- effects, the damage pipeline, cooldowns and the trait graph — into "casting".
-- Every game wanting a different arrangement rewrote those 177 lines, and every
-- spell inside it was a hand-written Lua function.
--
-- This is the arrangement, moved into the mudlib, so a designer writes a data
-- bag instead:
--
--     { id = "emberlance", category = "spell", open = true,
--       cost = { mp = 8 }, cooldown = 4, target = "creature",
--       damage = { min = 8, max = 8, type = "fire",
--                  scale = { trait = "spell_power", per = 2 } },
--       messages = { self = "{red}You draw a line of fire at $target.{/}" } }
--
-- No Lua. `run = function(ctx)` is still there for the things that genuinely are
-- programs, and one of the four shipped spells still uses it.
--
-- ─── It adds no new machinery ────────────────────────────────────────────────
--
-- Costs are gauges spent through `trait_d`. Outcomes go through
-- `Mobile:take_damage` / `heal`, so armour, resists and the effect pipeline
-- apply to an ability exactly as they do to a sword. Gates are `cooldown_d`.
-- **A channel is an `effect_d` instance.** A fight an ability starts is
-- `combat_d`'s. A cast in flight is one `ticker_d` timer and one memory-tier
-- cache key.
--
-- ─── The order, and what it costs you ────────────────────────────────────────
--
--    1 known           6 cooldowns (own, shared, global)
--    2 rank            7 resolve the target
--    3 not busy        8 target requirements
--    4 requirements    9 can you afford it
--    5 ───────────────── nothing has been spent to this point ─────────────────
--   10 cast time or channel: begin, and return
--   11 commit: spend, mark
--   12 outcomes
--
-- Step 7 after step 6 and before step 9 is `spell_d`'s discipline kept
-- verbatim: **the target is resolved before anything is spent, so a mistyped
-- name does not cost mana.**
--
-- For a cast that spans time: **cost at the start, the ability's own cooldown at
-- completion, the global cooldown at the start.** Cost at the start because that
-- is what makes an interrupt *cost* something — a cast you can begin for free
-- and abort for free is not a risk, it is a free oracle. Cooldown at completion
-- because a cooldown rate-limits the outcome, not the attempt, and marking it up
-- front punishes an interrupted cast twice. The GCD at the start because it is
-- the one gate that exists to rate-limit *inputs*.
--
-- Nothing is refunded on an interrupt. A game that wants a partial refund writes
-- three lines of `trait.adjust` in `on_interrupt`; the mudlib takes no position
-- on what is policy.
--
-- Exposes:
--   DAEMON.ability.define(spec) / define_all(list) / get(id) / all() / defs()
--   DAEMON.ability.define_check(kind, fn, opts)
--   DAEMON.ability.known(entity, category)  -> array of { id, spec, rank, sources }
--   DAEMON.ability.knows(entity, id) / rank(entity, id)
--   DAEMON.ability.grant(entity, id, opts) / revoke(entity, id_or_source, opts)
--   DAEMON.ability.set_source_abilities(entity, source, specs) -> added, removed
--   DAEMON.ability.use(user, id, opts)      -> ok, why
--   DAEMON.ability.casting(entity)          -> record | nil
--   DAEMON.ability.cancel(entity, reason)   -> boolean
--   DAEMON.ability.on_damaged(entity, amount, opts) / on_moved(char_id)
--   DAEMON.ability.cleanup(char_id) / detach(entity)
--   DAEMON.ability.sweep()
--
-- See docs/src/lua-api/abilities.md.

local Abilities = require('lib.abilities')

local M = {}

M._abilities = {}

--- Which generated channel effects exist already. One definition per channelling
--- ability, made on first use — the same trick `lib/equipment.lua` uses for
--- `equip_trait_<id>`, and for the same reason: an effect's hooks are fixed at
--- define time, so a per-subject definition is the established answer.
local _channels = {}

local GRANTS      = "abilities"
local GRANTS_FAST = "abilities_fast"
local CASTS       = "ability_cast"

local GCD_KEY = "ability.gcd"

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

local function log_warn(message)
    log("warn", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.warn, message) end
end

do
    if DAEMON and DAEMON.cache then
        -- A bare grant — a quest reward, a tome read. Nothing rebuilds it, so it
        -- has to be written.
        DAEMON.cache.define(GRANTS, {
            tier              = "write_behind",
            flush_seconds     = 30,
            scope_prefix      = "char:",
            owner             = "char",
            preload           = true,
            delete_when_empty = true,
        })
        -- Anything reconciled from a source that is itself saved: equipment, a
        -- form, later a class. What is worn is saved; the grant is *derived* from
        -- it, and a derived copy is the only one that can be wrong. `owner` is
        -- none because the scope space is shared with creatures.
        DAEMON.cache.define(GRANTS_FAST, { tier = "memory", owner = "none" })
        -- A cast in flight when the server dies is over. Memory is also the only
        -- tier that may hold a live entity reference, and the record must hold
        -- the *resolved* target — re-resolving by name at completion would let
        -- somebody retarget mid-cast by having a matching creature walk in.
        -- `combat_d`'s `combat` namespace is the precedent.
        DAEMON.cache.define(CASTS, { tier = "memory", owner = "none" })
    else
        log("error", "ABILITY_D: cache_d is not loaded — abilities will not work")
    end
end

local function conf(key, default)
    local ok, v = pcall(config, key)
    if ok and type(v) == "number" then return v end
    return default
end

--- `char:<id>` for a player, `obj:<id>` for a creature. The same convention
--- `effect_d` and `combat_d` use, so a mob and a player are one code path.
local function scope_of(entity)
    if type(entity) ~= "table" then return nil end
    if entity.char_id ~= nil then return "char:" .. tostring(entity.char_id) end
    if entity.id ~= nil then return "obj:" .. tostring(entity.id) end
    return nil
end

local function tell(entity, text)
    if text == nil or text == "" then return end
    if type(entity) == "table" and type(entity.send) == "function" then
        pcall(entity.send, entity, text)
    end
end

local function tell_room(entity, text)
    if text == nil or text == "" then return end
    if type(entity) == "table" and type(entity.message_room) == "function" then
        pcall(entity.message_room, entity, text)
    end
end

--- Say what an ability is doing, in whichever of the two shapes it declares.
---
--- `messages.line` is one authored sentence rendered per reader — the actor sees
--- "You draw a line of fire at a pale wisp", the target sees "Wren draws a line
--- of fire at you", the room sees names throughout. `self`/`room`/`target` stay
--- for the case that genuinely is three different statements, where the actor is
--- told something the room must not hear.
local function announce(spec, ctx)
    local messaging = require('lib.messaging')

    if spec.messages.line ~= nil then
        pcall(messaging.announce, spec.messages.line, ctx)
        return
    end

    tell(ctx.user, Abilities.render(spec.messages.self, ctx, ctx.user))
    tell_room(ctx.user, Abilities.render(spec.messages.room, ctx, nil))
    if ctx.target and ctx.target ~= ctx.user then
        tell(ctx.target, Abilities.render(spec.messages.target, ctx, ctx.target))
    end
end

-- ─── Definitions ─────────────────────────────────────────────────────────────

--- Register one ability.
--- @param spec table
--- @return boolean
function M.define(spec)
    local normalised, err = Abilities.normalise(spec)
    if not normalised then
        log_warn("ABILITY_D.define: " .. tostring(err))
        return false
    end

    -- A cost against anything but a gauge is a modifier pretending to be a
    -- payment, which is the mistake the whole effect design exists to avoid.
    -- The mirror of `effect_d`'s refusal to modify a gauge, inverted.
    if DAEMON and DAEMON.trait and DAEMON.trait.get_def then
        for _, c in ipairs(normalised.cost) do
            local def = DAEMON.trait.get_def(c.trait)
            if def and def.kind ~= "gauge" then
                log_warn("ABILITY_D.define('" .. normalised.id .. "'): cost names '"
                    .. tostring(c.trait) .. "', which is a " .. tostring(def.kind)
                    .. " and not a gauge. A cost has to be something you can spend.")
                return false
            end
        end
        -- The same rule for `adjust`, and for the same reason: `trait.adjust`
        -- moves a *current* value, and only a gauge has one. Moving an attribute
        -- is what an effect is for, and doing it from here would be a permanent
        -- change wearing an action's clothes.
        for _, a in ipairs(normalised.adjust) do
            local def = DAEMON.trait.get_def(a.trait)
            if def and def.kind ~= "gauge" then
                log_warn("ABILITY_D.define('" .. normalised.id .. "'): adjust names '"
                    .. tostring(a.trait) .. "', which is a " .. tostring(def.kind)
                    .. " and not a gauge. To change an attribute, apply an effect.")
                return false
            end
        end
    end

    M._abilities[normalised.id] = normalised
    return true
end

function M.define_all(list)
    local n = 0
    for _, spec in ipairs(list or {}) do
        if M.define(spec) then n = n + 1 end
    end
    log("info", "ABILITY_D: defined " .. n .. " abilit" .. (n == 1 and "y" or "ies"))
    return n
end

function M.get(id) return M._abilities[id] end

function M.all()
    local out = {}
    for id in pairs(M._abilities) do out[#out + 1] = id end
    table.sort(out)
    return out
end

M.defs = M.all

--- Add a requirement predicate from the game layer.
--- @param kind string
--- @param fn function  f(user, target, spec, ctx) -> ok, why
--- @param opts table|nil  { needs_target = true }
function M.define_check(kind, fn, opts)
    if type(kind) ~= "string" or type(fn) ~= "function" then
        log_warn("ABILITY_D.define_check: needs a string kind and a function")
        return false
    end
    Abilities.checks()[kind] = fn
    if opts and opts.needs_target then Abilities.set_needs_target(kind, true) end
    return true
end

-- ─── Ownership ───────────────────────────────────────────────────────────────

--- Forward-declared: `known` reports whether the gates pass, and the gates are
--- defined below beside the predicates they run.
local run_checks

local function grant_scopes(entity)
    local scope = scope_of(entity)
    return scope
end

--- Every grant on this entity, from both namespaces, as `{ [id] = record }`.
local function grants_of(entity)
    local out = {}
    if not (DAEMON and DAEMON.cache) then return out end

    local scope = grant_scopes(entity)
    if not scope then return out end

    if type(entity) == "table" and entity.char_id ~= nil then
        for _, key in ipairs(DAEMON.cache.keys(GRANTS, entity.char_id)) do
            local rec = DAEMON.cache.get(GRANTS, entity.char_id, key)
            if type(rec) == "table" then out[key] = rec end
        end
    end
    for _, key in ipairs(DAEMON.cache.keys(GRANTS_FAST, scope)) do
        local rec = DAEMON.cache.get(GRANTS_FAST, scope, key)
        if type(rec) == "table" then
            local prior = out[key]
            if prior then
                -- The higher rank of the two, for the same reason ranks fold by
                -- max everywhere else.
                out[key] = (tonumber(rec.rank) or 1) > (tonumber(prior.rank) or 1)
                    and rec or prior
            else
                out[key] = rec
            end
        end
    end
    return out
end

--- What this entity's rank in an ability is.
---
--- **`math.max` of the trait and every grant.** A sword granting Cleave at rank
--- 2 must not *reduce* a swordmaster already at 5, and picking the sword up must
--- not drop its floor for somebody at 0. It is the only rule that is right in
--- both directions and it needs no explanation.
--- @return number rank, table sources
function M.rank(entity, id)
    local spec = M._abilities[id]
    if not spec then return 0, {} end

    local rank, sources = 0, {}

    if spec.rank_trait and DAEMON and DAEMON.trait then
        if DAEMON.trait.has(entity, spec.rank_trait) then
            rank = math.max(rank, DAEMON.trait.value(entity, spec.rank_trait) or 0)
            sources[#sources + 1] = "trait:" .. spec.rank_trait
        end
    end

    local rec = grants_of(entity)[id]
    if rec then
        rank = math.max(rank, tonumber(rec.rank) or 1)
        sources[#sources + 1] = tostring(rec.source or "granted")
    end

    if #sources == 0 and spec.open then
        rank = math.max(rank, 1)
        sources[#sources + 1] = "open"
    end

    return rank, sources
end

--- Does this entity have this ability at all?
---
--- Presence, decided by storage and by an explicit grant — never by a list.
--- An ability with no `rank_trait`, no `open` and no grant is **not known**;
--- there is no implicit "everybody knows everything".
function M.knows(entity, id)
    local spec = M._abilities[id]
    if not spec then return false end
    if spec.open then return true end
    if spec.rank_trait and DAEMON and DAEMON.trait and DAEMON.trait.has(entity, spec.rank_trait) then
        return true
    end
    return grants_of(entity)[id] ~= nil
end

--- Everything this entity can do, folding both ownership models.
---
--- `usable` is the gates evaluated — everything that can be answered without a
--- target. A listing wants both halves: an ability you have but cannot use yet
--- should say *why*, and one you have not been taught at all should not appear.
--- @param category string|nil  filter
--- @return table  array of { id, spec, rank, sources, usable, why }, sorted by id
function M.known(entity, category)
    local out = {}
    for _, id in ipairs(M.all()) do
        local spec = M._abilities[id]
        if (not category or spec.category == category) and M.knows(entity, id) then
            local rank, sources = M.rank(entity, id)
            local ctx = { user = entity, ability = spec, rank = rank }
            local usable, why = run_checks(spec.pre_requires, entity, nil, ctx)
            if usable and rank < spec.min_rank then
                usable, why = false, "That is beyond you for now."
            end
            out[#out + 1] = {
                id = id, spec = spec, rank = rank, sources = sources,
                usable = usable and true or false, why = why,
            }
        end
    end
    return out
end

--- Hand somebody an ability.
--- @param opts table|nil { source, rank, expires, persist }
function M.grant(entity, id, opts)
    if not M._abilities[id] then
        log_warn("ABILITY_D.grant: no ability '" .. tostring(id) .. "'")
        return false
    end
    if not M._abilities[id].grantable then
        log_warn("ABILITY_D.grant('" .. id .. "'): this ability is not grantable")
        return false
    end
    if not (DAEMON and DAEMON.cache) then return false end

    opts = opts or {}
    local record = {
        rank    = tonumber(opts.rank) or 1,
        source  = opts.source or "granted",
        expires = opts.expires,
    }

    -- Durable by default: a bare grant is a quest reward or a tome, and nothing
    -- rebuilds it. `set_source_abilities` passes `persist = false`, because what
    -- it derives from is itself saved.
    local durable = opts.persist
    if durable == nil then durable = true end

    if durable and entity.char_id ~= nil then
        return DAEMON.cache.set(GRANTS, entity.char_id, id, record,
            opts.expires and { expires_at = opts.expires } or nil)
    end
    local scope = scope_of(entity)
    if not scope then return false end
    return DAEMON.cache.set(GRANTS_FAST, scope, id, record,
        opts.expires and { expires_at = opts.expires } or nil)
end

--- Take one back, by ability id or by the source that granted it.
--- @return number  how many went
function M.revoke(entity, id_or_source, opts)
    if not (DAEMON and DAEMON.cache) then return 0 end
    local scope = scope_of(entity)
    if not scope then return 0 end

    local by_source = opts and opts.by_source
    local n = 0

    local function sweep(ns, key_scope)
        for _, key in ipairs(DAEMON.cache.keys(ns, key_scope)) do
            local rec = DAEMON.cache.get(ns, key_scope, key)
            local hit = by_source
                and (type(rec) == "table" and rec.source == id_or_source)
                or (not by_source and key == id_or_source)
            if hit and DAEMON.cache.delete(ns, key_scope, key) then n = n + 1 end
        end
    end

    if entity.char_id ~= nil then sweep(GRANTS, entity.char_id) end
    sweep(GRANTS_FAST, scope)
    return n
end

--- *The abilities from this source are now exactly these.*
---
--- The same contract, word for word, as `effect_d.set_source_effects`, and the
--- same reason: it is idempotent, so it is safe to call on every login and every
--- slot change without having to work out what it did last time.
--- @param specs table  array of `"id"` or `{ id = "...", rank = n }`
--- @return number added, number removed
function M.set_source_abilities(entity, source, specs)
    if not (DAEMON and DAEMON.cache) then return 0, 0 end
    local scope = scope_of(entity)
    if not scope or type(source) ~= "string" then return 0, 0 end

    local want = {}
    for _, s in ipairs(specs or {}) do
        if type(s) == "string" then
            want[s] = { rank = 1 }
        elseif type(s) == "table" and type(s.id) == "string" then
            want[s.id] = { rank = tonumber(s.rank) or 1 }
        end
    end

    local added, removed = 0, 0

    for _, key in ipairs(DAEMON.cache.keys(GRANTS_FAST, scope)) do
        local rec = DAEMON.cache.get(GRANTS_FAST, scope, key)
        if type(rec) == "table" and rec.source == source then
            if want[key] == nil then
                if DAEMON.cache.delete(GRANTS_FAST, scope, key) then removed = removed + 1 end
            elseif (tonumber(rec.rank) or 1) == want[key].rank then
                want[key] = nil        -- already exactly right; leave it alone
            end
        end
    end

    for id, w in pairs(want) do
        if M._abilities[id] and M._abilities[id].grantable then
            if DAEMON.cache.set(GRANTS_FAST, scope, id,
                { rank = w.rank, source = source }) then
                added = added + 1
            end
        end
    end

    return added, removed
end

-- ─── Requirements ────────────────────────────────────────────────────────────

run_checks = function(list, user, target, ctx)
    local checks = Abilities.checks()
    for _, req in ipairs(list or {}) do
        local fn = checks[req.kind]
        if not fn then
            log_warn("ABILITY_D: no requirement kind '" .. tostring(req.kind) .. "'")
        else
            local ok, allowed, why = pcall(fn, user, target, req, ctx)
            if not ok then
                log_error("ABILITY_D: requirement '" .. tostring(req.kind)
                    .. "' raised: " .. tostring(allowed))
                return false, "Something prevents you."
            end
            if not allowed then
                -- `why` on the spec wins, so a designer can say what they mean
                -- without writing a predicate.
                return false, req.why or why or "You cannot do that."
            end
        end
    end
    return true
end

-- ─── Targeting ───────────────────────────────────────────────────────────────

local function resolve_target(user, spec, wanted)
    if spec.target == "none" then return nil, nil end
    if spec.target == "self" then return user, nil end

    -- An entity handed in directly, which is how a game-layer caller and the
    -- completion path both avoid re-resolving a name.
    if type(wanted) == "table" then return wanted, nil end

    if (wanted == nil or wanted == "") and spec.default_target == "combat"
        and DAEMON and DAEMON.combat then
        local current = DAEMON.combat.target_of(user)
        if current then return current, nil end
    end

    if wanted == nil or wanted == "" then
        return nil, "At what?"
    end

    local room_id = DAEMON.world and user.char_id
        and DAEMON.world.get_character_room(user.char_id) or user.room_id
    local ok, found = pcall(DAEMON.mobs.find_in_room, room_id or "", wanted)
    local target = ok and found or nil
    if not target then return nil, "There is no " .. tostring(wanted) .. " here." end
    return target, nil
end

-- ─── Outcomes ────────────────────────────────────────────────────────────────

local function side(ctx, which)
    if which == "self" then return ctx.user end
    return ctx.target or ctx.user
end

--- Everything an ability does, in one fixed order.
---
--- Fixed, so two abilities with the same fields always behave the same way. The
--- order is: roll, announce, dispel, damage, heal, apply, report, `run`, engage.
local function resolve_outcomes(spec, ctx)
    local rng = M._roll

    announce(spec, ctx)

    for _, r in ipairs(spec.remove) do
        local who = side(ctx, r.to or "self")
        if who and DAEMON and DAEMON.effect then
            pcall(DAEMON.effect.remove, who, r.effect, { count = r.count })
        end
    end

    -- An attack can miss, be parried, land somewhere and be blunted by whatever
    -- covers that place. `damage` below always lands. Both together is refused
    -- at define time, so exactly one of these branches runs.
    if spec.attack then
        local who = side(ctx, spec.attack.to or "target")
        local amount = Abilities.roll(spec.attack.damage, ctx, rng)
        if who then
            if DAEMON and DAEMON.combat and DAEMON.combat.resolve_attack then
                local res = DAEMON.combat.resolve_attack(ctx.user, who, {
                    amount              = amount,
                    damage_type         = spec.attack.damage and spec.attack.damage.type,
                    accuracy_multiplier = spec.attack.accuracy,
                    defense_multipliers = spec.attack.defenses,
                    location            = spec.attack.location,
                    source              = "ability:" .. spec.id,
                })
                ctx.amount = amount
                ctx.dealt  = res.dealt
                ctx.hit    = res.hit
                ctx.location    = res.hit_part
                ctx.degree      = res.degree
                ctx.defended_by = res.channel
                if not res.hit then
                    tell(ctx.user, Abilities.render(spec.messages.miss, ctx, ctx.user))
                end
            elseif type(who.take_damage) == "function" then
                -- Degrade rather than no-op. An ability that silently did
                -- nothing because a daemon was missing is the worst failure
                -- mode available.
                local _, dealt = who:take_damage(math.max(0, math.floor(amount + 0.5)), {
                    damage_type = spec.attack.damage and spec.attack.damage.type or "physical",
                    attacker    = ctx.user,
                })
                ctx.amount, ctx.dealt, ctx.hit = amount, dealt, true
            end
        end
    end

    if spec.damage then
        local who = side(ctx, spec.damage.to or "target")
        local amount = math.max(0, math.floor(Abilities.roll(spec.damage, ctx, rng) + 0.5))
        if who and type(who.take_damage) == "function" then
            -- Through `take_damage`, so armour, resists and the whole
            -- `damage_taken` pipeline meet an ability exactly as they meet a
            -- sword. An ability that bypassed it would be the one thing in the
            -- game nothing could ever be designed against.
            local _, dealt = who:take_damage(amount, {
                damage_type = spec.damage.type or "physical",
                attacker    = ctx.user,
            })
            ctx.amount, ctx.dealt = amount, dealt
        end
    end

    if spec.heal then
        local who = side(ctx, spec.heal.to or "self")
        local amount = math.max(0, math.floor(Abilities.roll(spec.heal, ctx, rng) + 0.5))
        if who and type(who.heal) == "function" then
            local _, healed = who:heal(amount, { source = "ability:" .. spec.id })
            ctx.amount, ctx.healed = amount, healed
        end
    end

    for _, a in ipairs(spec.apply) do
        local who = side(ctx, a.to or "self")
        if who and DAEMON and DAEMON.effect then
            local chance = tonumber(a.chance)
            if chance == nil or (M._roll(100) <= chance * 100) then
                local inst, why = DAEMON.effect.apply(who, a.effect, {
                    source   = "ability:" .. spec.id,
                    duration = a.duration and Abilities.roll(a.duration, ctx, rng) or nil,
                    potency  = a.potency and Abilities.roll(a.potency, ctx, rng) or nil,
                    stacks   = a.stacks,
                    -- An **id**, never the entity. An effect instance is plain
                    -- data in a write-behind namespace, so a live table here
                    -- fails `lua_to_json` and the whole apply is refused — with
                    -- no reason, because the refusal comes from the cache write
                    -- rather than from anything about the effect.
                    caster   = ctx.user and (ctx.user.char_id or ctx.user.id),
                })
                if not inst then
                    ctx.why = why or "something is in the way"
                    tell(ctx.user, Abilities.render(spec.messages.fail, ctx, ctx.user))
                end
            end
        end
    end

    -- After the outcome, because it is what doing the thing did to you. A
    -- footing cost belongs here and not in `cost`: `cost` is what you must have
    -- to begin, and it cannot be positive.
    for _, a in ipairs(spec.adjust) do
        local who = side(ctx, a.to)
        local amount = Abilities.roll(a.amount, ctx, rng)
        if who and amount ~= 0 and DAEMON and DAEMON.trait then
            -- `adjust`, so it clamps to the gauge's bounds and settles
            -- regeneration first — the delta lands on the value as it is now.
            DAEMON.trait.adjust(who, a.trait, amount)
        end
    end

    tell(ctx.user, Abilities.render(spec.messages.result, ctx, ctx.user))

    if type(spec.run) == "function" then
        local ok, err = pcall(spec.run, ctx)
        if not ok then
            log_error("ABILITY_D: '" .. spec.id .. "' run raised: " .. tostring(err))
            return false, "The working comes apart in your hands."
        end
    end

    if spec.engage and ctx.target and ctx.target ~= ctx.user
        and DAEMON and DAEMON.combat then
        if ctx.target.is_alive and ctx.target:is_alive() then
            pcall(DAEMON.combat.engage, ctx.target, ctx.user)
        end
    end

    return true
end

--- Overridable, so a test can pin the dice. `combat_d._roll`'s twin.
function M._roll(n) return math.random(n) end

-- ─── Spending ────────────────────────────────────────────────────────────────

local function affordable(user, spec, ctx)
    for _, c in ipairs(spec.cost) do
        local need = Abilities.roll(c.amount, ctx, M._roll)
        if (user.trait and user:trait(c.trait) or 0) < need then
            local def = DAEMON.trait and DAEMON.trait.get_def(c.trait)
            local label = (def and def.label) or c.trait
            return false, "You have not the " .. tostring(label):lower() .. " for that."
        end
    end
    return true
end

--- Spend, and report what went so an interrupt handler need not recompute it.
local function spend(user, spec, ctx)
    local spent = {}
    for _, c in ipairs(spec.cost) do
        local need = Abilities.roll(c.amount, ctx, M._roll)
        if need > 0 and DAEMON and DAEMON.trait then
            -- `adjust`, never a modifier: it settles regeneration first, so the
            -- cost comes off the value as it is *now* rather than as it was when
            -- the bar was last read.
            DAEMON.trait.adjust(user, c.trait, -need)
            spent[c.trait] = (spent[c.trait] or 0) + need
        end
    end
    return spent
end

local function mark_cooldown(user, spec)
    if not (DAEMON and DAEMON.cooldown) then return end
    local seconds = spec.cooldown.seconds or 0
    if seconds > 0 then
        DAEMON.cooldown.mark(user, Abilities.cooldown_key(spec), seconds,
            spec.cooldown.durable ~= nil and { durable = spec.cooldown.durable } or nil)
    end
end

--- Owe this ability's track some recovery, after the outcome.
---
--- Recovery rather than a price: `cost` is what you must have to begin, and this
--- is what having acted leaves you owing. Which is why it is marked here and not
--- at step 11.
local function mark_roundtime(user, spec, ctx)
    if not (spec.track and spec.roundtime and DAEMON and DAEMON.queue) then return end
    DAEMON.queue.mark(user, spec.track, spec.roundtime, ctx)
end

local function mark_gcd(user, spec)
    local gcd = conf("game.ability_gcd_seconds", 0)
    if gcd > 0 and spec.gcd and DAEMON and DAEMON.cooldown then
        DAEMON.cooldown.mark(user, GCD_KEY, gcd, { durable = false })
    end
end

-- ─── Casts in flight ─────────────────────────────────────────────────────────

function M.casting(entity)
    if not (DAEMON and DAEMON.cache) then return nil end
    local scope = scope_of(entity)
    if not scope then return nil end
    local rec = DAEMON.cache.get(CASTS, scope, "cast")
    return type(rec) == "table" and rec or nil
end

local function clear_cast(entity, record)
    local scope = scope_of(entity)
    if not scope then return end
    DAEMON.cache.delete(CASTS, scope, "cast")
    if entity.char_id and DAEMON.ticker then
        DAEMON.ticker.remove("player." .. tostring(entity.char_id) .. ".ability.cast")
    end
    if record and record.effect_key and DAEMON.effect then
        pcall(DAEMON.effect.remove, entity, record.channel_def, { silent = true })
    end
end

--- End a cast in flight. The one door for `flee`, death and game code.
--- @return boolean  whether there was one
function M.cancel(entity, reason)
    local record = M.casting(entity)
    if not record then return false end
    local spec = M._abilities[record.ability]

    clear_cast(entity, record)

    if spec then
        local ctx = {
            user = entity, actor = entity, target = record.target, ability = spec,
            rank = record.rank, power = record.power,
            spent = record.spent, reason = reason or "interrupted",
            name = entity.name,
        }
        if type(spec.on_interrupt) == "function" then
            local ok, err = pcall(spec.on_interrupt, ctx)
            if not ok then
                log_error("ABILITY_D: '" .. spec.id .. "' on_interrupt raised: " .. tostring(err))
            end
        end
        tell(entity, "{yellow}Your " .. tostring(spec.name):lower()
            .. " comes apart. (" .. tostring(reason or "interrupted") .. "){/}")
    end

    if DAEMON and DAEMON.event then
        pcall(DAEMON.event.emit, "ability.interrupted", {
            char_id = entity.char_id, id = record.ability, reason = reason,
        })
    end
    return true
end

--- Finish a cast that has run its time.
local function complete(entity, record)
    local spec = M._abilities[record.ability]
    if not spec then
        clear_cast(entity, record)
        return
    end

    local ctx = {
        user = entity, actor = entity, target = record.target, ability = spec,
        rank = record.rank, power = record.power, spent = record.spent,
        name = entity.name,
        target_name = record.target and (record.target.short or record.target.name),
    }
    ctx.target = record.target

    -- Re-checked at completion. Three seconds is enough for the target to die,
    -- to walk out, or for the caster to be silenced — and a cast that resolved
    -- anyway would be the one path in the game where a requirement is advisory.
    local ok, why = run_checks(spec.requires, entity, record.target, ctx)
    if ok and record.target and record.target.is_alive and not record.target:is_alive()
        and not spec.allow_dead then
        ok, why = false, "it is already dead"
    end
    if not ok then
        M.cancel(entity, why or "conditions changed")
        return
    end

    clear_cast(entity, record)
    mark_cooldown(entity, spec)
    resolve_outcomes(spec, ctx)
    -- Owed for having acted. An interrupted cast owes none — same shape, and the
    -- same justification, as the cooldown it also does not mark.
    mark_roundtime(entity, spec, ctx)

    if type(spec.on_complete) == "function" then pcall(spec.on_complete, ctx) end

    if DAEMON and DAEMON.event then
        pcall(DAEMON.event.emit, "ability.used", {
            char_id = entity.char_id, id = spec.id,
            target = record.target and record.target.id,
        })
    end
end

M._complete = complete

--- The effect definition that *is* a channel.
---
--- Generated per ability, lazily and idempotently. Everything a channel needs,
--- `effect_d` already has: a timed thing attached to an entity, something that
--- happens every N seconds, an end that knows *why* it ended, a sweep for a
--- player who is typing nothing, removal on death and on logout, and visibility
--- in the `effects` command. Building a second one of those would be building
--- `effect_d` again, worse.
local function channel_def(spec)
    local id = "channel_" .. spec.id
    if _channels[id] then return id end
    if not (DAEMON and DAEMON.effect) then return nil end

    local tick = math.max(1, spec.channel.tick or 1)

    DAEMON.effect.define({
        id       = id,
        label    = spec.name,
        desc     = "Channelling " .. tostring(spec.name) .. ".",
        duration = spec.channel.duration,
        tick     = tick,
        stack    = "replace",
        persist  = false,
        hooks = {
            heartbeat = { phase = "post", fn = function(ev, ctx)
                local entity = ctx and ctx.entity
                local record = entity and M.casting(entity)
                if not record or record.ability ~= spec.id then return end

                for _ = 1, (ev.ticks or 1) do
                    record.ticks_done = (record.ticks_done or 0) + 1
                    local c = {
                        user = entity, actor = entity, target = record.target, ability = spec,
                        rank = record.rank, power = record.power,
                        tick = record.ticks_done, name = entity.name,
                    }
                    resolve_outcomes(spec, c)
                    if type(spec.on_tick) == "function" then pcall(spec.on_tick, c) end
                end
            end },
        },
        on_expire = function(entity, _, reason)
            -- `"timeout"` is completion; anything else is an interrupt. That
            -- distinction is exactly what a channel needs and `effect_d` already
            -- draws it.
            local record = M.casting(entity)
            if not record or record.ability ~= spec.id then return end
            if reason == "timeout" then
                record.effect_key = nil
                complete(entity, record)
            else
                M.cancel(entity, tostring(reason or "interrupted"))
            end
        end,
    })

    _channels[id] = true
    return id
end

-- ─── use ─────────────────────────────────────────────────────────────────────

--- Do the thing.
--- @param user table
--- @param id string
--- @param opts table|nil  { target = "name" | <entity> }
--- @return boolean ok, string|nil why
function M.use(user, id, opts)
    if type(user) ~= "table" then return false, "Nobody can do that." end
    opts = opts or {}

    local spec = M._abilities[id]
    if not spec then return false, "You do not know any such thing." end
    if not M.knows(user, id) then return false, "You do not know any such thing." end

    local rank = M.rank(user, id)
    if rank < spec.min_rank then return false, "That is beyond you for now." end

    if M.casting(user) then return false, "You are already busy." end

    local ctx = {
        user = user, actor = user, ability = spec, rank = rank, name = user.name,
        power = 1 + (user.trait and user:trait("spell_power") or 0),
    }

    local ok, why = run_checks(spec.pre_requires, user, nil, ctx)
    if not ok then return false, why end

    if DAEMON and DAEMON.cooldown then
        local key = Abilities.cooldown_key(spec)
        if spec.cooldown.seconds > 0 and not DAEMON.cooldown.ready(user, key) then
            return false, "Not yet. (" .. math.ceil(DAEMON.cooldown.remaining(user, key)) .. "s)"
        end
        if spec.gcd and not DAEMON.cooldown.ready(user, GCD_KEY) then
            return false, "Not yet."
        end
    end

    -- Before anything is spent.
    local target, terr = resolve_target(user, spec, opts.target)
    if terr then return false, terr end
    ctx.target = target
    ctx.target_name = target and (target.short or target.name)

    ok, why = run_checks(spec.target_requires, user, target, ctx)
    if not ok then return false, why end

    ok, why = affordable(user, spec, ctx)
    if not ok then return false, why end

    -- ── nothing has been spent above this line ──────────────────────────────

    -- **Only roundtime enqueues.** Cooldown, GCD, cost, requirements, an unknown
    -- ability and a mistyped target all still refuse, which keeps two different
    -- pieces of information distinct: a cooldown says "not this, for a while";
    -- roundtime says "not yet, but soon and certainly".
    --
    -- Below target resolution on purpose — a typo refuses immediately rather
    -- than queueing something doomed.
    if spec.track and not (opts.from_queue) and DAEMON and DAEMON.queue then
        if DAEMON.queue.in_roundtime(user, spec.track) then
            local queued, why = DAEMON.queue.enqueue(user, spec.track, {
                kind = "ability", id = spec.id, target = target,
            })
            if not queued then return false, why end
            local left = math.ceil(DAEMON.queue.roundtime(user, spec.track))
            tell(user, "You will " .. tostring(spec.name):lower()
                .. " next. (" .. left .. "s)")
            return true, nil, "queued"
        end
    end

    if spec.cast_time > 0 or spec.channel then
        local spent = spend(user, spec, ctx)
        mark_gcd(user, spec)

        local duration = spec.channel and spec.channel.duration or spec.cast_time
        local record = {
            ability = spec.id,
            kind    = spec.channel and "channel" or "cast",
            user    = user, target = target,
            started = os_time(), ends_at = os_time() + duration,
            ticks_done = 0, rank = rank, power = ctx.power, spent = spent,
        }

        DAEMON.cache.set(CASTS, scope_of(user), "cast", record)

        if type(spec.on_begin) == "function" then pcall(spec.on_begin, ctx) end
        tell(user, Abilities.render(spec.messages.begin, ctx, user))

        if spec.channel then
            local def = channel_def(spec)
            if def then
                record.channel_def = def
                record.effect_key = def
                DAEMON.effect.apply(user, def, { source = "ability:" .. spec.id })
            end
        elseif user.char_id and DAEMON.ticker then
            -- `player.<char_id>.…`, so `character_d.unload`'s existing
            -- `remove_by_prefix` cleans it up with no new code.
            DAEMON.ticker.after(spec.cast_time,
                "player." .. tostring(user.char_id) .. ".ability.cast", function()
                    local still = M.casting(user)
                    if still and still.ability == spec.id then complete(user, still) end
                end)
        end
        return true, nil, "casting"
    end

    local spent = spend(user, spec, ctx)
    ctx.spent = spent
    mark_cooldown(user, spec)
    mark_gcd(user, spec)

    local done, err = resolve_outcomes(spec, ctx)
    if not done then return false, err end

    if type(spec.on_complete) == "function" then pcall(spec.on_complete, ctx) end

    mark_roundtime(user, spec, ctx)

    if DAEMON and DAEMON.event then
        pcall(DAEMON.event.emit, "ability.used", {
            char_id = user.char_id, id = spec.id, target = target and target.id,
        })
    end
    return true, nil, "done"
end

-- ─── Interrupts ──────────────────────────────────────────────────────────────

--- A working in flight is broken by being hit.
---
--- Called directly from `Mobile:take_damage` rather than through an event or an
--- effect hook. Not an event, because `event.emit` on every hit of every fight
--- is a table and a dispatch on the hottest loop in the game. Not the effect
--- pipeline, because a `post`-phase handler only runs for an entity that *has*
--- an effect on it — and a caster part-way through a plain `cast_time` has none,
--- so the interrupt would silently not fire for exactly the case it is for.
function M.on_damaged(entity, amount, _opts)
    local record = M.casting(entity)
    if not record then return end
    local spec = M._abilities[record.ability]
    if not spec or not spec.interrupt.on_damage then return end
    if (tonumber(amount) or 0) <= spec.interrupt.threshold then return end
    M.cancel(entity, "you were struck")
end

--- Walking out ends it, and **never refuses the move**: a channel that pinned
--- you in place until it finished would be a trap, not a cost.
function M.on_moved(char_id)
    if not (DAEMON and DAEMON.character) then return end
    local entity = DAEMON.character.get(char_id)
    if not entity then return end
    local record = M.casting(entity)
    if not record then return end
    local spec = M._abilities[record.ability]
    if not spec or not spec.interrupt.on_move then return end
    M.cancel(entity, "you moved")
end

-- ─── Housekeeping ────────────────────────────────────────────────────────────

--- Complete or cancel anything past its deadline.
---
--- The backstop, and it has the same justification as `effect_d.sweep`:
--- `ticker_d` holds its callbacks in a module-level table that does not survive
--- a hot reload of `ticker_d` itself, so a cast in flight across one would
--- otherwise hang forever.
function M.sweep()
    if not (DAEMON and DAEMON.cache) then return 0 end
    local now, n = os_time(), 0
    for _, scope in ipairs(DAEMON.cache.scopes(CASTS)) do
        local record = DAEMON.cache.get(CASTS, scope, "cast")
        if type(record) == "table" and record.ends_at and now >= record.ends_at then
            local entity = record.user
            if type(entity) == "table" then
                if record.kind == "channel" then
                    -- The effect owns a channel's ending; if it is already gone
                    -- the record is stale and the cast is simply over.
                    if not (DAEMON.effect and DAEMON.effect.has(entity, record.channel_def)) then
                        complete(entity, record)
                        n = n + 1
                    end
                else
                    complete(entity, record)
                    n = n + 1
                end
            else
                DAEMON.cache.delete(CASTS, scope, "cast")
            end
        end
    end
    return n
end

--- Everything this character is holding, on disconnect.
function M.cleanup(char_id)
    if not (DAEMON and DAEMON.cache) then return end
    local scope = "char:" .. tostring(char_id)
    DAEMON.cache.delete(CASTS, scope, "cast")
    DAEMON.cache.drop(GRANTS_FAST, scope)
end

--- The same for a creature, called from `mob_d.despawn`.
function M.detach(entity)
    local scope = scope_of(entity)
    if not scope or not (DAEMON and DAEMON.cache) then return end
    DAEMON.cache.delete(CASTS, scope, "cast")
    DAEMON.cache.drop(GRANTS_FAST, scope)
end

do
    local every = conf("game.ability_sweep_seconds", 2)
    if every > 0 and DAEMON and DAEMON.ticker then
        DAEMON.ticker.every(every, "ability.sweep", function()
            local ok, err = pcall(M.sweep)
            if not ok then log_error("ABILITY_D: sweep raised: " .. tostring(err)) end
        end)
    end
end

log("info", "ability_d daemon loaded")

return M
