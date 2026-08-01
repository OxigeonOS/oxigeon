-- mudlib/cmds/save.lua — Manually save your character data

local M = {}
M.name = 'save'
M.aliases = {}
M.category = 'general'
M.summary = 'Save your character data.'
M.permission = nil

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if DAEMON and DAEMON.character then
        local ok, err = pcall(DAEMON.character.save, player.char_id)
        if ok then
            player:send("Character saved.")
        else
            player:send("Save failed: " .. tostring(err))
        end
    else
        player:send("Save system not available.")
    end
end

return M
