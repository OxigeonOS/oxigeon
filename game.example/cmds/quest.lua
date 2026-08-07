-- game/cmds/quest.lua — Take one on, hand one in, give one up.
--
--   quest              what is on offer here
--   quest accept <id>
--   quest complete <id>
--   quest abandon <id>
--
-- Offers come from whoever is standing here, which is why `talk` is how you
-- find them: a quest board would work too, and would make the giver decorative.

local Quest = require('daemons.quest_d')

local M = {}
M.name = 'quest'
M.aliases = { 'q' }
M.category = 'information'
M.summary = 'Accept, hand in or abandon a task.'
M.usage = {
    "quest                  what is on offer here",
    "quest accept <id>",
    "quest complete <id>    hand it in to whoever is here",
    "quest abandon <id>",
}
M.permission = nil

--- Every quest offered by anyone in this room.
--- @return table  array of quests
local function offered_here(player)
    if not (DAEMON and DAEMON.world and DAEMON.mobs) then return {} end
    local room_id = DAEMON.world.get_character_room(player.char_id)
    if not room_id then return {} end

    local out = {}
    for _, mob in ipairs(DAEMON.mobs.in_room(room_id)) do
        for _, quest in ipairs(Quest.offers(mob.template_id, player)) do
            out[#out + 1] = { quest = quest, giver = mob }
        end
    end
    return out
end

--- Is somebody here who gave this quest? Handing a quest in to the wrong person
--- — or to nobody — is the thing that makes a giver more than a label.
local function giver_here(player, quest)
    if not quest.giver then return true end
    if not (DAEMON and DAEMON.world and DAEMON.mobs) then return false end
    local room_id = DAEMON.world.get_character_room(player.char_id)
    for _, mob in ipairs(DAEMON.mobs.in_room(room_id or "")) do
        if mob.template_id == quest.giver then return true end
    end
    return false
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    local verb = (args[1] or ""):lower()

    if verb == "" then
        local offers = offered_here(player)
        if #offers == 0 then
            player:send("{yellow}Nobody here has anything for you.{/}")
            return
        end
        local lines = { "{cyan}On offer here{/}", "" }
        for _, o in ipairs(offers) do
            lines[#lines + 1] = "  {yellow}" .. o.quest.id .. "{/}  "
                .. o.quest.name .. "  (level " .. o.quest.level .. ")"
            lines[#lines + 1] = "      " .. o.quest.summary
            lines[#lines + 1] = "      — " .. (o.giver.short or o.giver.name)
        end
        lines[#lines + 1] = ""
        lines[#lines + 1] = "Take one with {cyan}quest accept <id>{/}."
        player:send_lines(lines)
        return
    end

    local id = args[2]
    if not id then
        player:send("{cyan}Which task? Try `quest` to see what is on offer.{/}")
        return
    end

    if verb == "accept" then
        local quest = Quest.get(id)
        if not quest then
            player:send("{red}There is no task called '" .. id .. "'.{/}")
            return
        end
        if not giver_here(player, quest) then
            player:send("{red}You would have to ask them yourself.{/}")
            return
        end
        local ok, why = Quest.accept(player, id)
        if not ok then player:send("{red}" .. (why or "Not that one.") .. "{/}") end
        return
    end

    if verb == "complete" or verb == "hand" or verb == "turnin" then
        local quest = Quest.get(id)
        if not quest then
            player:send("{red}There is no task called '" .. id .. "'.{/}")
            return
        end
        if not giver_here(player, quest) then
            player:send("{red}You would have to take it back to them.{/}")
            return
        end
        local ok, why = Quest.complete(player, id)
        if not ok then player:send("{red}" .. (why or "Not yet.") .. "{/}") end
        return
    end

    if verb == "abandon" or verb == "drop" then
        if Quest.abandon(player, id) then
            player:send("{yellow}Given up: " .. id .. ".{/}")
            player:send("Your progress on it is gone. Taking it again starts over.")
        else
            player:send("{red}You are not doing that.{/}")
        end
        return
    end

    player:send("{red}Unknown option '" .. verb .. "'.{/}")
    player:send_lines(M.usage)
end

return M
