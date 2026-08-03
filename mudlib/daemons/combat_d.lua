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

local weaponlib = require('lib.weapon')

local M = {}

local NS = "combat"

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
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

local function display_name(entity)
    if entity.name then return entity.name end
    local Object = require('lib.object')
    return Object.resolve(entity.short, entity) or "something"
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

--- One swing. Returns what happened so the caller can describe it.
--- @return table { hit, damage, dealt, killed, message }
function M.attack_once(attacker, target)
    local result = { hit = false, damage = 0, dealt = 0, killed = false }
    if not attacker or not target then return result end
    if not target:is_alive() then result.already_dead = true; return result end

    -- To hit: even-ish, nudged by the difference in dexterity, and never a
    -- certainty in either direction.
    local a_dex = attacker.trait and attacker:trait("dexterity") or 5
    local d_dex = target.trait and target:trait("dexterity") or 5
    local chance = math.max(5, math.min(95, 60 + (a_dex - d_dex) * 3))
    if M._roll(100) > chance then
        return result
    end

    result.hit = true
    local raw, weapon = weapon_damage(attacker)
    result.damage = raw
    result.weapon = weapon

    -- The damage type comes from the weapon, so a silver dagger's `magic`
    -- reaches the defender's resist table and a warded cloak can blunt it.
    -- A creature with no weapon may still declare one on its template — a wisp
    -- deals magic with nothing in its hands — and defaulting to physical here
    -- rather than at the read site means every attacker takes the same path.
    local damage_type = (weaponlib.is(weapon) and weapon.weapon.damage_type)
        or attacker.damage_type
        or "physical"
    result.damage_type = damage_type

    local _, dealt = target:take_damage(raw, {
        damage_type = damage_type,
        attacker = attacker,
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

--- Resolve one round for everyone currently fighting.
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
            local ok, result = pcall(M.attack_once, attacker, target)
            if not ok then
                log_error("COMBAT_D: attack failed: " .. tostring(result))
            else
                resolved = resolved + 1
                local a_name, t_name = display_name(attacker), display_name(target)
                if result.hit then
                    tell(attacker, "You hit " .. t_name .. " for {red}" .. result.dealt .. "{/} damage.")
                    tell(target, a_name .. " hits you for {red}" .. result.dealt .. "{/} damage.")
                    tell_room(attacker, a_name .. " hits " .. t_name .. ".", { attacker, target })
                else
                    tell(attacker, "You miss " .. t_name .. ".")
                    tell(target, a_name .. " misses you.")
                end
                if result.killed then
                    handle_death(attacker, target)
                end
            end
        elseif not target:is_alive() or not attacker:is_alive() then
            M.disengage(attacker)
        end
    end

    return resolved
end

-- ─── Ticker ──────────────────────────────────────────────────────────────────

do
    local ok, v = pcall(config, "game.combat_round_seconds")
    local interval = (ok and type(v) == "number") and v or 3
    if interval > 0 and DAEMON and DAEMON.ticker then
        DAEMON.ticker.every(interval, "combat.round", function()
            if DAEMON and DAEMON.combat then
                local rok, err = pcall(DAEMON.combat.round)
                if not rok then
                    log("error", "COMBAT_D: round failed: " .. tostring(err))
                end
            end
        end)
    end
end

log("info", "combat_d daemon loaded")

return M
