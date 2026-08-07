-- mudlib/lib/combat.lua — Whether it lands, and how well.
--
-- The arithmetic half of `combat_d`, the way `lib/traits.lua` is trait_d's and
-- `lib/effects.lua` is effect_d's: no `DAEMON`, no clock, no world, an injected
-- die. Everything here can be driven from a table of numbers, which matters
-- because this is the code that can be *silently* wrong.
--
-- ─── The old formula is the new default, not a second path ───────────────────
--
-- Combat used to be one line:
--
--     chance = clamp(60 + (a_dex - d_dex) * 3, 5, 95)
--
-- The pipeline below computes `clamp(BASE + (A - D) * STEP, FLOOR, CEIL)` and
-- ships `60 / 3 / 5 / 95`, with accuracy and defence both falling back to
-- dexterity. That is arithmetically the same expression — so a game that
-- configures nothing gets exactly what it had, and there is no
-- `if legacy then` anywhere to rot.
--
-- The prettier ratio form `A/(A+D)` was rejected for precisely that reason: it
-- silently rebalances a shipped game (parity becomes 50% instead of 60%) to buy
-- something the four config keys already give.
--
-- ─── Degree of success, and why the margin is out of 100 ─────────────────────
--
--     margin = threshold - roll
--
-- Not `margin / threshold`. The distinction is the whole design:
--
--   a 95%-to-hit attack rolling 3   margin 92   a decisive blow
--   a 10%-to-hit attack rolling 3   margin  7   a graze
--
-- The first is a skill differential expressing itself and should hurt. The
-- second is *luck*, and luck should produce a scrape rather than a decapitation.
-- Normalising by the threshold inverts both.
--
-- What a degree is *worth* is game content: `DEFAULT_DEGREES` is one band at
-- power 1.0, so a game that registers nothing gets today's damage exactly, and
-- a game that wants grazes and critical hits registers a band table.
--
-- Exposes:
--   Combat.DEFAULTS / Combat.DEFAULT_DEGREES
--   Combat.normalise_channel(id, spec)  -> spec, err
--   Combat.normalise_degrees(list)      -> list, err
--   Combat.shares(alloc, available)     -> { [id] = share }
--   Combat.channels(alloc, pool, available, opts) -> array of { id, value }
--   Combat.threshold(accuracy, channels, opts)    -> t, channel, defence
--   Combat.degree(margin, bands)        -> band
--   Combat.resolve(attack, rng)         -> result
--   Combat.damage(base, band, part, damage_type) -> number
--
-- See docs/src/lua-api/combat.md.

local M = {}

--- The knobs, and the values that reproduce what combat did before there was a
--- pipeline to configure.
M.DEFAULTS = { base = 60, step = 3, floor = 5, ceiling = 95 }

--- One band, worth exactly what a hit was worth before degrees existed.
---
--- Deliberately trivial. A band table is a statement about how a game's damage
--- curve feels, and the mudlib has no opinion — it only guarantees that `margin`
--- is computed and reported so a game *can* have one.
M.DEFAULT_DEGREES = { { id = "hit", at = 0, power = 1.0 } }

local function clamp(v, lo, hi)
    if v < lo then return lo end
    if v > hi then return hi end
    return v
end

-- ─── Specs ───────────────────────────────────────────────────────────────────

--- Fill a defence channel out, or say why it cannot be one.
---
--- A channel is a registry entry rather than a hardcoded name, the same shape as
--- `Abilities.checks()`. What stops it rotting is that **which channels an
--- entity has is decided by which traits it stores** — there is no list anywhere
--- saying who can parry.
--- @return table|nil spec, string|nil err
function M.normalise_channel(id, spec)
    if type(id) ~= "string" or id == "" then return nil, "a channel needs a string id" end
    if type(spec) ~= "table" then spec = {} end
    spec.id = id
    spec.trait = spec.trait or ("defense_" .. id)
    if spec.available ~= nil and type(spec.available) ~= "function" then
        return nil, "a channel's `available` must be a function"
    end
    spec.why = spec.why or ("you cannot " .. id)
    return spec
end

--- Sort a band table so a lookup is a walk from the top.
--- @return table|nil list, string|nil err
function M.normalise_degrees(list)
    if type(list) ~= "table" then return nil, "a degree table must be an array" end
    local out = {}
    for _, band in ipairs(list) do
        if type(band) ~= "table" or type(band.id) ~= "string" then
            return nil, "each degree needs a string id"
        end
        out[#out + 1] = {
            id = band.id,
            at = tonumber(band.at) or 0,
            power = tonumber(band.power) or 1.0,
            reroll_location = band.reroll_location and true or false,
        }
    end
    if #out == 0 then return nil, "a degree table needs at least one band" end
    table.sort(out, function(a, b)
        if a.at ~= b.at then return a.at > b.at end
        return a.id < b.id
    end)
    return out
end

-- ─── Defence channels ────────────────────────────────────────────────────────

--- How a defender's effort divides between the channels they can actually use.
---
--- **Normalisation over the available set, and that is the entire rule.** A
--- channel you allocated effort to and cannot use costs you nothing: drop it,
--- then divide. There is no leftover to redistribute in a second pass and so
--- nothing to get wrong.
--- @param alloc table  { [channel id] = weight }
--- @param available table  { [channel id] = true }
--- @return table  { [channel id] = share summing to 1 }
function M.shares(alloc, available)
    local total, kept = 0, {}
    for id, weight in pairs(alloc or {}) do
        local w = tonumber(weight) or 0
        if w > 0 and (available == nil or available[id]) then
            kept[id] = w
            total = total + w
        end
    end
    local out = {}
    if total <= 0 then return out end
    for id, w in pairs(kept) do out[id] = w / total end
    return out
end

--- What each usable channel is worth to this defender.
--- @param opts table|nil { multipliers = { [id] = n }, resists = { [id] = { [type] = n } },
---                         damage_type = "..." }
--- @return table  array of { id, value }, strongest first
function M.channels(alloc, pool, available, opts)
    opts = opts or {}
    local shares = M.shares(alloc, available)
    local out = {}
    for id, share in pairs(shares) do
        local value = (tonumber(pool) or 0) * share
        local mult = opts.multipliers and opts.multipliers[id]
        if mult then value = value * mult end
        local resist = opts.resists and opts.resists[id]
        if resist and opts.damage_type and resist[opts.damage_type] then
            value = value * (1 + resist[opts.damage_type])
        end
        out[#out + 1] = { id = id, value = value }
    end
    -- Strongest first, ties by id — nothing here is decided by `pairs` order.
    table.sort(out, function(a, b)
        if a.value ~= b.value then return a.value > b.value end
        return a.id < b.id
    end)
    return out
end

-- ─── The contest ─────────────────────────────────────────────────────────────

--- The number a d100 has to come in at or under.
--- @return number threshold, string|nil channel, number defence
function M.threshold(accuracy, channels, opts)
    opts = opts or {}
    local base    = tonumber(opts.base)    or M.DEFAULTS.base
    local step    = tonumber(opts.step)    or M.DEFAULTS.step
    local floor   = tonumber(opts.floor)   or M.DEFAULTS.floor
    local ceiling = tonumber(opts.ceiling) or M.DEFAULTS.ceiling

    local defence, channel = 0, nil
    for _, c in ipairs(channels or {}) do
        if channel == nil or c.value > defence then
            defence, channel = c.value, c.id
        end
    end

    local t = base + ((tonumber(accuracy) or 0) - defence) * step
    -- Floored to a whole number, because the die is whole.
    return clamp(math.floor(t + 0.5), floor, ceiling), channel, defence
end

--- Which band a margin falls in. The table is sorted descending, so the first
--- one it reaches is the answer.
function M.degree(margin, bands)
    bands = bands or M.DEFAULT_DEGREES
    for _, band in ipairs(bands) do
        if margin >= band.at then return band end
    end
    return bands[#bands]
end

--- One attack, start to finish, as plain data in and plain data out.
---
--- `rng` is injected so a test can pin every die — the same contract
--- `combat_d._roll` has always had.
--- @param attack table { accuracy, accuracy_multiplier, channels, bands,
---                       base, step, floor, ceiling }
--- @return table { threshold, roll, margin, hit, degree, power, channel, defence }
function M.resolve(attack, rng)
    attack = attack or {}
    local roll_fn = rng or function(n) return math.random(n) end

    local accuracy = (tonumber(attack.accuracy) or 0)
        * (tonumber(attack.accuracy_multiplier) or 1)

    local threshold, channel, defence = M.threshold(accuracy, attack.channels, attack)
    local roll = roll_fn(100)
    local margin = threshold - roll
    local hit = margin >= 0

    local band = hit and M.degree(margin, attack.bands) or nil

    return {
        threshold = threshold,
        roll      = roll,
        margin    = margin,
        hit       = hit,
        degree    = band and band.id or nil,
        power     = band and band.power or 1.0,
        reroll_location = band and band.reroll_location or false,
        channel   = channel,
        defence   = defence,
    }
end

-- ─── Damage ──────────────────────────────────────────────────────────────────

--- Compose a rolled amount with the degree and the place it landed.
---
--- `part.vulnerable` is **proportional** and deliberately named differently from
--- armour's `resist`, which is flat. `+0.25` means a quarter more.
--- @param part table|nil  the body part struck
--- @return number
function M.damage(base, band, part, damage_type)
    local out = (tonumber(base) or 0) * ((band and tonumber(band.power)) or 1.0)
    if type(part) == "table" then
        if part.multiplier then out = out * (tonumber(part.multiplier) or 1) end
        local vuln = part.vulnerable and damage_type and part.vulnerable[damage_type]
        if vuln then out = out * (1 + vuln) end
    end
    return out
end

return M
