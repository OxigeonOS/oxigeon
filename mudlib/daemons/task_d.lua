local M = {}

local _tasks = {}

local function log_error(msg)
    log("error", msg)
    if DAEMON and DAEMON.journal then
        local ok = pcall(function() DAEMON.journal.error(msg) end)
    end
end

function M.schedule(config)
    local id = config.id
    if not id or type(config.func) ~= "function" or not config.interval then 
        return false, "Invalid task config" 
    end
    
    M.cancel(id)
    
    local enabled = true
    if config.enabled ~= nil then enabled = config.enabled end
    
    _tasks[id] = {
        id = id,
        interval = config.interval,
        func = config.func,
        paused = not enabled,
        last_run = 0,
        run_count = 0
    }
    
    if enabled then
        local ok, err = pcall(function()
            DAEMON.ticker.every(config.interval, "task_" .. id, function()
                local t = _tasks[id]
                if t and not t.paused then
                    local t_ok = pcall(function() t.last_run = time() end)
                    t.run_count = t.run_count + 1
                    local run_ok, run_err = pcall(t.func)
                    if not run_ok then
                        log_error("Task execution error (" .. id .. "): " .. tostring(run_err))
                    end
                end
            end)
        end)
        if not ok then log_error("task_d schedule error: " .. tostring(err)) end
    end
    
    if config.run_now and enabled then
        M.run_now(id)
    end
    
    return true
end

function M.cancel(id)
    if _tasks[id] then
        _tasks[id] = nil
        local ok, err = pcall(function() DAEMON.ticker.remove("task_" .. id) end)
        if not ok then log_error("task_d cancel error: " .. tostring(err)) end
        return true
    end
    return false
end

function M.list()
    local result = {}
    for id, t in pairs(_tasks) do
        table.insert(result, {
            id = t.id,
            interval = t.interval,
            last_run = t.last_run,
            run_count = t.run_count,
            paused = t.paused
        })
    end
    return result
end

function M.run_now(id)
    local t = _tasks[id]
    if t then
        local t_ok = pcall(function() t.last_run = time() end)
        t.run_count = t.run_count + 1
        local ok, err = pcall(t.func)
        if not ok then
            log_error("Task run_now error (" .. id .. "): " .. tostring(err))
        end
        return true
    end
    return false
end

function M.pause(id)
    local t = _tasks[id]
    if t then
        t.paused = true
        return true
    end
    return false
end

function M.resume(id)
    local t = _tasks[id]
    if t then
        t.paused = false
        return true
    end
    return false
end

log("info", "task_d loaded")
return M
