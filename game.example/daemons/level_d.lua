-- game/daemons/level_d.lua — Turning experience into levels.
--
-- `Player:award_xp` accumulated experience and **nothing ever read it**:
-- `level` was a counter that started at 1 and stayed there. Everything gated on
-- it — the quest chain, the silver dagger, three of the four spells — was
-- therefore unreachable by an ordinary player, and `player.levelup` was a
-- documented event with no emitter.
--
-- ─── Why this is content ─────────────────────────────────────────────────────
--
-- The mudlib owns the *mechanism*: `xp` is a Player field, `level` is a trait,
-- `award_xp` runs the `xp_gained` pipeline so a buff composes, and
-- `player.xp_gained` is emitted for anyone who cares. What it does not own is
-- the **curve** — how much a level costs, whether it is linear or quadratic,
-- whether there is a cap, and what happens when you reach one. A game with a
-- twenty-level arc and a game with a two-hundred-level grind want different
-- files, not a configuration option.
--
-- So this listens to `player.xp_gained` and does the rest, exactly as `aggro_d`
-- listens to `room.entered`.
--
-- ─── Why a table and not a formula ───────────────────────────────────────────
--
-- A formula is shorter and a table is *readable*: a designer can see that level
-- 4 costs 450 and decide whether that is the right moment for the mine, without
-- evaluating anything. The demo world is tuned against these numbers — the town
-- quests carry a character to about level 5, and the mine carries them to the
-- boss — so they are a design document as much as data.

local M = {}

--- Total experience needed to *reach* each level. Index N is level N.
---
--- Beyond the end of the table the last gap repeats, so the curve does not
--- stop and nothing has to special-case the top.
M.THRESHOLDS = {
    [1] = 0,
    [2] = 100,
    [3] = 250,
    [4] = 450,
    [5] = 700,
    [6] = 1000,
    [7] = 1400,
    [8] = 1900,
    [9] = 2500,
    [10] = 3200,
    [11] = 4100,
    [12] = 5100,
    [13] = 6200,
    [14] = 7400,
    [15] = 8700,
}

--- The gap repeated past the end of the table.
local TAIL_STEP = 1500

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

--- Total experience needed to reach a level.
--- @param level number
--- @return number
function M.threshold(level)
    if level <= 1 then return 0 end
    local known = M.THRESHOLDS[level]
    if known then return known end

    local top = #M.THRESHOLDS
    -- `#` on a table with an explicit [1] = 0 is the array length, which is
    -- what we want: the highest contiguous index.
    return M.THRESHOLDS[top] + (level - top) * TAIL_STEP
end

--- What level this much experience is worth.
--- @param xp number
--- @return number
function M.level_for(xp)
    xp = tonumber(xp) or 0
    local level = 1
    -- Walks up rather than searching: levels are small numbers and this runs
    -- once per experience award, which is not a hot path.
    while xp >= M.threshold(level + 1) do
        level = level + 1
        if level > 200 then break end   -- a cap on the loop, not on the game
    end
    return level
end

--- Experience into and needed for the current level, for a progress display.
--- @param player table
--- @return number into, number needed
function M.progress(player)
    local level = player:trait("level")
    local base = M.threshold(level)
    local next_at = M.threshold(level + 1)
    return math.max(0, (player.xp or 0) - base), math.max(1, next_at - base)
end

--- Bring a character's level into line with their experience.
---
--- Called after every award, and safe to call at any other time: it computes
--- the level from the experience rather than incrementing, so a character
--- granted experience directly, or loaded from a save written before this
--- daemon existed, lands on the right level rather than one level per login.
--- @param player table
--- @return number|nil  the new level, when it changed
function M.reconcile(player)
    if type(player) ~= "table" or not (DAEMON and DAEMON.trait) then return nil end

    local was = player:trait("level")
    local now = M.level_for(player.xp or 0)
    -- Only ever upward. Losing experience should not take a level back: a
    -- character whose gear and quests assume level 8 becoming level 7 is a
    -- worse outcome than the inconsistency, and death does not cost experience
    -- in this game anyway.
    if now <= was then return nil end

    local ok, err = pcall(DAEMON.trait.set_base, player, "level", now)
    if not ok then
        log_error("LEVEL_D: could not set level for char "
            .. tostring(player.char_id) .. ": " .. tostring(err))
        return nil
    end

    -- `max_hp` and `max_mp` are derived from level, so the ceilings have just
    -- moved. Filling the gauges to the new maximum is this game's decision —
    -- a levelling-up heal — and it is one line because a gauge's bound is an
    -- ordinary trait.
    for _, gauge in ipairs({ "hp", "mp" }) do
        local max_id = "max_" .. gauge
        if DAEMON.trait.get_def(max_id) then
            pcall(DAEMON.trait.set_cur, player, gauge, player:trait(max_id))
        end
    end

    if player.send then
        for level = was + 1, now do
            pcall(player.send, player,
                "{green}You are now level " .. level .. ".{/}")
        end
        pcall(player.send, player, "You feel steadier. ("
            .. player:trait("hp") .. "/" .. player:trait("max_hp") .. ")")
    end
    if player.message_room then
        pcall(player.message_room, player,
            player.name .. " looks suddenly more sure of themselves.")
    end

    if DAEMON.event then
        pcall(DAEMON.event.emit, "player.levelup", {
            char_id = player.char_id,
            from = was,
            new_level = now,
        })
    end

    return now
end

-- ─── Wiring ──────────────────────────────────────────────────────────────────

if DAEMON and DAEMON.event then
    local ok, err = pcall(DAEMON.event.on, "player.xp_gained", "level_d.check",
        function(data)
            if type(data) ~= "table" or not data.char_id then return end
            local player = DAEMON.character and DAEMON.character.get(data.char_id)
            if player then M.reconcile(player) end
        end)
    if not ok then log_error("LEVEL_D: could not subscribe: " .. tostring(err)) end

    -- On login too. A character who gained experience before this daemon
    -- existed — or whose level was somehow left behind — is corrected once,
    -- quietly, rather than staying wrong forever.
    local lok, lerr = pcall(DAEMON.event.on, "player.login", "level_d.catchup",
        function(data)
            if type(data) ~= "table" or not data.char_id then return end
            local player = DAEMON.character and DAEMON.character.get(data.char_id)
            if player then M.reconcile(player) end
        end)
    if not lok then log_error("LEVEL_D: could not subscribe to login: " .. tostring(lerr)) end
else
    log("warn", "LEVEL_D: no event daemon — nobody will ever gain a level")
end

log("info", "level_d loaded")

return M
