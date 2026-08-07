-- game/daemons/spell_d.lua — Casting, now a projection of `ability_d`.
--
-- This used to be the arrangement itself: 177 lines wiring gauges, effects, the
-- damage pipeline, cooldowns and the trait graph into "casting", with every
-- spell inside it a hand-written function. All of that moved to
-- `mudlib/daemons/ability_d.lua`, because arranging those five things is not
-- what makes one game's magic different from another's — the *content* is, and
-- content should not have to carry a scheduler.
--
-- What survives here is the vocabulary. A spell is an ability with
-- `category = "spell"`, so `cast`, `spells` and this daemon keep meaning what
-- they meant, and the four legacy spec fields keep working.
--
-- ─── The cost model, which is why this file still has prose ──────────────────
--
-- Mana is a gauge, so it is spent with `adjust` and never with a modifier: a
-- spell that "modified" your mana would be a buff you have to unapply, which is
-- the mistake the whole effect design avoids. Power scales with `spell_power`,
-- a derived-of-derived trait, so a wisdom buff reaches a fireball through two
-- levels of the graph without anything here knowing.

local M = {}

--- Translate the four legacy fields, then hand it to the mudlib.
---
--- `cost` was a bare mana number, `level` a bare minimum, `cast` a function of
--- `(player, target, power)`. All three are still accepted, because a game with
--- its own spell list should not have to rewrite it to take this upgrade.
--- @param spec table  { id, name, cost, cooldown, target, level, cast }
--- @return boolean
function M.register(spec)
    if type(spec) ~= "table" or type(spec.id) ~= "string" then
        log("warn", "SPELL_D.register: a spell needs a string id")
        return false
    end
    if not (DAEMON and DAEMON.ability) then
        log("error", "SPELL_D.register: ability_d is not loaded")
        return false
    end

    local translated = {}
    for k, v in pairs(spec) do translated[k] = v end
    translated.category = spec.category or "spell"
    -- Every spell in this game is known to anyone who has the level for it.
    -- That is a real classless design and it should be one word.
    if translated.open == nil then translated.open = true end

    if type(spec.cost) == "number" then translated.cost = { mp = spec.cost } end

    if type(spec.cast) == "function" and translated.run == nil then
        local fn = spec.cast
        translated.cast = nil
        translated.run = function(ctx) return fn(ctx.user, ctx.target, ctx.power) end
    end

    return DAEMON.ability.define(translated)
end

function M.register_all(list)
    local n = 0
    for _, spec in ipairs(list or {}) do
        if M.register(spec) then n = n + 1 end
    end
    log("info", "SPELL_D: registered " .. n .. " spell(s)")
    return n
end

--- The legacy shape, projected back out of the ability spec.
---
--- A projection rather than a passthrough, and both directions matter: `cost` is
--- a list of gauge costs on an ability and a bare mana number on a spell,
--- `cooldown` is a table and a bare number. Everything that has ever read
--- `spell.cost` — `cast`, and the tests that read what `cast` prints — keeps
--- reading a number.
local function project(spec)
    if not spec then return nil end
    local mana = 0
    for _, c in ipairs(spec.cost or {}) do
        if c.trait == "mp" and type(c.amount) == "number" then mana = c.amount end
    end
    return {
        id       = spec.id,
        name     = spec.name,
        summary  = spec.summary,
        cost     = mana,
        cooldown = spec.cooldown and spec.cooldown.seconds or 0,
        target   = spec.target,
        level    = tonumber(spec.level) or 1,
        category = spec.category,
    }
end

function M.get(id)
    local spec = DAEMON.ability and DAEMON.ability.get(id)
    if spec and spec.category == "spell" then return project(spec) end
    return nil
end

function M.all()
    local out = {}
    for _, id in ipairs(DAEMON.ability and DAEMON.ability.all() or {}) do
        if DAEMON.ability.get(id).category == "spell" then out[#out + 1] = id end
    end
    return out
end

--- Which spells this character may cast **now** — the level gate applied.
---
--- `ability_d.known` reports what you have *and* whether the gates pass, because
--- a listing generally wants to say "you have this, but not yet". `cast` has
--- always shown only what you can actually cast, and a spell appearing in the
--- list the level before you can use it would read as a bug.
--- @return table  array of legacy-shaped specs
function M.known(player)
    local out = {}
    for _, entry in ipairs(DAEMON.ability and DAEMON.ability.known(player, "spell") or {}) do
        if entry.usable then out[#out + 1] = project(entry.spec) end
    end
    return out
end

--- What a spell is worth for this caster.
---
--- `spell_power` is derived from intelligence and willpower, and willpower is
--- itself derived — so this reaches through two levels of the trait graph and a
--- wisdom buff changes a fireball without anything here knowing.
--- @return number
function M.power(player)
    return 1 + player:trait("spell_power")
end

--- Cast it. Unchanged signature, unchanged return, unchanged refusals.
--- @return boolean ok, string|nil why
function M.cast(player, id, target_name)
    if not (DAEMON and DAEMON.ability) then return false, "Nothing happens." end
    local spec = M.get(id)
    if not spec then return false, "You do not know any such thing." end
    return DAEMON.ability.use(player, id, { target = target_name })
end

log("info", "spell_d loaded")

return M
