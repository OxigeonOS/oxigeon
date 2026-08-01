local M = {}
M.name = 'events'
M.aliases = {'@events'}
M.category = 'admin'
M.summary = 'List events and their listeners.'
M.permission = 'admin'

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not DAEMON.event then
        player:send("Event daemon not loaded.\r\n")
        return
    end

    if not args_str or args_str == "" then
        local events = DAEMON.event.events and DAEMON.event.events() or {}
        local lines = {"Events:"}
        table.sort(events)
        for _, ev in ipairs(events) do
            local count = DAEMON.event.count and DAEMON.event.count(ev) or 0
            table.insert(lines, string.format("  %-30s | Listeners: %d", ev, count))
        end
        if #events == 0 then table.insert(lines, "  (none)") end
        player:send_lines(lines)
        return
    end

    local ev = args_str
    local listeners = DAEMON.event.listeners and DAEMON.event.listeners(ev) or {}
    if #listeners == 0 then
        player:send(string.format("No listeners found for event '%s'.\r\n", ev))
        return
    end

    local lines = {string.format("Listeners for '%s':", ev)}
    for _, l in ipairs(listeners) do
        table.insert(lines, string.format("  %s", tostring(l)))
    end
    player:send_lines(lines)
end

return M
