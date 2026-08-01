local M = {}
M.name = 'tasks'
M.aliases = {'@tasks'}
M.category = 'admin'
M.summary = 'Manage background tasks and tickers.'
M.permission = 'admin'

local function format_ago(t)
    if not t or t == 0 then return "never" end
    local diff = time() - t
    if diff < 0 then return "in future" end
    if diff < 60 then return string.format("%ds ago", diff) end
    if diff < 3600 then return string.format("%dm ago", math.floor(diff/60)) end
    return string.format("%dh ago", math.floor(diff/3600))
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not DAEMON.task or not DAEMON.ticker then
        player:send("Task or Ticker daemon not loaded.\r\n")
        return
    end

    if not args_str or args_str == "" then
        local lines = {"Tasks:"}
        local tasks = DAEMON.task.list and DAEMON.task.list() or {}
        for _, t in ipairs(tasks) do
            local status = t.paused and "PAUSED" or "ACTIVE"
            table.insert(lines, string.format("  %-20s | Int: %-5s | Last: %-10s | Runs: %-5s | %s", t.id, tostring(t.interval), format_ago(t.last_run), tostring(t.run_count), status))
        end
        if #tasks == 0 then table.insert(lines, "  (none)") end
        
        table.insert(lines, "")
        table.insert(lines, "System Tickers:")
        local tickers = DAEMON.ticker.list and DAEMON.ticker.list() or {}
        local count = 0
        for _, id in ipairs(tickers) do
            if not id:match("^task_") then
                table.insert(lines, string.format("  %s", id))
                count = count + 1
            end
        end
        if count == 0 then table.insert(lines, "  (none)") end
        
        player:send(table.concat(lines, "\r\n") .. "\r\n")
        return
    end

    local cmd = args[1]
    local id = args[2]
    if not id then
        player:send("Usage: tasks <pause|resume|run|cancel> <id>\r\n")
        return
    end

    if cmd == "pause" then
        if DAEMON.task.pause then DAEMON.task.pause(id) end
        player:send(string.format("Task %s paused.\r\n", id))
    elseif cmd == "resume" then
        if DAEMON.task.resume then DAEMON.task.resume(id) end
        player:send(string.format("Task %s resumed.\r\n", id))
    elseif cmd == "run" then
        if DAEMON.task.run_now then DAEMON.task.run_now(id) end
        player:send(string.format("Task %s triggered.\r\n", id))
    elseif cmd == "cancel" then
        if DAEMON.task.cancel then DAEMON.task.cancel(id) end
        player:send(string.format("Task %s cancelled.\r\n", id))
    else
        player:send("Unknown task command.\r\n")
    end
end

return M
