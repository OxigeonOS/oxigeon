-- mudlib/cmds/trace.lua — Trace Lua execution for debugging

local M = {}

M.name       = "trace"
M.aliases    = {}
M.category   = "admin"
M.summary    = "Trace Lua execution. Usage: trace <on|off|calls|lines|time|show|timings|status|clear> [all]"
M.permission = "admin"

--- Split the argument list into a subcommand, a numeric argument, and a scope.
--- Factored out so it can be unit-tested without a Player.
--- @param args string[]
--- @return string sub, integer|nil count, string|nil scope
function M.parse_args(args)
    local sub = (args[1] or "status"):lower()
    local count, scope
    for i = 2, #args do
        local a = args[i]:lower()
        if a == "all" then
            scope = "all"
        elseif tonumber(a) then
            count = math.floor(tonumber(a))
        end
    end
    return sub, count, scope
end

local MODES = { on = true, calls = true, lines = true, time = true, off = true }

--- Send a plain-text body, paging it if it is long.
--- The body must not contain {colour} tags: DAEMON.pager.page writes through the
--- raw send() efun and skips Player:_process_output, so tags would show up as-is.
local function send_body(player, session_id, lines)
    if #lines == 0 then
        player:send("{dim}(nothing recorded){/}")
        return
    end
    local body = table.concat(lines, "\r\n")
    if DAEMON and DAEMON.pager and #lines > 20 then
        DAEMON.pager.page(session_id, body, 20)
    else
        player:send_raw(body)
    end
end

local function show_status(player)
    local st = trace_status()
    local out = {}
    table.insert(out, "{cyan}Trace status{/}")
    table.insert(out, "  mode:      {yellow}" .. st.mode .. "{/}")
    table.insert(out, "  hook:      " ..
        (st.armed and "{green}installed{/}" or "{dim}removed (no overhead){/}"))
    if st.all_sessions then
        table.insert(out, "  scope:     {yellow}all sessions{/}")
    else
        table.insert(out, "  scope:     {yellow}" .. #st.sessions .. "{/} session(s)")
    end
    table.insert(out, "  records:   {yellow}" .. st.records .. "{/} / " .. st.capacity)
    table.insert(out, "  timings:   {yellow}" .. st.timings .. "{/}")
    if st.dropped > 0 then
        table.insert(out, "  {dim}dropped " .. st.dropped .. " record(s) — ring full{/}")
    end
    player:send(table.concat(out, "\r\n"))
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    local sub, count, scope = M.parse_args(args)

    if sub == "status" then
        show_status(player)
        return
    end

    if MODES[sub] then
        local mode = (sub == "on") and "calls" or sub
        local ok, err = trace_set(mode, scope)
        if not ok then
            player:send("{red}" .. tostring(err or "failed") .. "{/}")
            return
        end
        if mode == "off" then
            player:send("{green}Tracing off.{/}")
        else
            player:send("{green}Tracing {yellow}" .. mode .. "{green} for " ..
                (scope == "all" and "all sessions" or "this session") .. ".{/}")
            player:send("{dim}Every traced line runs interpreted — turn it off when done.{/}")
        end
        return
    end

    if sub == "show" then
        player:send_raw("{cyan}── Trace ──{/}")
        send_body(player, session_id, trace_show(count or 40))
        return
    end

    if sub == "timings" then
        player:send_raw("{cyan}── Command timings ──{/}")
        send_body(player, session_id, trace_timings(count or 20))
        return
    end

    if sub == "clear" then
        trace_clear()
        player:send("{green}Trace buffers cleared.{/}")
        return
    end

    local usage = {}
    table.insert(usage, "Usage:")
    table.insert(usage, "  trace status            show current settings")
    table.insert(usage, "  trace on|calls [all]    trace function entry/exit")
    table.insert(usage, "  trace lines [all]       trace every executed line (verbose)")
    table.insert(usage, "  trace time [all]        counters only, no per-line records")
    table.insert(usage, "  trace off [all]         stop tracing")
    table.insert(usage, "  trace show [n]          last n trace records (default 40)")
    table.insert(usage, "  trace timings [n]       per-command timings (default 20)")
    table.insert(usage, "  trace clear             empty both buffers")
    player:send(table.concat(usage, "\r\n"))
end

return M
