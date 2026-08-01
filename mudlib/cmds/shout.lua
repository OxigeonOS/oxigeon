local M = {}
M.name = 'shout'
M.aliases = {}
M.category = 'communication'
M.summary = 'Global broadcast.'
M.permission = nil

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args_str or args_str == "" then
        player:send("Shout what?")
        return
    end

    player:send("You shout: " .. args_str)
    local msg = player.name .. " shouts: " .. args_str

    for _, sid in ipairs(all_sessions()) do
        if sid ~= session_id then
            local p = get_player(sid)
            if p then
                p:send(msg)
            end
        end
    end
end

return M
