-- mudlib/daemons/combat_d.lua — Rounds, hits, and dying.
--
-- Deliberately small. This exists so the trait and effect systems have
-- something real to act on: an attack resolves through the same pipeline
-- everything else does, so "take 15% less damage" and "negate 5 per hit" are
-- observable in the numbers a player sees rather than only in a test.
--
-- What it does NOT do, and does not pretend to: initiative, groups, positioning,
-- ranged weapons, spell casting, aggro, fleeing in a direction. One attacker,
-- one target, one shared round timer.
--
-- Combat state is memory-tier: a target, an engagement, who swung last. If the
-- server restarts the fight is over, which is the correct answer — writing any
-- of it to disk would be the mistake this whole design exists to avoid.
--
-- Exposes:
--   DAEMON.combat.engage(attacker, target)  -> boolean, reason
--   DAEMON.combat.disengage(entity)
--   DAEMON.combat.disengage_all(char_id)
--   DAEMON.combat.is_fighting(entity)       -> boolean
--   DAEMON.combat.target_of(entity)         -> entity | nil
--   DAEMON.combat.round()                   -- the ticker
--   DAEMON.combat.attack_once(attacker, target) -> table  (one exchange)
--
-- See docs/src/lua-api/combat.md.

local weaponlib = require('components.weapon')
local Combat    = require('lib.combat')
local persist   = require('lib.persist')
local Body      = require('lib.body')

local M = {}

local NS = "combat"

local function conf(key, default)
    local ok, v = pcall(config, key)
    if ok and type(v) == "number" then return v end
    return default
end

--- The defence channels and degree bands a game has registered.
---
--- Held in `persist.root` so a game's registrations survive a hot reload of this
--- file — the same reason `mob_d` keeps its templates there, and the same
--- upvalue-caching idiom because `get_persistent` crosses into Rust.
local CS = nil

local function registry()
    if CS then return CS end
    CS = persist.root("combat_d", 1, function()
        return { channels = {}, degrees = nil }
    end)
    return CS
end

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
        -- Memory tier: holds live entity references, which a persistent tier
        -- could not accept even if we wanted it to.
        DAEMON.cache.define(NS, { tier = "memory" })
    else
        log("error", "COMBAT_D: cache_d is not loaded — combat will not work")
    end
end

--- Replaceable so a test can make a fight deterministic. Production rolls dice.
--- @param n number  upper bound, inclusive
function M._roll(n)
    return math.random(1, n)
end

local function scope_of(entity)
    if type(entity) ~= "table" then return nil end
    if entity.char_id then return "char:" .. tostring(entity.char_id) end
    if entity.id then return "obj:" .. tostring(entity.id) end
    return nil
end

local function tell(entity, text)
    if entity and entity.send then pcall(entity.send, entity, text) end
end

--- Everyone in the room except the two fighting, so a bystander sees the fight.
local function tell_room(entity, text, exclude)
    if not (DAEMON and DAEMON.world) then return end
    local room_id = entity.room_id
    if not room_id and entity.char_id then
        local ok, id = pcall(DAEMON.world.get_character_room, entity.char_id)
        if ok then room_id = id end
    end
    if not room_id then return end

    local ok, room = pcall(DAEMON.world.get_room, room_id)
    if not ok or not room or not room.get_characters then return end
    for _, char_id in ipairs(room:get_characters()) do
        local skip = false
        for _, e in ipairs(exclude or {}) do
            if e and e.char_id == char_id then skip = true end
        end
        if not skip and DAEMON.character then
            local pok, other = pcall(DAEMON.character.get, char_id)
            if pok and other then tell(other, text) end
        end
    end
end

--- What something is called in a message.
---
--- Was a private copy of this rule; it is now `lib/render.lua`'s, so combat and
--- every ability agree, and so `game.display_name_prefers` reaches combat too —
--- a roleplay game reads "You hit a pale wisp", a hack-and-slash reads "You hit
--- wisp", and neither has to patch this daemon.
local function display_name(entity)
    return require('lib.render').display_name(entity)
end

-- ─── Channels and degrees ────────────────────────────────────────────────────

--- Register a way of not being hit.
---
--- A seeded registry the game extends, never a central list — the same
--- construct as `Abilities.checks()`. What stops it rotting is that which
--- channels an entity *has* is decided by which traits it stores, so there is no
--- second list saying who can parry.
--- @param spec table { trait, available = f(defender, attack), why }
--- @return boolean
function M.define_channel(id, spec)
    local normalised, err = Combat.normalise_channel(id, spec)
    if not normalised then
        log_warn("COMBAT_D.define_channel: " .. tostring(err))
        return false
    end
    registry().channels[id] = normalised
    return true
end

function M.channels() return registry().channels end

--- Register what a degree of success is worth.
---
--- The mudlib ships one band at power 1.0, so damage is unchanged until a game
--- says otherwise. What a graze or a decisive blow *does* is content.
--- @return boolean
function M.define_degrees(list)
    local normalised, err = Combat.normalise_degrees(list)
    if not normalised then
        log_warn("COMBAT_D.define_degrees: " .. tostring(err))
        return false
    end
    registry().degrees = normalised
    return true
end

function M.degrees() return registry().degrees or Combat.DEFAULT_DEGREES end

do
    -- Seeded, not listed anywhere else. A game adds `deflect` or `ward` with one
    -- call and this file never learns about it.
    M.define_channel("dodge", { trait = "defense_dodge", why = "you cannot move" })

    M.define_channel("parry", {
        trait = "defense_parry",
        why   = "you have nothing to parry with",
        available = function(defender)
            local worn = type(defender) == "table" and defender.equipment
            local entry = type(worn) == "table" and worn.weapon
            if not entry then return false end
            local item = entry
            if DAEMON and DAEMON.items and DAEMON.items.resolve then
                local ok, resolved = pcall(DAEMON.items.resolve, entry)
                if ok and resolved then item = resolved end
            end
            -- A crossbow is not a parrying implement.
            if weaponlib.is(item) and item.weapon.parry == false then return false end
            return true
        end,
    })

    M.define_channel("block", {
        trait = "defense_block",
        why   = "you have no shield",
        available = function(defender)
            local worn = type(defender) == "table" and defender.equipment
            local entry = type(worn) == "table" and worn.offhand
            if not entry then return false end
            local item = entry
            if DAEMON and DAEMON.items and DAEMON.items.resolve then
                local ok, resolved = pcall(DAEMON.items.resolve, entry)
                if ok and resolved then item = resolved end
            end
            return type(item) == "table" and item.armour ~= nil
                and item.armour.shield == true
        end,
    })
end

-- ─── Engagement ──────────────────────────────────────────────────────────────

--- Start a fight. Both sides are engaged, so the target swings back.
--- @return boolean ok
--- @return string|nil reason
function M.engage(attacker, target)
    if not (DAEMON and DAEMON.cache) then return false, "combat is unavailable" end
    local a_scope, t_scope = scope_of(attacker), scope_of(target)
    if not a_scope or not t_scope then return false, "you cannot attack that" end
    if a_scope == t_scope then return false, "You cannot attack yourself." end
    if not target:is_alive() then return false, "It is already dead." end

    DAEMON.cache.set(NS, a_scope, "self", attacker)
    DAEMON.cache.set(NS, a_scope, "target", target)
    -- The target fights back, unless it is already busy with someone else.
    if not DAEMON.cache.get(NS, t_scope, "target") then
        DAEMON.cache.set(NS, t_scope, "self", target)
        DAEMON.cache.set(NS, t_scope, "target", attacker)
    end

    -- So a game layer can react — a guard assisting its faction, a quest
    -- counting a fight picked. The driver takes no position on what should
    -- happen; that is content, and this is how content finds out.
    if DAEMON.event then
        pcall(DAEMON.event.emit, "combat.started", {
            attacker_char_id = attacker.char_id,
            attacker_id      = attacker.id,
            defender_char_id = target.char_id,
            defender_id      = target.id,
        })
    end
    return true
end

function M.disengage(entity)
    local scope = scope_of(entity)
    if not scope or not (DAEMON and DAEMON.cache) then return false end
    DAEMON.cache.drop(NS, scope)
    return true
end

--- Everything that was fighting this character stops, and so does the
--- character. Called when they disconnect or die.
function M.disengage_all(char_id)
    if not (DAEMON and DAEMON.cache) then return 0 end
    local scope = "char:" .. tostring(char_id)
    local n = 0
    for _, other in ipairs(DAEMON.cache.scopes(NS)) do
        local target = DAEMON.cache.get(NS, other, "target")
        if target and scope_of(target) == scope then
            DAEMON.cache.drop(NS, other)
            n = n + 1
        end
    end
    DAEMON.cache.drop(NS, scope)
    return n
end

function M.is_fighting(entity)
    return M.target_of(entity) ~= nil
end

function M.target_of(entity)
    local scope = scope_of(entity)
    if not scope or not (DAEMON and DAEMON.cache) then return nil end
    return DAEMON.cache.get(NS, scope, "target")
end

-- ─── Resolving one attack ────────────────────────────────────────────────────

--- What this entity hits for, before the defender's effects reduce it.
--- What this attacker is swinging, resolved from the slot. nil for bare hands.
local function wielded(attacker)
    if not (type(attacker) == "table" and attacker.equipment
        and DAEMON and DAEMON.items) then
        return nil
    end
    local entry = attacker.equipment.weapon or attacker.equipment.main_hand
    local item
    if type(entry) == "table" then
        local ok, resolved = pcall(DAEMON.items.resolve, entry)
        if ok then item = resolved end
    elseif type(entry) == "string" then
        local ok, resolved = pcall(DAEMON.items.get, entry)
        if ok then item = resolved end
    end
    return weaponlib.is(item) and item or nil
end

--- **How many swings fit in one of this attacker's rounds.**
---
--- `speed` was authored on every weapon in the game and read by nothing but
--- `examine`. It is a *rate* — the dead `weapon.dps` helper is
--- `avg_damage * speed`, which only type-checks if speed is attacks per unit
--- time — so the time one swing takes is `round_length / speed`, and every
--- authored number already meant the right thing: a dagger at 1.2 swings in
--- 0.83 of a round and a greatsword at 0.7 takes 1.43 of one.
---
--- A creature with nothing in its hands uses its template's `speed`, which is
--- what makes a rat fast and weak rather than merely weak. Without it a rat's
--- rate came from `round_length` alone, and `round_length` moves 0.05s per
--- point of dexterity — so no creature could ever be meaningfully quicker than
--- any other, which is exactly what it looked like in play.
--- @return number  strictly positive
local function swing_rate(attacker)
    local weapon = wielded(attacker)
    local speed
    if weapon then
        speed = tonumber(weapon.weapon.speed)
    else
        speed = tonumber(type(attacker) == "table" and attacker.speed or nil)
    end
    if type(speed) ~= "number" or speed <= 0 then return 1.0 end
    return speed
end

local function weapon_damage(attacker)
    local weapon
    if attacker.equipment and DAEMON and DAEMON.items then
        -- The weapon slot holds an item *instance*, so it resolves against its
        -- template like anything else. It held a bare template id when nothing
        -- wrote the slot at all; an enchanted sword needs the instance, because
        -- the enchantment is what makes it different from every other sword
        -- built from the same template.
        local entry = attacker.equipment.weapon or attacker.equipment.main_hand
        if type(entry) == "table" then
            local ok, item = pcall(DAEMON.items.resolve, entry)
            if ok then weapon = item end
        elseif type(entry) == "string" then
            local ok, item = pcall(DAEMON.items.get, entry)
            if ok then weapon = item end
        end
    end

    -- Rolled through M._roll, not math.random. The class version reached for
    -- math.random itself, which made it a second source of randomness that
    -- nothing overriding _roll could reach — so a test that pinned the fight
    -- still got random weapon damage.
    if weaponlib.is(weapon) then
        local dmg = weaponlib.roll_damage(weapon, M._roll)
        if type(dmg) == "number" then return dmg, weapon end
    end

    -- A template can state its own damage; otherwise it is bare hands, scaled
    -- by strength.
    local spread = attacker.damage
    if type(spread) == "table" and spread.min and spread.max then
        return spread.min + M._roll(math.max(1, spread.max - spread.min + 1)) - 1, nil
    end

    local str = attacker.trait and attacker:trait("strength") or 5
    return math.max(1, M._roll(math.max(1, math.floor(str / 2)))), nil
end

-- ─── Resolution ──────────────────────────────────────────────────────────────

--- What a fighter brings to the contest.
---
--- Falls back to `dexterity` when a game has defined no combat traits, which is
--- what makes the default configuration arithmetically the one-line formula this
--- replaced. A bare table with no `trait` method — a synthetic killer in a test,
--- a room hazard — reads 5, exactly as it did before.
local function rating(entity, trait_id)
    if type(entity) ~= "table" or type(entity.trait) ~= "function" then return 5 end
    if DAEMON and DAEMON.trait and DAEMON.trait.has and DAEMON.trait.has(entity, trait_id) then
        local v = entity:trait(trait_id)
        if type(v) == "number" then return v end
    end
    local dex = entity:trait("dexterity")
    return type(dex) == "number" and dex or 5
end

--- Which defence channels this defender can actually use, and how they divide.
---
--- **Presence is decided by storage**: an entity holds a channel iff it holds
--- that channel's trait. If it holds none, there is one implicit dodge worth the
--- whole pool — which is the no-configuration path, and it is what makes the
--- contest reduce to `60 + (a_dex - d_dex) * 3`.
local function defences(defender, attack)
    local registered = M.channels()
    local alloc, available = {}, {}
    local held = false

    for id, spec in pairs(registered) do
        if DAEMON and DAEMON.trait and DAEMON.trait.has
            and DAEMON.trait.has(defender, spec.trait) then
            held = true
            alloc[id] = defender:trait(spec.trait)
            local ok = true
            if type(spec.available) == "function" then
                local pok, answer = pcall(spec.available, defender, attack)
                ok = pok and answer and true or false
            end
            if ok then available[id] = true end
        end
    end

    if not held then
        return { { id = "dodge", value = rating(defender, "defense_dodge") } }
    end

    return Combat.channels(alloc, rating(defender, "defense"), available, {
        multipliers = attack and attack.defense_multipliers,
        damage_type = attack and attack.damage_type,
    })
end

--- One swing. Returns what happened so the caller can describe it.
--- @return table { hit, damage, dealt, killed, threshold, roll, margin, degree, channel }
function M.attack_once(attacker, target, opts)
    local result = { hit = false, damage = 0, dealt = 0, killed = false }
    if not attacker or not target then return result end
    if not target:is_alive() then result.already_dead = true; return result end
    opts = opts or {}

    local contest = Combat.resolve({
        accuracy            = rating(attacker, "accuracy"),
        accuracy_multiplier = opts.accuracy_multiplier,
        channels            = defences(target, opts),
        bands               = M.degrees(),
        base    = conf("game.combat_base_hit_chance", Combat.DEFAULTS.base),
        step    = conf("game.combat_hit_step",        Combat.DEFAULTS.step),
        floor   = conf("game.combat_hit_floor",       Combat.DEFAULTS.floor),
        ceiling = conf("game.combat_hit_ceiling",     Combat.DEFAULTS.ceiling),
    }, M._roll)

    result.threshold = contest.threshold
    result.roll      = contest.roll
    result.margin    = contest.margin
    result.channel   = contest.channel
    result.degree    = contest.degree
    result.power     = contest.power

    if not contest.hit then return result end

    result.hit = true
    local raw, weapon = weapon_damage(attacker)

    -- Where it landed, when the defender is made of anything. **No layout
    -- consumes no roll**, so a test with pinned dice sees the identical
    -- sequence it saw before locations existed.
    local part = Body.locate(attacker, target, weapon, {
        force = opts.location,
        rng   = M._roll,
    })
    result.location = part
    result.hit_part = part and part.id
    result.hit_slot = part and part.slot

    -- The damage type comes from the weapon, so a silver dagger's `magic`
    -- reaches the defender's resist table and a warded cloak can blunt it.
    -- A creature with no weapon may still declare one on its template — a wisp
    -- deals magic with nothing in its hands — and defaulting to physical here
    -- rather than at the read site means every attacker takes the same path.
    local damage_type = (weaponlib.is(weapon) and weapon.weapon.damage_type)
        or attacker.damage_type
        or "physical"
    result.damage_type = damage_type

    raw = math.max(0, math.floor(Combat.damage(raw, contest, part, damage_type) + 0.5))
    result.damage = raw
    result.weapon = weapon

    local _, dealt = target:take_damage(raw, {
        damage_type = damage_type,
        attacker    = attacker,
        -- Which piece of armour is even in the way. Nil for a layout-less
        -- defender, and the guard downstream is skipped when it is nil — which
        -- is every call the game makes today.
        hit_part = result.hit_part,
        hit_slot = result.hit_slot,
    })
    result.dealt = dealt or raw
    result.killed = not target:is_alive()

    -- `on_combat` was declared on `Mobile` and never called. It is where a
    -- creature's *own* trick goes — a lurker's bite poisoning you, a wisp
    -- marking you — so combat does not grow a special case per monster.
    --
    -- After the damage, so a creature can react to having killed you, and
    -- protected, so a broken hook does not end the fight.
    if type(attacker.on_combat) == "function" then
        local ok, err = pcall(attacker.on_combat, attacker, target)
        if not ok then
            log_error("COMBAT_D: on_combat for '" .. tostring(attacker.id)
                .. "' raised: " .. tostring(err))
        end
    end

    return result
end

--- One attack that is not a weapon swing — an ability's, with its own amount.
---
--- The seam, and the only thing `ability_d` knows about any of this. Everything
--- below the roll is the same code a swing takes, so an ability's damage meets
--- armour, resists, hit locations and the effect pipeline exactly as a sword's
--- does. An ability that reached around this would be the one thing in the game
--- nothing could be designed against.
--- @param opts table { amount, damage_type, accuracy_multiplier,
---                     defense_multipliers, location, source }
--- @return table  the same shape `attack_once` returns
function M.resolve_attack(attacker, defender, opts)
    local result = { hit = false, damage = 0, dealt = 0, killed = false }
    if not (attacker and defender) then return result end
    if not defender:is_alive() then result.already_dead = true; return result end
    opts = opts or {}

    local contest = Combat.resolve({
        accuracy            = rating(attacker, "accuracy"),
        accuracy_multiplier = opts.accuracy_multiplier,
        channels            = defences(defender, opts),
        bands               = M.degrees(),
        base    = conf("game.combat_base_hit_chance", Combat.DEFAULTS.base),
        step    = conf("game.combat_hit_step",        Combat.DEFAULTS.step),
        floor   = conf("game.combat_hit_floor",       Combat.DEFAULTS.floor),
        ceiling = conf("game.combat_hit_ceiling",     Combat.DEFAULTS.ceiling),
    }, M._roll)

    result.threshold, result.roll, result.margin = contest.threshold, contest.roll, contest.margin
    result.channel, result.degree, result.power = contest.channel, contest.degree, contest.power
    if not contest.hit then return result end

    result.hit = true
    local part = Body.locate(attacker, defender, nil, {
        force = opts.location, rng = M._roll,
    })
    result.location = part
    result.hit_part = part and part.id
    result.hit_slot = part and part.slot

    local damage_type = opts.damage_type or "physical"
    result.damage_type = damage_type

    local raw = math.max(0, math.floor(
        Combat.damage(tonumber(opts.amount) or 0, contest, part, damage_type) + 0.5))
    result.damage = raw

    local _, dealt = defender:take_damage(raw, {
        damage_type = damage_type,
        attacker    = attacker,
        hit_part    = result.hit_part,
        hit_slot    = result.hit_slot,
        source      = opts.source,
    })
    result.dealt = dealt or raw
    result.killed = not defender:is_alive()
    return result
end

-- ─── Death ───────────────────────────────────────────────────────────────────

local function reward(killer, victim)
    if not killer or not killer.award_xp then return end
    local template = victim.template_id and DAEMON.mobs and DAEMON.mobs.get(victim.template_id)
    local award = (template and template.xp_award) or (victim.trait and victim:trait("level") * 5) or 5

    -- Through award_xp, so an experience buff applies here and nowhere else
    -- has to know about it.
    local ok, gained = pcall(killer.award_xp, killer, award, { source = "kill" })
    if ok and gained and gained > 0 then
        tell(killer, "You gain " .. gained .. " experience.")
    end

    -- Loot goes on the floor, not into the killer's pack.
    --
    -- It used to go straight to the killer, and the reason was not a design
    -- decision: there was nowhere else for it to go, because nothing in the
    -- mudlib could put an item in a room. Now that ground items exist, dropping
    -- it is what makes `get`, a corpse container, weight limits and someone
    -- else walking in and taking it all mean something. The killer is told what
    -- fell, so nothing is lost that they would have noticed.
    if template and template.loot_table and DAEMON and DAEMON.items then
        local room_id = victim.room_id
            or (DAEMON.world and killer and killer.char_id
                and DAEMON.world.get_character_room(killer.char_id))

        for _, entry in ipairs(template.loot_table) do
            if entry.item_id and (not entry.chance or (M._roll(100) / 100) <= entry.chance) then
                local dropped = nil
                if room_id then
                    local location = DAEMON.items.location("room", room_id)
                    local ok, instance = pcall(DAEMON.items.spawn, entry.item_id, location)
                    if ok then dropped = instance end
                end

                if dropped then
                    local item = DAEMON.items.resolve(dropped)
                    local name = (item and item.short) or entry.item_id
                    tell(killer, name .. " falls from the corpse.")
                    tell_room(victim, name .. " falls from the corpse.", { victim })
                elseif killer and killer.add_item then
                    -- No room to drop it into — a fight in a room the world does
                    -- not know about. Handing it over beats losing it.
                    pcall(killer.add_item, killer, entry.item_id)
                    tell(killer, "You take " .. tostring(entry.item_id) .. " from the corpse.")
                end
            end
        end
    end
end

local function handle_death(killer, victim)
    tell(killer, "{green}" .. display_name(victim) .. " is dead!{/}")
    tell_room(victim, display_name(victim) .. " is dead!", { killer, victim })

    reward(killer, victim)

    M.disengage(victim)
    M.disengage(killer)

    -- A mob leaves the world and may come back; a player is death_d's problem,
    -- and it is already listening for the event Player.on_death emits.
    if victim.template_id and DAEMON and DAEMON.mobs then
        pcall(DAEMON.mobs.despawn, victim, { respawn = true })
    end
end

-- ─── The round ───────────────────────────────────────────────────────────────

--- One exchange between two fighters: resolve, say what happened, handle a death.
---
--- Factored out so `round`, `auto_round` and a queued attack all go through the
--- same body. Nothing here consults roundtime — that is the caller's decision,
--- and it is the difference between the two rounds below.
--- @return boolean  whether an attack was resolved
function M.swing(attacker, target)
    if not (attacker and target) then return false end
    if not (attacker:is_alive() and target:is_alive()) then return false end

    local ok, result = pcall(M.attack_once, attacker, target)
    if not ok then
        log_error("COMBAT_D: attack failed: " .. tostring(result))
        return false
    end

    local a_name, t_name = display_name(attacker), display_name(target)
    if result.hit then
        tell(attacker, "You hit " .. t_name .. " for {red}" .. result.dealt .. "{/} damage.")
        tell(target, a_name .. " hits you for {red}" .. result.dealt .. "{/} damage.")
        tell_room(attacker, a_name .. " hits " .. t_name .. ".", { attacker, target })
    else
        tell(attacker, "You miss " .. t_name .. ".")
        tell(target, a_name .. " misses you.")
    end
    if result.killed then handle_death(attacker, target) end
    return true
end

--- Resolve one round for everyone currently fighting.
---
--- > **This deliberately ignores roundtime.** It is the manual door — a test
--- > driving twenty exchanges inside one clock second, an admin forcing a round
--- > — and it has always meant "one exchange per fighter, now". `auto_round` is
--- > the one that respects the queue's pacing.
--- @return number  attacks resolved
function M.round()
    if not (DAEMON and DAEMON.cache) then return 0 end
    local resolved = 0

    -- Snapshot first: killing a mob mutates the set of fighters, and iterating
    -- it while it changes would be a bug that only shows up when two things
    -- die on the same tick.
    local fights = {}
    for _, scope in ipairs(DAEMON.cache.scopes(NS)) do
        local attacker = DAEMON.cache.get(NS, scope, "self")
        local target   = DAEMON.cache.get(NS, scope, "target")
        if attacker and target then
            fights[#fights + 1] = { scope = scope, attacker = attacker, target = target }
        end
    end

    for _, fight in ipairs(fights) do
        local attacker, target = fight.attacker, fight.target
        -- Either may have died earlier in this same round.
        if attacker:is_alive() and target:is_alive() and M.target_of(attacker) == target then
            if M.swing(attacker, target) then resolved = resolved + 1 end
        elseif not target:is_alive() or not attacker:is_alive() then
            M.disengage(attacker)
        end
    end

    return resolved
end

--- The same, but only for fighters whose combat track is free.
---
--- What the ticker calls. A fighter with something queued has already acted
--- through `queue.tick` and owes roundtime, so this skips them; a fighter with
--- an empty queue and the `auto` policy swings, which is the shipped behaviour
--- unchanged.
--- @return number  attacks resolved
function M.auto_round()
    if not (DAEMON and DAEMON.cache) then return 0 end
    local resolved = 0

    local fights = {}
    for _, scope in ipairs(DAEMON.cache.scopes(NS)) do
        local attacker = DAEMON.cache.get(NS, scope, "self")
        local target   = DAEMON.cache.get(NS, scope, "target")
        if attacker and target then
            fights[#fights + 1] = { attacker = attacker, target = target }
        end
    end

    for _, fight in ipairs(fights) do
        local attacker, target = fight.attacker, fight.target
        if attacker:is_alive() and target:is_alive() and M.target_of(attacker) == target then
            local free = true
            if DAEMON.queue then
                free = not DAEMON.queue.in_roundtime(attacker, "combat")
                    and DAEMON.queue.policy(attacker, "combat") == "auto"
                    and #DAEMON.queue.list(attacker, "combat") == 0
            end
            if DAEMON.ability and DAEMON.ability.casting
                and DAEMON.ability.casting(attacker) then
                free = false
            end
            if free and M.swing(attacker, target) then
                resolved = resolved + 1
                if DAEMON.queue then
                    -- `{ rounds = n }` is multiplicative against `round_length`,
                    -- so the reciprocal of the rate is the time this swing
                    -- costs. This is the whole of what makes a weapon's `speed`
                    -- mean anything, and it needed no new concept.
                    DAEMON.queue.mark(attacker, "combat",
                        { rounds = 1 / swing_rate(attacker) })
                end
            end
        elseif not target:is_alive() or not attacker:is_alive() then
            M.disengage(attacker)
        end
    end

    return resolved
end

-- ─── The combat track ────────────────────────────────────────────────────────

do
    if DAEMON and DAEMON.queue then
        -- Registered here rather than in `queue_d`, because this is the only
        -- thing that knows what a swing is. A game registers `"crafting"` the
        -- same way and touches neither file.
        DAEMON.queue.define_track("combat", {
            round_trait   = "round_length",
            round_seconds = (function()
                local ok, v = pcall(config, "game.combat_round_seconds")
                return (ok and type(v) == "number" and v > 0) and v or 3
            end)(),
            -- An engaged fighter with nothing queued keeps swinging, which is
            -- what combat did before a queue existed. A game that wants "you
            -- stand there unless you say otherwise" sets `idle`.
            empty   = "auto",
            resolve = function(entity, entry)
                if entry.kind == "attack" then
                    local target = entry.target or M.target_of(entity)
                    if not target or not target:is_alive() then return false end
                    if not M.swing(entity, target) then return false end
                    DAEMON.queue.mark(entity, "combat", entry.roundtime or { rounds = 1 })
                    return true
                end
                if entry.kind == "ability" and DAEMON.ability then
                    local ok, _msg, why = DAEMON.ability.use(entity, entry.id,
                        { target = entry.target, from_queue = true })
                    if ok then return true end
                    -- Still on cooldown: keep it. `use` says so rather than
                    -- this matching on the refusal text, because a message is
                    -- for a player and a reason is for a caller.
                    if why == "cooldown" then return "retry" end
                    return false
                end
                return false
            end,
        })
    end
end

-- ─── Ticker ──────────────────────────────────────────────────────────────────

do
    local ok, v = pcall(config, "game.combat_round_seconds")
    local round = (ok and type(v) == "number") and v or 3

    local tok, tv = pcall(config, "game.queue_tick_seconds")
    local interval = (tok and type(tv) == "number") and tv or 1

    -- One repeating timer rather than one per fighter. `ticker_d` holds its
    -- callbacks in a module table that does not survive a hot reload of itself,
    -- so a per-actor timer would strand every fight in the game where this
    -- strands one — and this one is re-registered by the line below on load.
    --
    -- The id is unchanged, because it is what an admin and a test both reach for.
    if round > 0 and interval > 0 and DAEMON and DAEMON.ticker then
        DAEMON.ticker.every(interval, "combat.round", function()
            -- Queued intent first, so it marks roundtime before the default
            -- swing looks to see whether the track is free.
            if DAEMON and DAEMON.queue then
                local qok, qerr = pcall(DAEMON.queue.tick)
                if not qok then log("error", "COMBAT_D: queue tick failed: " .. tostring(qerr)) end
            end
            if DAEMON and DAEMON.combat then
                local rok, err = pcall(DAEMON.combat.auto_round)
                if not rok then
                    log("error", "COMBAT_D: round failed: " .. tostring(err))
                end
            end
        end)
    end
end

log("info", "combat_d daemon loaded")

return M
