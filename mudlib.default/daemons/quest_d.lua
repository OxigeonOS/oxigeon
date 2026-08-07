-- game/daemons/quest_d.lua — Quests, and where each kind of quest state lives.
--
-- `Player:set_quest_flag` / `get_quest_flag` / `has_quest_flag` and the
-- `quest:` effect source scheme all existed and had **no callers**. This is the
-- system that uses them, and it is in the game layer because quest design is
-- content — the driver has no opinion about what a quest is.
--
-- ─── Three tiers, on purpose ─────────────────────────────────────────────────
--
-- The interesting thing about a quest system is that it needs all three
-- persistence tiers at once, and choosing wrongly is invisible until it is not:
--
--   quest_flags     "have I ever finished this"     -> a SAVE_FIELD on the
--                   Player. A forever answer, and it is already saved.
--   counters        "how many rats so far"          -> the write-behind cache.
--                   Losing thirty seconds of progress on a crash is annoying;
--                   paying a database write per rat is a design mistake.
--   daily gates     "have I done this today"        -> `DAEMON.cooldown`, over
--                   the durable threshold so it is written through. **Not**
--                   room object state, which an area reset wipes — that is the
--                   bug `task_list.md` opens with.
--
-- ─── What a quest is ─────────────────────────────────────────────────────────
--
--   { id, name, summary, giver, level,
--     requires = { flag = "...", level = n },   -- what gates offering it
--     objective = { kind = "kill"|"collect"|"deliver"|"visit", target, count },
--     reward = { xp, gold, items, effect },
--     repeatable = false | "daily",
--     on_complete = function(player) end }

local M = {}

--- Namespace for the counters. Write-behind: a kill counter is exactly the
--- example the tier exists for.
local NS = "quests"

M._quests = {}
--- giver template id -> array of quest ids, so `talk` can offer without a scan.
M._by_giver = {}

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

local function log_warn(message)
    log("warn", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.warn, message) end
end

-- ─── Registration ────────────────────────────────────────────────────────────

--- @param spec table
--- @return boolean
function M.register(spec)
    if type(spec) ~= "table" or type(spec.id) ~= "string" or #spec.id == 0 then
        log_warn("QUEST_D.register: a quest needs a string id")
        return false
    end
    if type(spec.objective) ~= "table" or type(spec.objective.kind) ~= "string" then
        log_warn("QUEST_D.register('" .. spec.id .. "'): a quest needs an objective")
        return false
    end

    local quest = {
        id         = spec.id,
        name       = spec.name or spec.id,
        summary    = spec.summary or "",
        giver      = spec.giver,
        level      = tonumber(spec.level) or 1,
        requires   = type(spec.requires) == "table" and spec.requires or {},
        objective  = spec.objective,
        reward     = type(spec.reward) == "table" and spec.reward or {},
        repeatable = spec.repeatable,
        on_complete = spec.on_complete,
        -- What to say when it is handed in. Optional, because most quests are
        -- fine with the generic line.
        completion = spec.completion,
    }
    quest.objective.count = tonumber(quest.objective.count) or 1

    M._quests[quest.id] = quest
    if quest.giver then
        M._by_giver[quest.giver] = M._by_giver[quest.giver] or {}
        table.insert(M._by_giver[quest.giver], quest.id)
    end
    return true
end

function M.register_all(list)
    if type(list) ~= "table" then
        log_warn("QUEST_D.register_all: expected an array of specs")
        return 0
    end
    local n = 0
    for _, spec in ipairs(list) do
        if M.register(spec) then n = n + 1 end
    end
    log("info", "QUEST_D: registered " .. n .. " quest(s)")
    return n
end

function M.get(id) return M._quests[id] end

function M.all()
    local out = {}
    for id in pairs(M._quests) do out[#out + 1] = id end
    table.sort(out)
    return out
end

--- What this creature has to offer, filtered to what the player may take.
--- @return table  array of quests
function M.offers(giver_template, player)
    local out = {}
    for _, id in ipairs(M._by_giver[giver_template] or {}) do
        local quest = M._quests[id]
        if quest and M.can_accept(player, id) then out[#out + 1] = quest end
    end
    return out
end

-- ─── State ───────────────────────────────────────────────────────────────────

local function flag_active(id)   return "quest.active." .. id end
local function flag_done(id)     return "quest.done." .. id end
local function cooldown_of(id)   return "quest.daily." .. id end

--- Has this character ever finished it? A forever answer, so it lives in
--- `quest_flags`, which is already a SAVE_FIELD.
--- @return boolean
function M.is_complete(player, id)
    return type(player) == "table" and player.has_quest_flag
        and player:has_quest_flag(flag_done(id)) == true
end

--- @return boolean
function M.is_active(player, id)
    return type(player) == "table" and player.has_quest_flag
        and player:has_quest_flag(flag_active(id)) == true
end

--- Progress on the objective.
---
--- Write-behind, not a character field: a kill counter is written on every kill
--- and read almost never, which is exactly the shape the tier exists for.
--- Putting it in `player.custom` would rewrite the whole character blob on
--- every rat.
--- @return number
function M.progress(player, id)
    if not (DAEMON and DAEMON.cache) or type(player) ~= "table" then return 0 end
    local ok, n = pcall(DAEMON.cache.get, NS, "char:" .. tostring(player.char_id), id)
    return (ok and tonumber(n)) or 0
end

--- @return number  the new total
function M.advance(player, id, by)
    if not (DAEMON and DAEMON.cache) or type(player) ~= "table" then return 0 end
    local quest = M._quests[id]
    if not quest or not M.is_active(player, id) then return 0 end

    local scope = "char:" .. tostring(player.char_id)
    local now = M.progress(player, id) + (tonumber(by) or 1)
    -- Clamped, so a counter cannot run away past what the quest asks for and
    -- report "12 / 5".
    now = math.min(now, quest.objective.count)
    pcall(DAEMON.cache.set, NS, scope, id, now)

    if player.send then
        if now >= quest.objective.count then
            player:send("{green}" .. quest.name .. ": complete. Return to "
                .. (quest.giver_name or "whoever sent you") .. ".{/}")
        else
            player:send("{cyan}" .. quest.name .. ": " .. now .. " / "
                .. quest.objective.count .. "{/}")
        end
    end
    return now
end

--- @return boolean
function M.is_ready(player, id)
    local quest = M._quests[id]
    if not quest or not M.is_active(player, id) then return false end
    return M.progress(player, id) >= quest.objective.count
end

-- ─── Taking and finishing ────────────────────────────────────────────────────

--- May this character take it on?
--- @return boolean ok, string|nil why
function M.can_accept(player, id)
    local quest = M._quests[id]
    if not quest then return false, "There is no such task." end
    if M.is_active(player, id) then return false, "You are already doing that." end

    if M.is_complete(player, id) then
        if quest.repeatable == "daily" then
            if DAEMON and DAEMON.cooldown
                and not DAEMON.cooldown.ready(player.char_id, cooldown_of(id)) then
                local left = DAEMON.cooldown.remaining(player.char_id, cooldown_of(id))
                return false, "Not again today. (" .. math.ceil(left / 3600) .. "h)"
            end
        elseif not quest.repeatable then
            return false, "You have already done that."
        end
    end

    if quest.requires.level and player:trait("level") < quest.requires.level then
        return false, "You are not ready for that yet."
    end
    -- A chain: this quest is gated on having finished another one. The flag is
    -- the mechanism, and it is the same flag the earlier quest set.
    if quest.requires.flag and not player:has_quest_flag(quest.requires.flag) then
        return false, "There is something else to do first."
    end

    return true
end

--- @return boolean ok, string|nil why
function M.accept(player, id)
    local ok, why = M.can_accept(player, id)
    if not ok then return false, why end

    local quest = M._quests[id]
    player:set_quest_flag(flag_active(id), true)
    -- Start from zero rather than from whatever a previous attempt left: a
    -- daily quest taken a second time must not begin already finished.
    if DAEMON and DAEMON.cache then
        pcall(DAEMON.cache.set, NS, "char:" .. tostring(player.char_id), id, 0)
    end

    if player.send then
        player:send("{green}Accepted: " .. quest.name .. "{/}")
        if quest.summary ~= "" then player:send("  " .. quest.summary) end
    end
    if DAEMON and DAEMON.event then
        pcall(DAEMON.event.emit, "quest.accepted", {
            char_id = player.char_id, quest = id,
        })
    end
    return true
end

--- Hand it in.
--- @return boolean ok, string|nil why
function M.complete(player, id)
    local quest = M._quests[id]
    if not quest then return false, "There is no such task." end
    if not M.is_active(player, id) then return false, "You are not doing that." end
    if not M.is_ready(player, id) then
        return false, "You have not finished: " .. M.progress(player, id)
            .. " / " .. quest.objective.count
    end

    -- A `collect` quest takes the items. Checked *and* taken here rather than
    -- at hand-in time by the caller, so there is one place that can get the
    -- ordering wrong and it is this one.
    if quest.objective.kind == "collect" then
        local Carry = require('lib.carry')
        local taken = 0
        for _ = 1, quest.objective.count do
            -- `any`: the quest named the item, not the player, so there is
            -- nobody to ask which of three identical roots they meant.
            local entry = select(1, Carry.find(player, quest.objective.target,
                { inventory = true, room = false, any = true }))
            if not entry then break end
            for i, e in ipairs(player.inventory) do
                if e == entry then table.remove(player.inventory, i) break end
            end
            pcall(DAEMON.items.destroy, entry)
            taken = taken + 1
        end
        if taken < quest.objective.count then
            return false, "You do not have them all any more."
        end
    end

    -- `clear`, not `set(..., nil)`: a missing value means `true`, so setting
    -- it to nil would leave the quest permanently active.
    player:clear_quest_flag(flag_active(id))
    player:set_quest_flag(flag_done(id), true)

    if quest.repeatable == "daily" and DAEMON and DAEMON.cooldown then
        -- Durable, because it is over the threshold — so it survives a restart
        -- *and* an area reset. A daily gate on room state is the original bug.
        DAEMON.cooldown.mark(player.char_id, cooldown_of(id), 24 * 3600)
    end

    M.reward(player, quest)

    if player.send then
        player:send("{green}" .. (quest.completion or (quest.name .. ": done.")) .. "{/}")
    end
    if type(quest.on_complete) == "function" then
        local ok, err = pcall(quest.on_complete, player)
        if not ok then
            log_error("QUEST_D: on_complete for '" .. id .. "' raised: " .. tostring(err))
        end
    end
    if DAEMON and DAEMON.event then
        pcall(DAEMON.event.emit, "quest.completed", {
            char_id = player.char_id, quest = id,
        })
    end
    return true
end

--- Give up. The counter goes with it, because a quest you abandoned and took
--- again should not start where you left off.
--- @return boolean
function M.abandon(player, id)
    if not M.is_active(player, id) then return false end
    player:clear_quest_flag(flag_active(id))
    if DAEMON and DAEMON.cache then
        pcall(DAEMON.cache.delete, NS, "char:" .. tostring(player.char_id), id)
    end
    return true
end

--- Pay out.
---
--- XP through `award_xp` so an experience buff applies and nothing here has to
--- know it exists; an effect through the `quest:` source scheme, which was
--- documented and had no user.
function M.reward(player, quest)
    local r = quest.reward or {}

    if r.xp and player.award_xp then
        pcall(player.award_xp, player, r.xp, { source = "quest:" .. quest.id })
    end
    if r.gold and player.award_gold then
        pcall(player.award_gold, player, r.gold)
    end
    for _, item_id in ipairs(r.items or {}) do
        pcall(player.add_item, player, item_id)
    end
    if r.effect and DAEMON and DAEMON.effect then
        pcall(DAEMON.effect.apply, player, r.effect, {
            source = "quest:" .. quest.id,
            duration = r.effect_duration,
        })
    end
    -- A skill is a trait now, so teaching one is `set_base` on something the
    -- character does not have. No separate mechanism.
    if r.skill and DAEMON and DAEMON.trait then
        local current = DAEMON.trait.value(player, r.skill)
        pcall(DAEMON.trait.set_base, player, r.skill, current + (r.skill_amount or 1))
        if player.send then
            player:send("{cyan}You have learned something about " .. r.skill .. ".{/}")
        end
    end
end

--- Everything this character is doing or has done.
--- @return table  array of { quest, active, complete, progress, ready }
function M.journal(player)
    local out = {}
    for _, id in ipairs(M.all()) do
        local active, done = M.is_active(player, id), M.is_complete(player, id)
        if active or done then
            out[#out + 1] = {
                quest = M._quests[id],
                active = active,
                complete = done,
                progress = active and M.progress(player, id) or 0,
                ready = active and M.is_ready(player, id) or false,
            }
        end
    end
    return out
end

-- ─── Listeners ───────────────────────────────────────────────────────────────
--
-- The counters advance from *events*, not from the commands that cause them.
-- That is what keeps combat from knowing quests exist: a kill emits `mob.died`
-- and this listens, exactly as the loot handler and the XP award do.

local function on_mob_died(data)
    if type(data) ~= "table" or not data.killer_char_id then return end
    local player = DAEMON.character and DAEMON.character.get(data.killer_char_id)
    if not player then return end

    for _, id in ipairs(M.all()) do
        local quest = M._quests[id]
        if quest.objective.kind == "kill"
            and quest.objective.target == data.template_id
            and M.is_active(player, id) then
            M.advance(player, id, 1)
        end
    end
end

local function on_room_entered(data)
    if type(data) ~= "table" then return end
    local player = DAEMON.character and DAEMON.character.get(data.char_id)
    if not player then return end

    for _, id in ipairs(M.all()) do
        local quest = M._quests[id]
        if (quest.objective.kind == "visit" or quest.objective.kind == "deliver")
            and quest.objective.target == data.room_id
            and M.is_active(player, id) then
            M.advance(player, id, 1)
        end
    end
end

local function on_item_picked_up(data)
    if type(data) ~= "table" then return end
    local player = DAEMON.character and DAEMON.character.get(data.char_id)
    if not player then return end

    for _, id in ipairs(M.all()) do
        local quest = M._quests[id]
        if quest.objective.kind == "collect"
            and quest.objective.target == data.template_id
            and M.is_active(player, id) then
            -- Counted from what they are *holding* rather than incremented,
            -- because an item picked up, dropped and picked up again is one
            -- item. A counter would call it two.
            local have = 0
            for _, entry in ipairs(player.inventory or {}) do
                if type(entry) == "table" and entry.template == quest.objective.target then
                    have = have + 1
                end
            end
            local scope = "char:" .. tostring(player.char_id)
            local before = M.progress(player, id)
            pcall(DAEMON.cache.set, NS, scope, id, math.min(have, quest.objective.count))
            if have > before and player.send then
                player:send("{cyan}" .. quest.name .. ": "
                    .. math.min(have, quest.objective.count) .. " / "
                    .. quest.objective.count .. "{/}")
            end
        end
    end
end

if DAEMON and DAEMON.event then
    local ok, err = pcall(function()
        DAEMON.event.on("mob.died", "quest_d.kill", on_mob_died)
        DAEMON.event.on("room.entered", "quest_d.visit", on_room_entered)
        DAEMON.event.on("item.picked_up", "quest_d.collect", on_item_picked_up)
    end)
    if not ok then log_error("QUEST_D: could not subscribe: " .. tostring(err)) end
end

-- The write-behind namespace. Declared here rather than in the mudlib because
-- quests are content, and a game with no quests should not pay for the
-- namespace.
if DAEMON and DAEMON.cache and DAEMON.cache.define then
    local ok, err = pcall(DAEMON.cache.define, NS, {
        tier = "write_behind",
        flush_seconds = 30,
        -- A counter is worth losing thirty seconds of and is not worth a
        -- database write per kill. That is the whole tier argument in one line.
    })
    if not ok then log_error("QUEST_D: could not declare the namespace: " .. tostring(err)) end
end

log("info", "quest_d loaded")

return M
