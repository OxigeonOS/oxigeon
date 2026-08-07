-- mudlib/cmds/ask.lua — `ask <creature> about <topic>`.
--
-- The implementation is `talk`'s: one question — who did they mean, what did
-- they say, what if there is no answer — and two copies of it would drift.

local Talk = require('cmds.talk')

local M = {}
M.name = 'ask'
M.aliases = {}
M.category = 'communication'
M.summary = 'Ask someone about something.'
M.usage = { "ask <creature> about <topic>" }
M.permission = nil

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args_str or args_str == "" then
        player:send("{cyan}Usage: ask <creature> about <topic>{/}")
        return
    end

    local who, topic = args_str:match("^(.-)%s+about%s+(.+)$")
    if not who then
        -- `ask smith ore` — no `about`, which people leave out. The first word
        -- is who, the rest is the topic.
        who, topic = args_str:match("^(%S+)%s+(.+)$")
    end
    if not who then
        player:send("{cyan}Ask whom about what?{/}")
        return
    end

    Talk.speak_to(player, who, topic)
end

return M
