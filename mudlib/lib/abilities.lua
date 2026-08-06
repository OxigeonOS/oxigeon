-- mudlib/lib/abilities.lua — The arithmetic and the vocabulary of an ability.
--
-- The `lib/effects.lua` and `lib/traits.lua` counterpart: everything an ability
-- needs that is a pure function of its arguments. No `DAEMON`, no clock, no
-- world — so the rules below can be driven directly by a test rather than
-- inferred from a fight.
--
-- ─── Which half is data and which is code ────────────────────────────────────
--
-- The same split `effect_d` already draws, and it is what makes the system
-- low-code rather than no-code:
--
--   definitions   game/abilities/*.lua, ordinary code. May hold functions.
--                 Never serialized, never written anywhere.
--   grants, casts DAEMON.cache. Ids, numbers and timestamps only.
--
-- A designer writes the first. A coder writes the effects and the occasional
-- `run`. Nothing a designer writes has to be Lua.
--
-- ─── Three shapes for a number, and no formula strings ───────────────────────
--
--     6                                          a number
--     { min = 4, max = 9 }                       a range
--     { min = 8, max = 8, scale = { trait = "spell_power", per = 2 } }
--     function(ctx) return 2 + math.floor(ctx.power / 3) end
--
-- Not a formula string. Parsing `"6 + spell_power * 2"` needs either a
-- hand-rolled expression parser — **a second string-to-value converter, which
-- CLAUDE.md forbids by name** — or `load()` on author text, which is a sandbox
-- hole. The precedent is already functions: `trait.formula`, `effect.condition`,
-- the room lfun pattern. And `{ trait, per }` covers `base + stat * n`, which is
-- what a damage formula almost always is, and is the half that survives being
-- written by somebody who does not write Lua.
--
-- Exposes:
--   Abilities.roll(amount, ctx, rng)     -> number
--   Abilities.normalise(spec)            -> spec, err   (mutates and returns)
--   Abilities.checks()                   -> the seeded predicate table
--   Abilities.render(template, ctx)      -> string
--   Abilities.cooldown_key(spec)         -> string
--   Abilities.COST_KINDS / Abilities.TARGETS
--
-- See docs/src/lua-api/abilities.md.

local M = {}

--- What `target` may say. Resolved by `ability_d`, so every ability refuses the
--- same way rather than each inventing a message.
M.TARGETS = {
    none = true, self = true, creature = true, ally = true, any = true, item = true,
}

-- ─── Amounts ─────────────────────────────────────────────────────────────────

--- The value of one trait for the scaling walk.
---
--- `rank` is a pseudo-trait resolving to the ability's own rank, so a designer
--- scales with rank without knowing which trait backs it — and it still works
--- for an ability that was granted and has no trait at all.
local function trait_value(ctx, id)
    if id == "rank"  then return tonumber(ctx and ctx.rank)  or 0 end
    if id == "power" then return tonumber(ctx and ctx.power) or 0 end
    local user = ctx and ctx.user
    if type(user) == "table" and type(user.trait) == "function" then
        local ok, v = pcall(user.trait, user, id)
        if ok and type(v) == "number" then return v end
    end
    return 0
end

--- Resolve one authored amount to a number.
---
--- `per` is flat per point of the trait; `pct` is percent of the rolled base per
--- point. Both are applied against the base *before* rounding, so two `pct`
--- entries compose additively rather than multiplying by accident.
--- @param amount number|table|function
--- @param ctx table|nil    { user, rank, power, ... }
--- @param rng function|nil f(n) -> 1..n, so a test can pin the dice
--- @return number
function M.roll(amount, ctx, rng)
    if type(amount) == "function" then
        local ok, v = pcall(amount, ctx)
        return (ok and tonumber(v)) or 0
    end
    if type(amount) == "number" then return amount end
    if type(amount) ~= "table" then return 0 end

    local lo = tonumber(amount.min) or 0
    local hi = tonumber(amount.max)
    if hi == nil then hi = lo end
    if hi < lo then lo, hi = hi, lo end

    local base = lo
    if hi > lo then
        local roll = rng or function(n) return math.random(n) end
        -- `+ 1` because both ends are inclusive: `{ min = 4, max = 9 }` is six
        -- outcomes, not five.
        base = lo + (roll(math.floor(hi - lo) + 1) - 1)
    end

    local scale = amount.scale
    if scale ~= nil then
        -- A bare `{ trait = ..., per = ... }` is normalised into an array here
        -- rather than at define time, so a hand-built spec in a test behaves the
        -- same as an authored one.
        if scale.trait ~= nil then scale = { scale } end
        local added = 0
        for _, s in ipairs(scale) do
            local v = trait_value(ctx, s.trait)
            if s.per then added = added + v * s.per end
            if s.pct then added = added + base * (s.pct / 100) * v end
        end
        base = base + added
    end

    return base
end

-- ─── Requirement predicates ──────────────────────────────────────────────────

--- A predicate is `f(user, target, spec) -> ok, why`.
---
--- A registry rather than a list, seeded here beside the code that implements
--- them and extended by the game layer with `DAEMON.ability.define_check`. The
--- house rule: never a central list, because that is the thing that rots.
---
--- `why` on the spec overrides the default refusal, so a designer can say
--- "You need a sword in your hand for that" without writing a predicate.
local CHECKS = {}

local function ent_trait(entity, id)
    if type(entity) ~= "table" or type(entity.trait) ~= "function" then return 0 end
    local ok, v = pcall(entity.trait, entity, id)
    return (ok and type(v) == "number") and v or 0
end

CHECKS.trait = function(user, _, spec)
    local v = ent_trait(user, spec.id)
    if spec.min and v < spec.min then return false, "You are not practised enough at that." end
    if spec.max and v > spec.max then return false, "You are past that." end
    return true
end

CHECKS.rank = function(_, _, spec, ctx)
    local rank = ctx and ctx.rank or 0
    if spec.min and rank < spec.min then
        return false, "You have not learned enough of that."
    end
    return true
end

CHECKS.resource = function(user, _, spec)
    if ent_trait(user, spec.trait) < (spec.min or 1) then
        return false, "You have not the " .. tostring(spec.label or spec.trait) .. " for that."
    end
    return true
end

CHECKS.effect = function(user, _, spec)
    if not (DAEMON and DAEMON.effect and DAEMON.effect.has(user, spec.id)) then
        return false, "You are not in the right state for that."
    end
    return true
end

CHECKS.effect_absent = function(user, _, spec)
    if DAEMON and DAEMON.effect and DAEMON.effect.has(user, spec.id) then
        return false, "Something prevents you."
    end
    return true
end

CHECKS.equipped = function(user, _, spec)
    local worn = type(user) == "table" and user.equipment
    local entry = type(worn) == "table" and worn[spec.slot]
    if not entry then return false, "You are not holding the right thing." end

    if spec.tag or spec.template then
        local resolved = entry
        if DAEMON and DAEMON.items and DAEMON.items.resolve then
            local ok, r = pcall(DAEMON.items.resolve, entry)
            if ok and r then resolved = r end
        end
        if spec.template and resolved.id ~= spec.template
            and resolved.template ~= spec.template then
            return false, "You are not holding the right thing."
        end
        if spec.tag then
            local hit = false
            for _, t in ipairs(resolved.tags or {}) do if t == spec.tag then hit = true end end
            if not hit then return false, "You are not holding the right thing." end
        end
    end
    return true
end

-- Target-needing predicates. Partitioned by name in `normalise`, because the
-- `use` flow has to run the others *before* a target is resolved — a mistyped
-- name must not cost mana, and a refusal you could have given without looking
-- should be given without looking.
CHECKS.target_alive = function(_, target, _)
    if type(target) ~= "table" then return false, "At what?" end
    if target.is_alive and not target:is_alive() then return false, "It is already dead." end
    return true
end

CHECKS.target_dead = function(_, target, _)
    if type(target) == "table" and target.is_alive and target:is_alive() then
        return false, "It still lives."
    end
    return true
end

CHECKS.target_hostile = function(user, target, _)
    if not (DAEMON and DAEMON.combat) then return true end
    if DAEMON.combat.target_of(user) ~= target then
        return false, "You are not fighting that."
    end
    return true
end

CHECKS.target_not_self = function(user, target, _)
    if user == target then return false, "Not on yourself." end
    return true
end

CHECKS.in_combat = function(user, _, _)
    if DAEMON and DAEMON.combat and not DAEMON.combat.is_fighting(user) then
        return false, "Only in a fight."
    end
    return true
end

CHECKS.out_of_combat = function(user, _, _)
    if DAEMON and DAEMON.combat and DAEMON.combat.is_fighting(user) then
        return false, "Not while you are fighting."
    end
    return true
end

--- The bridge to `lib/checks.lua`, and the reason this does not become a second
--- predicate library. Those are `f(player) -> ok, why`; these are
--- `f(user, target, spec) -> ok, why`. `all`, `any` and `not_` come from there
--- unchanged.
CHECKS.check = function(user, _, spec)
    if type(spec.fn) ~= "function" then return true end
    local ok, allowed, why = pcall(spec.fn, user)
    if not ok then return false, "Something prevents you." end
    return allowed and true or false, why
end

--- Which predicates need a resolved target. `use` runs everything else first.
local NEEDS_TARGET = {
    target_alive = true, target_dead = true, target_hostile = true,
    target_not_self = true,
}

--- The live predicate table. `ability_d.define_check` writes into it.
function M.checks() return CHECKS end

function M.needs_target(kind) return NEEDS_TARGET[kind] == true end

--- Mark a game-layer predicate as needing a target.
function M.set_needs_target(kind, needs)
    NEEDS_TARGET[kind] = needs and true or nil
end

-- ─── Normalising a spec ──────────────────────────────────────────────────────

local function as_list(v)
    if v == nil then return {} end
    if type(v) ~= "table" then return {} end
    if v[1] ~= nil then return v end
    return { v }
end

--- Turn everything an author may write into the one shape the daemon reads.
---
--- Sugar is desugared here and nowhere else, the way `effect_d` desugars
--- `modifiers` into hook handlers at define time. The daemon then has exactly
--- one shape to reason about, and the sugar cannot drift from what it means.
--- @param spec table  mutated
--- @return table|nil spec, string|nil err
function M.normalise(spec)
    if type(spec) ~= "table" then return nil, "an ability spec must be a table" end
    if type(spec.id) ~= "string" or spec.id == "" then
        return nil, "an ability needs a string id"
    end

    spec.name     = spec.name or spec.id
    spec.category = spec.category or "ability"
    spec.summary  = spec.summary or ""
    spec.target   = spec.target or "none"
    spec.range    = spec.range or "room"
    spec.min_rank = tonumber(spec.min_rank) or 1
    spec.open     = spec.open and true or false
    if spec.grantable == nil then spec.grantable = true end

    if not M.TARGETS[spec.target] then
        return nil, "'" .. tostring(spec.target) .. "' is not a target kind. One of: "
            .. "none self creature ally any item"
    end

    -- `cost = { mp = 8 }` and `cost = { { trait = "mp", amount = 8 } }` are the
    -- same thing said two ways. The map form cannot express a scaling amount,
    -- which is the only reason the long form exists.
    local costs = {}
    if type(spec.cost) == "table" then
        if spec.cost[1] ~= nil then
            for _, c in ipairs(spec.cost) do
                costs[#costs + 1] = { trait = c.trait, amount = c.amount or 0 }
            end
        else
            local names = {}
            for trait in pairs(spec.cost) do names[#names + 1] = trait end
            table.sort(names)   -- deterministic spend order
            for _, trait in ipairs(names) do
                costs[#costs + 1] = { trait = trait, amount = spec.cost[trait] }
            end
        end
    end
    spec.cost = costs

    -- `cooldown = 4` and the table form.
    local cd = spec.cooldown
    if type(cd) == "number" then cd = { seconds = cd } end
    if type(cd) ~= "table" then cd = {} end
    cd.seconds = tonumber(cd.seconds) or 0
    spec.cooldown = cd
    if spec.gcd == nil then spec.gcd = true end

    -- `level = 3` is sugar for the requirement everybody writes first.
    local requires = {}
    if spec.level ~= nil then
        requires[#requires + 1] = { kind = "trait", id = "level", min = tonumber(spec.level) or 1 }
    end
    for _, r in ipairs(as_list(spec.requires)) do
        if type(r) ~= "table" or type(r.kind) ~= "string" then
            return nil, "each `requires` entry needs a string `kind`"
        end
        requires[#requires + 1] = r
    end
    spec.requires = requires

    -- Partitioned once, so `use` never asks the question per call.
    spec.pre_requires, spec.target_requires = {}, {}
    for _, r in ipairs(requires) do
        if M.needs_target(r.kind) then
            spec.target_requires[#spec.target_requires + 1] = r
        else
            spec.pre_requires[#spec.pre_requires + 1] = r
        end
    end

    spec.apply  = as_list(spec.apply)
    spec.remove = as_list(spec.remove)

    -- Damage implies a fight, because the alternative is an ability that hurts
    -- something and leaves it standing there.
    if spec.engage == nil then spec.engage = spec.damage ~= nil end

    if spec.channel ~= nil then
        if type(spec.channel) ~= "table" then return nil, "`channel` must be a table" end
        spec.channel.duration = tonumber(spec.channel.duration) or 0
        spec.channel.tick     = tonumber(spec.channel.tick) or 1
    end
    spec.cast_time = tonumber(spec.cast_time) or 0

    local interrupt = spec.interrupt
    if type(interrupt) ~= "table" then interrupt = {} end
    if interrupt.on_damage == nil then interrupt.on_damage = true end
    if interrupt.on_move   == nil then interrupt.on_move   = true end
    interrupt.threshold = tonumber(interrupt.threshold) or 0
    spec.interrupt = interrupt

    spec.messages = type(spec.messages) == "table" and spec.messages or {}

    return spec
end

--- The cooldown key. `<category>.<id>`, or `shared` verbatim when one is named.
---
--- `<category>.<id>` rather than `ability.<id>` so the migrated spells keep the
--- exact `spell.emberlance` key `cast` already prints. `shared` is used as-is,
--- so two abilities naming `school.fire` gate each other with no new mechanism.
function M.cooldown_key(spec)
    if type(spec.cooldown) == "table" and type(spec.cooldown.shared) == "string" then
        return spec.cooldown.shared
    end
    return tostring(spec.category or "ability") .. "." .. tostring(spec.id)
end

-- ─── Messages ────────────────────────────────────────────────────────────────

local TOKENS = { "name", "target", "amount", "dealt", "healed", "rank", "why" }

--- Substitute `$name`, `$target`, `$dealt` and the rest.
---
--- An unknown token is **left alone** rather than erased. A message reading
--- "You strike $victim" is a typo somebody can see and fix; one reading "You
--- strike " is a bug they will stare at.
--- @param template string|function
--- @param ctx table
--- @return string
function M.render(template, ctx)
    if type(template) == "function" then
        local ok, v = pcall(template, ctx)
        return ok and tostring(v or "") or ""
    end
    if type(template) ~= "string" then return "" end

    local out = template
    for _, token in ipairs(TOKENS) do
        local value = ctx and ctx[token]
        if value ~= nil then
            out = out:gsub("%$" .. token, (tostring(value):gsub("%%", "%%%%")))
        end
    end
    return out
end

return M
