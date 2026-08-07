local M = {}
M.name = 'attack'
M.aliases = { 'kill', 'k' }
M.category = 'combat'
M.summary = 'Attack a creature.'
M.permission = nil

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not (DAEMON and DAEMON.combat and DAEMON.mobs and DAEMON.world) then
        player:send("{red}Combat is not available.{/}")
        return
    end

    if not args or #args == 0 then
        player:send("Attack what?")
        return
    end

    if not player:is_alive() then
        player:send("{red}You are in no condition to fight.{/}")
        return
    end

    local room_id = DAEMON.world.get_character_room(player.char_id)
    if not room_id then
        player:send("{red}You are nowhere. This is concerning.{/}")
        return
    end

    -- `why` is the disambiguation list when several creatures answer to that
    -- word. Preferred over this command's own wording because "you do not see
    -- rat here" is actively wrong when there are three of them.
    local target, why = DAEMON.mobs.find_in_room(room_id, args_str)
    if not target then
        player:send(why or ("You do not see " .. args_str .. " here."))
        return
    end

    if DAEMON.combat.target_of(player) == target then
        player:send("You are already fighting that.")
        return
    end

    local ok, reason = DAEMON.combat.engage(player, target)
    if not ok then
        player:send("{red}" .. tostring(reason or "You cannot attack that.") .. "{/}")
        return
    end

    local name = target.name or target.short or "it"
    player:send("{yellow}You attack " .. name .. "!{/}")
    player:message_room(player.name .. " attacks " .. name .. "!")

    -- Swing straight away rather than making the player wait out a round for
    -- something they just chose to do.
    local round_ok, err = pcall(DAEMON.combat.round)
    if not round_ok then
        log("error", "attack: first round failed: " .. tostring(err))
    end
end

return M
