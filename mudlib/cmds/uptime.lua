-- mudlib/cmds/uptime.lua — Show how long the server has been running

local M = {}

M.name       = "uptime"
M.aliases    = {}
M.category   = "general"
M.summary    = "Show how long the server has been running."
M.permission = nil

--- Format a duration in seconds into a human-readable string.
local function format_duration(secs)
    secs = math.floor(secs)
    local months  = math.floor(secs / (30 * 86400))
    secs = secs - months * (30 * 86400)
    local days    = math.floor(secs / 86400)
    secs = secs - days * 86400
    local hours   = math.floor(secs / 3600)
    secs = secs - hours * 3600
    local minutes = math.floor(secs / 60)
    local seconds = secs - minutes * 60

    local parts = {}
    if months  > 0 then parts[#parts+1] = months  .. (months  == 1 and " month"  or " months")  end
    if days    > 0 then parts[#parts+1] = days    .. (days    == 1 and " day"    or " days")    end
    if hours   > 0 then parts[#parts+1] = hours   .. (hours   == 1 and " hour"   or " hours")   end
    if minutes > 0 then parts[#parts+1] = minutes .. (minutes == 1 and " minute" or " minutes") end

    -- Always show seconds if under a minute
    if #parts == 0 then
        parts[#parts+1] = seconds .. (seconds == 1 and " second" or " seconds")
    end

    if #parts == 1 then return parts[1] end
    local last = table.remove(parts)
    return table.concat(parts, ", ") .. " and " .. last
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    local info = server_info()
    if not info then
        player:send("{red}Server info unavailable.{/}")
        return
    end

    local duration = format_duration(info.uptime_secs)
    local lines = {}
    table.insert(lines, string.format("{cyan}%s{/} has been running for {yellow}%s{/}.", info.name, duration))
    table.insert(lines, string.format("(Started: {yellow}%s{/})", info.started_at))
    player:send(table.concat(lines, "\r\n"))
end

return M
