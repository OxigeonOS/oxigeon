local M = {}
M.name = 'mudstatus'
M.aliases = {'@mudstatus', 'serverstatus'}
M.category = 'admin'
M.summary = 'Show server status, the Lua heap and loaded daemons.'
M.usage = {
    "mudstatus       status, including heap and GC counters",
    "mudstatus gc    force a full Lua collection and report what it cost",
}
M.permission = "cmd.mudstatus"

local function format_uptime(seconds)
    if not seconds then return "0s" end
    local d = math.floor(seconds / 86400)
    seconds = seconds % 86400
    local h = math.floor(seconds / 3600)
    seconds = seconds % 3600
    local m = math.floor(seconds / 60)
    -- Floored: `uptime_secs` is fractional, and `%d` on a fraction raises
    -- "number has no integer representation" from Lua 5.3 on. Nobody wants
    -- their uptime reported to three decimal places anyway.
    local s = math.floor(seconds % 60)

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
        table.insert(lines, string.format(" Areas:       %d loaded", areas))
        -- On its own line: four area names on the same line as the count blew
        -- past the wrap width and came out ragged, which made the one column
        -- somebody actually reads hard to scan.
        table.insert(lines, string.format("              %s", area_str))
    else
        table.insert(lines, string.format(" Areas:       %d loaded", areas))
    end
    
    table.insert(lines, string.format(" Rooms:       %d loaded", rooms))
    table.insert(lines, string.format(" Tickers:     %d active", tickers))
    table.insert(lines, string.format(" Tasks:       %d scheduled", tasks))
    table.insert(lines, string.format(" Events:      %d with listeners", events))
    table.insert(lines, string.format(" Daemons:     %d loaded", daemon_count))

    -- The Lua heap. This is the whole point of `server_info().lua`: nothing
    -- measured the heap or the GC before, so any claim about retention was an
    -- argument rather than a number. `mudstatus` twice, an hour apart, with a
    -- `mudstatus gc` between, is the heap drill.
    local heap = info.lua
    if type(heap) == "table" then
        table.insert(lines, "")
        local used_mb = (heap.heap_bytes or 0) / 1048576
        local limit_mb = (heap.limit_bytes or 0) / 1048576
        if limit_mb > 0 then
            local pct = (heap.heap_fraction or 0) * 100
            -- Coloured at two thirds, because LuaJIT runs an emergency full
            -- collection before failing and the first symptom is a latency
            -- spike, not an error. By the time allocations raise it is late.
            local colour = pct > 66 and "{red}" or (pct > 40 and "{yellow}" or "{green}")
            table.insert(lines, string.format(" Lua heap:    %s%.1f MB / %.0f MB (%.0f%%){/}",
                colour, used_mb, limit_mb, pct))
        else
            table.insert(lines, string.format(" Lua heap:    %.1f MB (no ceiling)", used_mb))
        end
        if (heap.gc_full_count or 0) > 0 then
            table.insert(lines, string.format(" Full GCs:    %d, %.1f ms total, %.1f MB reclaimed",
                heap.gc_full_count, heap.gc_full_ms or 0,
                (heap.gc_freed_bytes or 0) / 1048576))
        end
    end

    table.insert(lines, "")

    -- `mudstatus gc` runs a full collection and reports what it cost. Kept
    -- behind a subcommand rather than run on every status read: a full cycle
    -- is a stop-the-world pause on the game thread, and a diagnostic that
    -- causes the hitch it is meant to measure is worse than none.
    if args and args[1] and args[1]:lower() == "gc" then
        local ok, result = pcall(gc_collect)
        if ok and type(result) == "table" then
            table.insert(lines, string.format(
                " {cyan}Full collection:{/} %.1f MB reclaimed in %.1f ms, heap now %.1f MB",
                (result.freed_bytes or 0) / 1048576, result.ms or 0,
                (result.heap_bytes or 0) / 1048576))
            table.insert(lines, "")
            pcall(DAEMON.audit.log, "cmd.mudstatus", true, "forced a full Lua collection")
        else
            table.insert(lines, " {red}Collection failed: " .. tostring(result) .. "{/}")
        end
    end

    player:send(table.concat(lines, "\r\n") .. "\r\n")
end

return M
