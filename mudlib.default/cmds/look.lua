local M = {}

M.name = 'look'
M.aliases = {'l'}
M.category = 'navigation'
M.summary = 'Look at your surroundings or examine an item.'
M.permission = nil

local Light   = require('lib.light')
local Examine = require('cmds.examine')

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then
        send(session_id, "You are nowhere. This is concerning.\r\n")
        return
    end

    local session = get_session(session_id)
    if not session or not session.character_id then
        player:send("{red}You are nowhere. This is concerning.{/}")
        return
    end

    local char_id = session.character_id
    local room = DAEMON.world.get_character_room_obj(char_id)

    if not room then
        player:send("{red}You are nowhere. This is concerning.{/}")
        return
    end

    -- `Room.light_level` has been a field since rooms existed and nothing read
    -- it. This is what reads it — and it is checked before *everything*,
    -- because "look at the lever" in a pitch-dark room should not work either.
    -- Exits stay listed: you can feel your way to a doorway.
    if not Light.can_see(player, room) then
        player:send("{cyan}" .. Light.DARKNESS .. "{/}")
        local exits = {}
        for dir, exit in pairs(room.exits or {}) do
            if not (type(exit) == "table" and exit.hidden) then exits[#exits + 1] = dir end
        end
        table.sort(exits)
        if #exits > 0 then
            player:send("You can feel your way " .. table.concat(exits, ", ") .. ".")
        end
        return
    end

    -- If no arguments, show the full room
    if not args[1] or args_str == "" then
        -- room:get_appearance returns pre-formatted text with color tags
        player:send_raw(room:get_appearance(session_id))
        return
    end

    -- `look <target>` and `examine <target>` resolve through the same function,
    -- so they cannot disagree about what is in front of you. They used to: this
    -- command knew about scenery and exact player names only, which meant
    -- `look mephit` failed on a creature named in the room description directly
    -- above it, and `look sword` failed on a sword lying on the floor.
    if not Examine.describe_target(player, args_str) then
        player:send("You don't see that here.")
    end
end

return M
