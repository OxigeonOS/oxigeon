-- mudlib/daemons/task_d.lua — Named, inspectable, controllable periodic work.
--
-- A thin layer over `ticker_d`, and the difference is the whole point: a raw
-- ticker is anonymous and fire-and-forget, while a task has an id you can
-- list, pause, resume and run on demand. That is what an operator needs at
-- three in the morning when one periodic job is misbehaving and the rest must
-- keep running.
--
-- `sort`ed listing and a human `label` exist because the `tasks` command is the
-- only window onto this, and a window that reorders itself between two reads is
-- most of the way to useless.

local M = {}

local _tasks = {}

local function log_error(msg)
    log("error", msg)
    if DAEMON and DAEMON.journal then
        local ok = pcall(function() DAEMON.journal.error(msg) end)
    end
end

--- Schedule recurring work under a name.
--- @param config table  { id, interval, func, label?, enabled?, run_now? }
--- @return boolean ok, string|nil why
function M.schedule(config)
    if type(config) ~= "table" then return false, "Invalid task config" end
    local id = config.id
    -- `run` as well as `func`: two callers wrote the field two ways and the
    -- one that guessed wrong got a task that silently never registered.
    local func = config.func or config.run
    if not id or type(func) ~= "function" or not config.interval then
        return false, "Invalid task config"
    end
    config.func = func
    
    M.cancel(id)
    
    local enabled = true
    if config.enabled ~= nil then enabled = config.enabled end
    
    _tasks[id] = {
        id = id,
        -- What it is for, in words, so `tasks` reads as a list of jobs rather
        -- than a list of identifiers.
        label = config.label or id,
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

--- Every scheduled task, in id order.
---
--- Sorted rather than `pairs` order: this is what `tasks` prints, and a list
--- that reshuffles between two reads cannot be compared against itself.
--- @return table  array of { id, label, interval, last_run, run_count, paused }
function M.list()
    local result = {}
    for _, t in pairs(_tasks) do
        table.insert(result, {
            id = t.id,
            label = t.label,
            interval = t.interval,
            last_run = t.last_run,
            run_count = t.run_count,
            paused = t.paused
        })
    end
    table.sort(result, function(a, b) return a.id < b.id end)
    return result
end

--- One task's state, or nil.
--- @param id string
--- @return table|nil
function M.get(id)
    local t = _tasks[id]
    if not t then return nil end
    return {
        id = t.id, label = t.label, interval = t.interval,
        last_run = t.last_run, run_count = t.run_count, paused = t.paused,
    }
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
