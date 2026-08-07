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

    -- `{ rounds = 0.75 }` — three quarters of this character's round.
    --
    -- A real branch and **not** a desugar into `scale`, which was the obvious
    -- move and is wrong: `scale` is additive (`per` gives `base + trait*n`)
    -- where a round is multiplicative (`0.75 x round_length`). Solving for an
    -- equivalent `pct` gives a value that depends on the trait, so no fixed
    -- authored spec expresses it — a desugar written on that assumption would
    -- produce silently wrong numbers for every character whose round differed
    -- from whichever one it was eyeballed against.
    --
    -- The rounds component is itself rollable, so `{ rounds = { min = 1, max = 2 } }`
    -- is a variable number of rounds, and any `scale` on the outer table then
    -- adds flat seconds on top.
    if amount.rounds ~= nil then
        local rounds = M.roll(amount.rounds, ctx, rng)
        local length = tonumber(ctx and ctx.round_length) or 0
        local base = rounds * length
        local extra = nil
        if amount.scale ~= nil then
            extra = M.roll({ min = 0, max = 0, scale = amount.scale }, ctx, rng)
        end
        return base + (extra or 0)
    end

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

--- What this creature is made of. The payoff for layout features: an ability
--- that needs hands says so, and a snake cannot use it.
CHECKS.body_feature = function(user, _, spec)
    local ok, Body = pcall(require, 'lib.body')
    if not ok then return true end
    if Body.has_feature(user, spec.feature) then return true end
    return false, "You are not built for that."
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

    -- Which lane of intent this belongs to. Defaults to `"combat"`, which costs
    -- nothing today because a track only gates once something puts roundtime on
    -- it, and no shipped ability declares any.
    --
    -- `track = "none"` (or false) opts out for ever — a stance toggle, a shout,
    -- anything that should stay available while you are recovering. It has to
    -- exist because the gate belongs to the *track*: once a track is in
    -- roundtime every ability on it waits, whatever its own roundtime is.
    if spec.track == false or spec.track == "none" then
        spec.track = nil
    else
        spec.track = spec.track or "combat"
    end

    -- What using this costs the track in recovery. A number is seconds;
    -- `{ rounds = n }` is rounds. Marked *after* the action resolves, because it
    -- is recovery from having acted rather than a price for acting.
    if spec.roundtime ~= nil then
        local rt = spec.roundtime
        if type(rt) == "number" then rt = { min = rt, max = rt } end
        if type(rt) ~= "table" and type(rt) ~= "function" then
            return nil, "`roundtime` must be a number, a table or a function"
        end
        spec.roundtime = rt
    end

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

    -- `attack` routes damage through the resolution pipeline, so it can miss, be
    -- parried, land somewhere and be blunted by the armour covering that place.
    -- `damage` goes straight to `take_damage` and always lands.
    --
    -- **Both together is refused here**, the mirror of the gauge rule for cost.
    -- An ability that is ambiguous about whether it can miss is worse discovered
    -- at runtime than at define time.
    -- ─── A hostile ability defaults to what you are already fighting ────────
    --
    -- `perf emberlance` with no target used to answer "At what?" while a rat
    -- was biting you. `cleave` declared `default_target = "combat"` and nothing
    -- else did, so the mechanism existed and one ability used it.
    --
    -- Defaulted from the **declared outcome** rather than from a list of
    -- ability ids or from the word "spell": an ability that attacks or damages
    -- is aimed at an enemy, and an ability that heals is not. A list would need
    -- editing every time somebody wrote a new one, which is the thing that
    -- rots. An explicit `default_target` always wins, so an ability that means
    -- something else says so.
    if spec.default_target == nil and spec.target == "creature"
        and (spec.attack ~= nil or spec.damage ~= nil) then
        spec.default_target = "combat"
    end

    if spec.attack ~= nil then
        if type(spec.attack) ~= "table" then
            return nil, "`attack` must be a table"
        end
        if spec.damage ~= nil then
            return nil, "'" .. spec.id .. "' declares both `attack` and `damage`. "
                .. "`attack` can miss and `damage` cannot; pick one."
        end
        if spec.engage == nil then spec.engage = true end
    end

    -- `adjust = { balance = -5, stamina = 2 }` — a signed gauge delta, applied
    -- **after** the outcome rather than paid before it.
    --
    -- Not the same thing as `cost`, and the difference is the sign. A cost is
    -- what you must have to begin; an adjust is what doing it did to you, and it
    -- may be positive. A footing resource needs both: a heavy swing spends
    -- balance and a recovery step restores it, and there is no way to say the
    -- second with a cost.
    local adjust = {}
    if type(spec.adjust) == "table" then
        if spec.adjust[1] ~= nil then
            for _, a in ipairs(spec.adjust) do
                adjust[#adjust + 1] = { trait = a.trait, amount = a.amount or 0,
                                        to = a.to or "self" }
            end
        else
            local names = {}
            for trait in pairs(spec.adjust) do names[#names + 1] = trait end
            table.sort(names)   -- deterministic order, as `cost` is
            for _, trait in ipairs(names) do
                adjust[#adjust + 1] = { trait = trait, amount = spec.adjust[trait],
                                        to = "self" }
            end
        end
    end
    spec.adjust = adjust

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

    -- `line` is one authored sentence for everybody, rendered per reader. It is
    -- the whole point of the render layer, and it makes the three-string form
    -- unnecessary for anything that does not deliberately tell the actor
    -- something the room must not hear.
    --
    -- Both together is an author saying two things at once, so `line` wins and
    -- says so at define time rather than at the first cast.
    if spec.messages.line ~= nil then
        local also = {}
        for _, k in ipairs({ "self", "room", "target" }) do
            if spec.messages[k] ~= nil then also[#also + 1] = k end
        end
        if #also > 0 then
            return nil, "'" .. spec.id .. "' declares messages.line and also "
                .. table.concat(also, "/") .. ". `line` is one sentence for "
                .. "everybody; drop the others or drop `line`."
        end
    end

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

--- Substitute `$name`, `$target`, `$actor.their` and the rest.
---
--- Delegates to `lib/render.lua`. There is no second syntax and nothing is
--- deprecated: the sigil there was chosen as `$` precisely so it *subsumes*
--- what this used to do, which means every message already authored is already
--- valid input and simply gains meaning.
---
--- Three things this fixes by delegating:
---
--- * **`$target` printed a table address.** `ctx.target` is the entity, and
---   `target` was in a whitelist that ran `tostring` over whatever it found. So
---   `"You draw a line of fire at $target."` — shipped, in `game/abilities/` —
---   rendered `at table: 0x7f…`. Nothing asserted a successful cast's message
---   body, which is how it survived.
--- * **`$names` matched `$name` and left a stray `s`.** A token is now the
---   whole identifier.
--- * **The whitelist is gone.** It existed only because the old renderer looped
---   a fixed list; looking `ctx` up directly is strictly more useful and needs
---   no central list to rot.
---
--- An unknown token is still **left alone** rather than erased. A message
--- reading "You strike $victim" is a typo somebody can see and fix; one reading
--- "You strike " is a bug they will stare at.
--- @param template string|function
--- @param ctx table
--- @param viewer table|nil  who is reading; nil means nobody renders as "you"
--- @return string
function M.render(template, ctx, viewer)
    if type(template) == "function" then
        local ok, v = pcall(template, ctx)
        return ok and tostring(v or "") or ""
    end
    return require('lib.render').render(template, ctx, viewer)
end

return M
