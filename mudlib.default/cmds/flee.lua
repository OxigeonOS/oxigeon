local M = {}
M.name = 'flee'
M.aliases = { 'retreat' }
M.category = 'combat'
M.summary = 'Break off from a fight.'
M.permission = nil

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not (DAEMON and DAEMON.combat) then
        player:send("{red}Combat is not available.{/}")
        return
    end

    if not DAEMON.combat.is_fighting(player) then
        player:send("You are not fighting anything.")
        return
    end

    -- Both sides stop: leaving the mob engaged would have it swinging at
    -- someone who has walked away.
    DAEMON.combat.disengage_all(player.char_id)
    player:send("{yellow}You break off the fight.{/}")
    player:message_room(player.name .. " breaks off the fight.")
end

return M
