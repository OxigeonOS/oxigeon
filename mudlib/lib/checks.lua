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
        if player.stats and player.stats.level and player.stats.level >= min_level then
            return true
        end
        return false, "You are not a high enough level."
    end
end

function M.has_quest_flag(flag)
    return function(player)
        if get_object_state("quest." .. tostring(player.char_id), flag) == true then
            return true
        end
        return false, "You have not completed the required quest."
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
