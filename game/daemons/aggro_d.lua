-- game/daemons/aggro_d.lua — Creatures that attack on sight.
--
-- `Mobile.aggressive` has been on every mob template since the class existed
-- and **nothing read it**. `combat.md` is explicit that this is deliberate:
-- whether an aggressive creature attacks, how long it waits, whether it cares
-- about level or faction, and whether it gives up when you flee are all *game*
-- decisions. The driver provides the flag and the `room.entered` event; the
-- policy is content.
--
-- Which is why this lives in `game/daemons/` and not in the mudlib. A different
-- game wants a different rule, and the way to have one is to write a different
-- file rather than to configure this one into shapelessness.
--
-- ─── The policy this game has ────────────────────────────────────────────────
--
--  * Aggressive creatures attack a player who walks in, after a short delay —
--    long enough to read the room description and turn round, which is the
--    difference between a threat and an ambush.
--  * They ignore anyone more than `IGNORE_ABOVE` levels above them. A rat that
--    suicides into a passing archmage is comedy the first time and tedium the
--    second.
--  * A creature with a faction ignores members of that faction, and assists
--    other members of it who are already fighting.
--  * Aggro is memory-tier state by the rule in state-cache.md: who a rat is
--    cross with is not worth a database write, and a restart forgetting it is
--    the correct outcome.

local M = {}

--- How long a player gets between arriving and being attacked.
local ATTACK_DELAY = 3

--- Levels above a creature at which it stops caring.
local IGNORE_ABOVE = 8

--- Cache namespace for "this creature has already noticed someone". Memory
--- tier: sub-minute, worthless after a restart, and rewritten constantly.
local NS = "aggro"

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

--- Would this creature attack this character?
--- @param mob table
--- @param player table
--- @return boolean
function M.would_attack(mob, player)
    if type(mob) ~= "table" or type(player) ~= "table" then return false end
    if not mob.aggressive then return false end
    if mob.is_alive and not mob:is_alive() then return false end

    -- Same side. `faction` was another field nothing read.
    if mob.faction and player.faction and mob.faction == player.faction then
        return false
    end

    local mob_level = mob.trait and mob:trait("level") or 1
    local player_level = player.trait and player:trait("level") or 1
    if player_level - mob_level > IGNORE_ABOVE then return false end

    return true
end

--- Everything hostile standing in one room.
--- @param room_id string
--- @param player table
--- @return table  array of mobs
function M.hostiles_in(room_id, player)
    if not (DAEMON and DAEMON.mobs) then return {} end
    local ok, mobs = pcall(DAEMON.mobs.in_room, room_id)
    if not ok or type(mobs) ~= "table" then return {} end

    local out = {}
    for _, mob in ipairs(mobs) do
        if M.would_attack(mob, player) then out[#out + 1] = mob end
    end
    return out
end

--- Start a fight, once the delay has passed and if everyone is still here.
---
--- Re-checked at fire time rather than trusted from when it was scheduled: the
--- player may have walked out, the creature may have been killed by someone
--- else, and a timer that acts on a three-second-old world picks fights in
--- rooms nobody is in.
local function strike(mob_id, char_id, room_id)
    if not (DAEMON and DAEMON.mobs and DAEMON.combat and DAEMON.world) then return end

    local mob = DAEMON.mobs.get_instance(mob_id)
    if not mob or (mob.is_alive and not mob:is_alive()) then return end
    if mob.room_id ~= room_id then return end

    if DAEMON.world.get_character_room(char_id) ~= room_id then return end

    local player = DAEMON.character and DAEMON.character.get(char_id)
    if not player or (player.is_alive and not player:is_alive()) then return end
    if not M.would_attack(mob, player) then return end

    -- Already fighting somebody is enough; a creature does not need to pick a
    -- second target to be a threat.
    if DAEMON.combat.target_of and DAEMON.combat.target_of(mob) then return end

    if player.send then
        pcall(player.send, player,
            "{red}" .. (mob.short or mob.name or "Something")
            .. " notices you and attacks!{/}")
    end
    pcall(DAEMON.combat.engage, mob, player)
end

--- Somebody walked into a room. Anything hostile here notices.
function M.on_room_entered(data)
    if type(data) ~= "table" then return end
    local char_id, room_id = data.char_id, data.room_id
    if not char_id or not room_id then return end

    local player = DAEMON.character and DAEMON.character.get(char_id)
    if not player then return end

    local hostiles = M.hostiles_in(room_id, player)
    if #hostiles == 0 then return end

    for _, mob in ipairs(hostiles) do
        -- Deliberately silent here. `room.entered` is emitted by `world_d` at
        -- the moment the move lands, which is *before* `movement.lua` sends the
        -- room description — so a warning printed now would appear above the
        -- room the player is reading, which reads as belonging to the room they
        -- just left. The notice is part of the attack instead.

        -- One timer per (creature, target). The id is deterministic so a player
        -- walking in and out repeatedly re-arms the same timer rather than
        -- stacking one per visit — `ticker.after` replaces by id, which is what
        -- makes that free.
        if DAEMON.ticker then
            local timer_id = "aggro." .. mob.id .. "." .. tostring(char_id)
            local ok, err = pcall(DAEMON.ticker.after, ATTACK_DELAY, timer_id, function()
                local sok, serr = pcall(strike, mob.id, char_id, room_id)
                if not sok then log_error("AGGRO_D: strike failed: " .. tostring(serr)) end
            end)
            if not ok then log_error("AGGRO_D: could not arm timer: " .. tostring(err)) end
        end
    end
end

--- A creature's allies join in. `faction` finally reads.
function M.on_combat_started(data)
    if type(data) ~= "table" then return end
    if not (DAEMON and DAEMON.mobs and DAEMON.combat and DAEMON.character) then return end

    local defender = data.defender_id and DAEMON.mobs.get_instance(data.defender_id)
    if not defender or not defender.faction then return end
    if not defender.room_id then return end

    local attacker = data.attacker_char_id
        and DAEMON.character.get(data.attacker_char_id)
    if not attacker then return end

    local ok, bystanders = pcall(DAEMON.mobs.in_room, defender.room_id)
    if not ok then return end

    for _, mob in ipairs(bystanders) do
        if mob.id ~= defender.id and mob.faction == defender.faction
            and (not mob.is_alive or mob:is_alive())
            and not DAEMON.combat.target_of(mob) then
            pcall(DAEMON.combat.engage, mob, attacker)
        end
    end
end

--- Stop caring about anyone who left the world.
function M.on_character_left(data)
    if type(data) ~= "table" or not data.char_id then return end
    if DAEMON and DAEMON.ticker and DAEMON.ticker.remove_by_prefix then
        -- Every armed strike aimed at this character, whoever armed it. The
        -- suffix is the char id, so this is a suffix problem rather than a
        -- prefix one — walk the list instead.
        local ok, timers = pcall(DAEMON.ticker.list)
        if ok and type(timers) == "table" then
            local suffix = "." .. tostring(data.char_id)
            for _, timer in ipairs(timers) do
                local id = type(timer) == "table" and timer.id or timer
                if type(id) == "string" and id:sub(1, 6) == "aggro."
                    and id:sub(-#suffix) == suffix then
                    pcall(DAEMON.ticker.remove, id)
                end
            end
        end
    end
end

-- ─── Wiring ──────────────────────────────────────────────────────────────────

if DAEMON and DAEMON.event then
    local ok, err = pcall(function()
        DAEMON.event.on("room.entered", "aggro_d.entered", M.on_room_entered)
        DAEMON.event.on("combat.started", "aggro_d.assist", M.on_combat_started)
        DAEMON.event.on("character.left", "aggro_d.left", M.on_character_left)
    end)
    if not ok then log_error("AGGRO_D: could not subscribe: " .. tostring(err)) end
else
    log("warn", "AGGRO_D: no event daemon — aggressive creatures will not notice anyone")
end

-- Unused by this file, but part of its contract: a game daemon that wants to
-- ask "would this thing attack me" should not have to reimplement the rule.
M.NS = NS

log("info", "aggro_d loaded")

return M
