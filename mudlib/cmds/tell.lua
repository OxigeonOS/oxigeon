local M = {}
M.name = 'tell'
M.aliases = {}
M.category = 'communication'
M.summary = 'Private message between players.'
M.permission = nil

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args or #args < 2 then
        player:send("Tell whom what?")
        return
    end

    local target_name = args[1]
    local _, msg = args_str:match("^(%S+)%s+(.*)$")
    if not msg then msg = table.concat(args, " ", 2) end

    local target_player
    for _, sid in ipairs(all_sessions()) do
        local p = get_player(sid)
        if p and p.name and p.name:lower() == target_name:lower() then
            target_player = p
            break
        end
    end

    if not target_player then
        player:send("No player named '" .. target_name .. "' is online.")
        return
    end

    player:send("You tell " .. target_player.name .. ": " .. msg)
    target_player:send(player.name .. " tells you: " .. msg)
end

return M
