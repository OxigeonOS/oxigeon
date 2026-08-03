-- mudlib/cmds/emote.lua — Say what you are doing rather than what you are
-- saying.
--
--   emote grins.            ->  Alice grins.
--   :grins.                 ->  the same, and the spelling everyone uses
--   emote's hand shakes.    ->  Alice's hand shakes.
--
-- The apostrophe form is the one convention worth honouring: a leading `'s`
-- attaches directly to the name, because "Alice 's hand shakes" is wrong in a
-- way people notice immediately.

local messaging = require('lib.messaging')

local M = {}
M.name = 'emote'
M.aliases = { ':', 'me' }
M.category = 'communication'
M.summary = 'Describe what you are doing.'
M.usage = {
    "emote <action>     Alice <action>",
    ": <action>         the same",
}
M.permission = nil

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    local action = (args_str or ""):gsub("^%s+", ""):gsub("%s+$", "")
    if action == "" then
        player:send("{cyan}Emote what? Try `emote grins.`{/}")
        return
    end

    -- A possessive or a punctuation mark joins the name directly; anything else
    -- gets a space.
    local joiner = action:sub(1, 1):match("^['’,.!?;:]") and "" or " "
    local line = player.name .. joiner .. action

    player:send("{cyan}" .. line .. "{/}")

    if DAEMON and DAEMON.world then
        local room_id = DAEMON.world.get_character_room(player.char_id)
        if room_id then
            messaging.send_to_room(room_id, line, player.char_id)
        end
    end

    -- Audited like `say` is not — an emote is not privileged. But it *is*
    -- worth an event, so a game can have an NPC react to one.
    if DAEMON and DAEMON.event then
        pcall(DAEMON.event.emit, "player.emoted", {
            char_id = player.char_id, text = action,
        })
    end
end

return M
