local M = {}
M.name = 'mudstatus'
M.aliases = {'@mudstatus', 'serverstatus'}
M.category = 'admin'
M.summary = 'Show server status and loaded daemons.'
M.permission = 'admin'

local function format_uptime(seconds)
    if not seconds then return "0s" end
    local d = math.floor(seconds / 86400)
    seconds = seconds % 86400
    local h = math.floor(seconds / 3600)
    seconds = seconds % 3600
    local m = math.floor(seconds / 60)
    local s = seconds % 60
    
    local parts = {}
    if d > 0 then table.insert(parts, string.format("%dd", d)) end
    if h > 0 then table.insert(parts, string.format("%dh", h)) end
    if m > 0 then table.insert(parts, string.format("%dm", m)) end
    table.insert(parts, string.format("%ds", s))
    
    return table.concat(parts, " ")
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    local info = server_info() or {}
    local uptime_str = format_uptime(info.uptime_secs)

    local playing = 0
    local total_sessions = 0
    for _, sid in ipairs(all_sessions()) do
        total_sessions = total_sessions + 1
        local s = get_session(sid)
        if s and s.state == "playing" then
            playing = playing + 1
        end
    end

    local areas = 0
    local area_names = {}
    if DAEMON.world and DAEMON.world.all_area_meta then
        local metas = DAEMON.world.all_area_meta() or {}
        for a, _ in pairs(metas) do
            areas = areas + 1
            table.insert(area_names, a)
        end
    end

    local rooms = 0
    if DAEMON.world and DAEMON.world._rooms then
        for _ in pairs(DAEMON.world._rooms) do
            rooms = rooms + 1
        end
    end

    local tickers = 0
    if DAEMON.ticker and DAEMON.ticker.list then
        tickers = #(DAEMON.ticker.list() or {})
    end

    local tasks = 0
    if DAEMON.task and DAEMON.task.list then
        tasks = #(DAEMON.task.list() or {})
    end

    local events = 0
    if DAEMON.event and DAEMON.event.events then
        events = #(DAEMON.event.events() or {})
    end

    local daemon_count = 0
    if DAEMON then
        for k, v in pairs(DAEMON) do
            daemon_count = daemon_count + 1
        end
    end

    local lines = {}
    table.insert(lines, "═══════════════════════════════════════════")
    table.insert(lines, " Oxigeon MUD — Server Status")
    table.insert(lines, "═══════════════════════════════════════════")
    table.insert(lines, string.format(" Uptime:      %s", uptime_str))
    table.insert(lines, string.format(" Players:     %d online", playing))
    table.insert(lines, string.format(" Connections: %d total sessions", total_sessions))
    
    local area_str = table.concat(area_names, ", ")
    if area_str ~= "" then
        table.insert(lines, string.format(" Areas:       %d loaded (%s)", areas, area_str))
    else
        table.insert(lines, string.format(" Areas:       %d loaded", areas))
    end
    
    table.insert(lines, string.format(" Rooms:       %d loaded", rooms))
    table.insert(lines, string.format(" Tickers:     %d active", tickers))
    table.insert(lines, string.format(" Tasks:       %d scheduled", tasks))
    table.insert(lines, string.format(" Events:      %d with listeners", events))
    table.insert(lines, string.format(" Daemons:     %d loaded", daemon_count))
    table.insert(lines, "")

    player:send(table.concat(lines, "\r\n") .. "\r\n")
end

return M
