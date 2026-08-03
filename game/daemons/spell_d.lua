-- game/daemons/spell_d.lua — Casting.
--
-- Game layer, because what magic *is* is content. The mudlib provides gauges,
-- effects, a damage pipeline, cooldowns and a trait graph; a spell is those
-- five things arranged, and arranging them differently is what makes one game's
-- magic different from another's.
--
-- ─── What each spell exercises ───────────────────────────────────────────────
--
--   emberlance   damage through the `damage_taken` pipeline, so armour and
--                resists apply to a spell exactly as they do to a sword
--   mend         a gauge `adjust`, which settles regeneration first — so the
--                heal lands on the value as it is *now*
--   wardskin     an effect with a `condition`, refused before it lands
--   farsight     a **memory-tier** cooldown, under the durable threshold, and
--                so correctly forgotten on restart
--
-- ─── The cost model ──────────────────────────────────────────────────────────
--
-- Mana is a gauge, so it is spent with `adjust` and never with a modifier: a
-- spell that "modified" your mana would be a buff you have to unapply, which is
-- the mistake the whole effect design avoids. Power scales with `spell_power`,
-- a derived-of-derived trait, so a wisdom buff reaches a fireball through two
-- levels of the graph without anything here knowing.

local M = {}

M._spells = {}

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

--- @param spec table  { id, name, cost, cooldown, target, level, cast }
--- @return boolean
function M.register(spec)
    if type(spec) ~= "table" or type(spec.id) ~= "string" then
        log("warn", "SPELL_D.register: a spell needs a string id")
        return false
    end
    if type(spec.cast) ~= "function" then
        log("warn", "SPELL_D.register('" .. spec.id .. "'): a spell needs a `cast`")
        return false
    end

    M._spells[spec.id] = {
        id       = spec.id,
        name     = spec.name or spec.id,
        summary  = spec.summary or "",
        cost     = tonumber(spec.cost) or 0,
        cooldown = tonumber(spec.cooldown) or 0,
        -- "self" | "creature" | "none". Checked here so every spell refuses
        -- the same way rather than each one inventing a message.
        target   = spec.target or "none",
        level    = tonumber(spec.level) or 1,
        cast     = spec.cast,
    }
    return true
end

function M.register_all(list)
    local n = 0
    for _, spec in ipairs(list or {}) do
        if M.register(spec) then n = n + 1 end
    end
    log("info", "SPELL_D: registered " .. n .. " spell(s)")
    return n
end

function M.get(id) return M._spells[id] end

function M.all()
    local out = {}
    for id in pairs(M._spells) do out[#out + 1] = id end
    table.sort(out)
    return out
end

--- Which spells this character may cast, by level.
--- @return table  array of spells
function M.known(player)
    local out = {}
    for _, id in ipairs(M.all()) do
        local spell = M._spells[id]
        if player:trait("level") >= spell.level then out[#out + 1] = spell end
    end
    return out
end

--- What the spell is worth for this caster.
---
--- `spell_power` is derived from intelligence and willpower, and willpower is
--- itself derived — so this reaches through two levels of the trait graph and
--- a wisdom buff changes a fireball without anything here knowing.
--- @return number
function M.power(player)
    return 1 + player:trait("spell_power")
end

local function cooldown_key(id) return "spell." .. id end

--- Cast it.
--- @param player table
--- @param id string
--- @param target_name string|nil
--- @return boolean ok, string|nil why
function M.cast(player, id, target_name)
    local spell = M._spells[id]
    if not spell then return false, "You do not know any such thing." end
    if player:trait("level") < spell.level then
        return false, "That is beyond you for now."
    end

    -- A **memory-tier** cooldown when it is short. `cooldown_d` chooses the
    -- tier by duration, which is the same rule as everywhere else: a six-second
    -- gate is not worth a database write and losing it on a restart is correct.
    if spell.cooldown > 0 and DAEMON and DAEMON.cooldown then
        if not DAEMON.cooldown.ready(player.char_id, cooldown_key(id)) then
            local left = DAEMON.cooldown.remaining(player.char_id, cooldown_key(id))
            return false, "Not yet. (" .. math.ceil(left) .. "s)"
        end
    end

    -- Resolve the target before spending anything, so a mistyped name does not
    -- cost mana.
    local target = nil
    if spell.target == "self" then
        target = player
    elseif spell.target == "creature" then
        if not target_name or target_name == "" then
            return false, "Cast it at what?"
        end
        local room_id = DAEMON.world and DAEMON.world.get_character_room(player.char_id)
        local ok, found = pcall(DAEMON.mobs.find_in_room, room_id or "", target_name)
        target = ok and found or nil
        if not target then return false, "There is no " .. target_name .. " here." end
        if target.is_alive and not target:is_alive() then
            return false, "It is already dead."
        end
    end

    -- Mana. A gauge, so it is *spent* rather than modified — `adjust` settles
    -- regeneration first, so the cost comes off the value as it is now rather
    -- than as it was when the bar was last read.
    if spell.cost > 0 then
        if player:trait("mp") < spell.cost then
            return false, "You have not the mana for that."
        end
        DAEMON.trait.adjust(player, "mp", -spell.cost)
    end

    if spell.cooldown > 0 and DAEMON and DAEMON.cooldown then
        DAEMON.cooldown.mark(player.char_id, cooldown_key(id), spell.cooldown)
    end

    local ok, err = pcall(spell.cast, player, target, M.power(player))
    if not ok then
        log_error("SPELL_D: casting '" .. id .. "' raised: " .. tostring(err))
        -- The mana is gone. Refunding on an error would make a spell that
        -- throws halfway through free, which is worse than one that costs.
        return false, "The working comes apart in your hands."
    end

    if DAEMON and DAEMON.event then
        pcall(DAEMON.event.emit, "spell.cast", {
            char_id = player.char_id, spell = id,
            target = target and target.id,
        })
    end
    return true
end

log("info", "spell_d loaded")

return M
