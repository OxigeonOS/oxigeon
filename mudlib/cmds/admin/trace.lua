-- mudlib/cmds/trace.lua — Trace Lua execution for debugging

local M = {}

M.name       = "trace"
M.aliases    = {}
M.category   = "admin"
M.summary    = "Trace Lua execution. Usage: trace <on|off|calls|lines|time|show|timings|status|clear|freeze> [all]"
M.permission = "cmd.trace"

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
---
--- Colour tags are safe here now. They were not: `DAEMON.pager.page` writes
--- through the raw `send` efun and skips `Player:_process_output`, so a tag in a
--- paged body reached the client unrendered — and this function carried a
--- warning telling callers not to use colour, which is the wrong end to fix it.
--- `Player:send_paged` colourises to the player's own preference first.
local function send_body(player, session_id, lines)
    if #lines == 0 then
        player:send("{dim}(nothing recorded){/}")
        return
    end
    player:send_paged(table.concat(lines, "\r\n"), { page_length = 20 })
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
    -- What a breakpoint will do to everyone else. Worth saying before you set
    -- one on a server with people on it, not after.
    if st.stop_the_world then
        table.insert(out, "  breaks:    {yellow}freeze the whole game{/}")
    else
        table.insert(out, "  breaks:    {yellow}suspend one dispatch{/} " ..
            "{dim}(other players keep playing){/}")
    end
    if (st.suspended or 0) > 0 then
        table.insert(out, "  suspended: {yellow}" .. st.suspended ..
            "{/} dispatch(es) waiting at a breakpoint")
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

    if sub == "freeze" then
        -- Read `args` rather than `parse_args`, which only knows about `all` and
        -- a count and would drop `on`/`off` on the floor.
        local want = args[2] and args[2]:lower() or nil
        if want == nil then
            player:send("Breakpoints currently " ..
                (trace_freeze() and "{yellow}freeze the whole game{/}."
                                or "{yellow}suspend one dispatch{/}."))
            return
        end
        if want ~= "on" and want ~= "off" then
            player:send("{red}Usage: trace freeze on|off{/}")
            return
        end
        local ok, err = pcall(trace_freeze, want == "on")
        if not ok then
            player:send("{red}" .. tostring(err) .. "{/}")
            return
        end
        if want == "on" then
            player:send("{yellow}Breakpoints now freeze the whole game.{/} " ..
                "Every player stops until you continue.")
        else
            player:send("{yellow}Breakpoints now suspend only the dispatch that hit " ..
                "them.{/} Everyone else keeps playing.")
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
    table.insert(usage, "  trace freeze on|off     whether a breakpoint stops the whole game")
    player:send(table.concat(usage, "\r\n"))
end

return M
