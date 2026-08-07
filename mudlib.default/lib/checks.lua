-- mudlib/lib/checks.lua — Predicates, for effect conditions and quest gates.
--
-- Each function *returns* a predicate rather than being one, so a condition
-- reads as a sentence:
--
--   condition = checks.all(
--       checks.has_level(5),
--       checks.not_(checks.has_item("hooded_lantern")),
--       checks.cooldown_ready("daily_herb"))
--
-- The convention every predicate follows is `-> boolean, reason?`. The reason
-- is the whole point: a gate that refuses without saying why is a bug report,
-- and `all` returns the *first* failure's reason so the refusal names one thing
-- to fix rather than everything that is wrong at once.

local M = {}

function M.has_item(template_id)
    return function(player)
        if player:has_item(template_id) then
            return true
        end
        return false, "You don't have the required item."
    end
end

function M.has_level(min_level)
    return function(player)
        -- Effective level, so a level-boosting effect counts.
        local level = player.trait and player:trait("level")
            or (player.stats and player.stats.level)
        if level and level >= min_level then
            return true
        end
        return false, "You are not a high enough level."
    end
end

function M.has_quest_flag(flag)
    return function(player)
        -- `player.quest_flags`, not object state. Object state is an in-memory
        -- table that is wiped on restart and cleared by area resets — "have I
        -- ever finished this quest" is a forever answer, and it is already in
        -- SAVE_FIELDS. Choose the tier by how much you would mind losing it.
        if player.quest_flags and player.quest_flags[flag] then
            return true
        end
        return false, "You have not completed the required quest."
    end
end

function M.cooldown_ready(what)
    return function(player)
        if not (DAEMON and DAEMON.cooldown) then return true end
        if DAEMON.cooldown.ready(player.char_id, what) then return true end
        local left = math.ceil(DAEMON.cooldown.remaining(player.char_id, what) / 60)
        return false, "You must wait another " .. left .. " minute(s)."
    end
end

function M.has_permission(perm)
    return function(player)
        if has_permission(player.session_id, perm) then
            return true
        end
        return false, "You do not have permission to do that."
    end
end

function M.state_equals(obj_id, key, value)
    return function(player)
        if get_object_state(obj_id, key) == value then
            return true
        end
        return false, "The required state is not met."
    end
end

--- Invert a predicate.
---
--- A predicate library without negation is half a library: "only when you are
--- *not* carrying a light" is as ordinary a condition as its opposite, and
--- without this every author writes their own inline closure for it.
---
--- The reason cannot be inverted — "you don't have the required item" negates
--- to nothing useful — so an optional one is taken instead. Without it the
--- refusal is generic, which is honest rather than misleading.
--- @param check function
--- @param reason string|nil
--- @return function
function M.not_(check, reason)
    return function(player)
        local pass = check(player)
        if pass then
            return false, reason or "That condition must not be met."
        end
        return true
    end
end

function M.all(...)
    local checks = {...}
    return function(player)
        for _, check in ipairs(checks) do
            local pass, reason = check(player)
            if not pass then
                return false, reason
            end
        end
        return true
    end
end

function M.any(...)
    local checks = {...}
    return function(player)
        for _, check in ipairs(checks) do
            local pass = check(player)
            if pass then
                return true
            end
        end
        return false, "None of the required conditions were met."
    end
end

return M
