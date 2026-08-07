-- mudlib/cmds/give.lua — Hand something to someone in the room.
--
--   give lantern to alice
--   give lantern alice        (the `to` is optional, because people leave it out)
--
-- Players only. Giving to an NPC is a *game* decision — a quest turn-in, a
-- shopkeeper's appraisal — and belongs in an `on_interact` handler in the game
-- layer rather than in a mudlib verb that would have to guess.

local Carry = require('lib.carry')

local M = {}
M.name = 'give'
M.aliases = {}
M.category = 'items'
M.summary = 'Hand something to someone.'
M.usage = { "give <item> to <player>" }
M.permission = nil

local function split_to(args_str)
    local what, who = args_str:match("^(.-)%s+to%s+(.+)$")
    if what then return what, who end
    -- No `to`: the last word is the recipient, the rest is the item.
    local head, tail = args_str:match("^(.+)%s+(%S+)$")
    return head, tail
end

--- Someone else in this room, by name prefix.
local function find_recipient(player, name)
    if not (DAEMON and DAEMON.world) then return nil end
    local room = DAEMON.world.get_character_room_obj(player.char_id)
    if not room or not room.get_characters then return nil end

    local want = name:lower()
    for _, char_id in ipairs(room:get_characters()) do
        if char_id ~= player.char_id then
            local other = DAEMON.character and DAEMON.character.get(char_id)
            if other and other.name and other.name:lower():find(want, 1, true) == 1 then
                return other
            end
        end
    end
    return nil
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args_str or args_str == "" then
        player:send("{cyan}Usage: give <item> to <player>{/}")
        return
    end
    if not (DAEMON and DAEMON.items) then
        player:send("{red}You cannot give things here.{/}")
        return
    end

    local what, who = split_to(args_str)
    if not what or not who then
        player:send("{cyan}Usage: give <item> to <player>{/}")
        return
    end

    local recipient = find_recipient(player, who)
    if not recipient then
        player:send("{red}" .. who .. " is not here.{/}")
        return
    end

    local entry, item, _, why = Carry.find(player, what, { inventory = true, room = false })
    if not entry then
        player:send(why or ("{red}You are not carrying a " .. what .. ".{/}"))
        return
    end

    local name = item.display_name and item:display_name() or item.short or entry.template
    local ok, why = Carry.give(player, entry, item, recipient)
    if not ok then
        player:send("{red}" .. (why or "They will not take it.") .. "{/}")
        return
    end

    player:send("You give " .. name .. " to " .. recipient.name .. ".")
    if recipient.send then
        pcall(recipient.send, recipient, player.name .. " gives you " .. name .. ".")
    end
    player:message_room(player.name .. " gives " .. name .. " to " .. recipient.name .. ".")
end

return M
