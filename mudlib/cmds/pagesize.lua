local M = {}
M.name = 'pagesize'
M.aliases = {}
M.category = 'settings'
M.summary = 'Set page length.'
M.permission = nil

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end
    
    player.custom = player.custom or {}

    if not args or #args == 0 then
        local current = player.custom.page_length or 40
        player:send("Your page length is " .. current .. " lines. (0 = disabled)")
        return
    end

    local n = tonumber(args[1])
    if not n then
        player:send("Invalid page length. Usage: pagesize <number>")
        return
    end

    player.custom.page_length = n
    player:send("Page length set to " .. n .. " lines.")
end

return M
