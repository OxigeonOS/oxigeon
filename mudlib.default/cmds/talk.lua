-- mudlib/cmds/talk.lua — Say something to a creature and get an answer.
--
--   talk smith               the default greeting
--   ask smith about ore      one keyword
--
-- `Mobile.dialogue` and `Mobile:get_dialogue` have existed since the class did
-- and had **no callers**. A dialogue table is `keyword -> response`, where a
-- response is a string or a function returning one — the ordinary lfun pattern,
-- so an NPC can answer differently depending on a quest flag without needing
-- any new mechanism.
--
--   dialogue = {
--       greeting = "The smith looks up. \"Aye?\"",
--       ore      = function(mob, player)
--           if player:has_quest_flag("mine_opened") then return "..." end
--           return "\"Nothing comes out of that mine any more.\""
--       end,
--   }
--
-- `on_interact(mob, char_id, verb)` fires either way, so an NPC can react to
-- being spoken to at all — a shopkeeper greeting you, a guard turning round —
-- without having to enumerate every keyword.

local M = {}
M.name = 'talk'
M.aliases = { 'greet' }
M.category = 'communication'
M.summary = 'Speak to someone.'
M.usage = {
    "talk <creature>              their greeting",
    "ask <creature> about <topic> one subject",
}
M.permission = nil

--- Find a creature here by name prefix — the same match `attack` and `stat`
--- use, so all three agree about which one you meant.
--- @return table|nil
function M.find_here(player, name)
    if not (DAEMON and DAEMON.world and DAEMON.mobs) then return nil end
    local room_id = DAEMON.world.get_character_room(player.char_id)
    if not room_id then return nil end
    local ok, mob, why = pcall(DAEMON.mobs.find_in_room, room_id, name)
    if not ok then return nil, nil end
    return mob, why
end

--- Ask one creature about one topic and say what came back.
---
--- Shared with `ask.lua`, because "who did they mean, what did they say, and
--- what if there is no answer" is one question and two copies of it drift.
--- @param topic string|nil  nil means the default greeting
function M.speak_to(player, name, topic)
    local mob, why = M.find_here(player, name)
    if not mob then
        player:send(why or ("{red}" .. name .. " is not here.{/}"))
        return
    end

    local display = mob.short or mob.name or "They"

    -- The hook fires before the lookup, so an NPC can react to being addressed
    -- even about something it has nothing to say on.
    if type(mob.on_interact) == "function" then
        local ok, err = pcall(mob.on_interact, mob, player.char_id, topic and "ask" or "talk")
        if not ok then
            log("error", "TALK: on_interact for '" .. tostring(mob.id) .. "' raised: "
                .. tostring(err))
        end
    end

    local key = topic and topic:lower() or "greeting"
    -- The asker is passed through, so an lfun answer can be about *them* —
    -- what they are carrying, what they have already done. Without it a
    -- dialogue lfun is a slower way of writing a string.
    local response = mob.get_dialogue and mob:get_dialogue(key, player)

    -- A dialogue table with no `greeting` still answers when greeted, because
    -- an NPC that says nothing at all when spoken to reads as broken rather
    -- than as taciturn.
    if not response and not topic then
        response = mob.get_dialogue and mob:get_dialogue("default", player)
    end

    if not response then
        if topic then
            player:send(display .. " has nothing to say about " .. topic .. ".")
        else
            player:send(display .. " does not respond.")
        end
        return
    end

    player:send(response)
    player:message_room(player.name .. (topic
        and (" asks " .. display .. " about " .. topic .. ".")
        or (" speaks to " .. display .. ".")))
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args_str or args_str == "" then
        player:send("{cyan}Talk to whom?{/}")
        return
    end

    -- `talk smith about ore` is what people type even though `ask` exists.
    local who, topic = args_str:match("^(.-)%s+about%s+(.+)$")
    M.speak_to(player, who or args_str, topic)
end

return M
