local M = {}
M.name = 'spawn'
M.aliases = {}
M.category = 'admin'
M.summary = 'Create item in your inventory.'
M.permission = 'admin'

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args or #args == 0 then
        player:send("Usage: spawn <item_template_id>")
        return
    end

    local template_id = args[1]
    
    local ok, item = pcall(DAEMON.items.get, template_id)
    if not ok or not item then
        player:send("Unknown item template '" .. template_id .. "'.")
        return
    end
    
    player:add_item(template_id)
    
    local item_name = (type(item) == "table" and (item.short or item.name)) or template_id
    player:send("Spawned " .. item_name .. " into your inventory.")
    
    if DAEMON and DAEMON.audit then
        pcall(DAEMON.audit.log, "cmd.spawn", true, "spawned " .. template_id)
    end
end

return M
