-- mudlib/cmds/color.lua — Toggle or check color output
-- Allows players to disable ANSI color codes for screen reader compatibility.
-- When color is off, all {tag} markup is stripped from output instead of
-- being converted to ANSI escape sequences.

local M = {}

M.name       = "color"
M.aliases    = { "colours", "colors" }
M.category   = "settings"
M.summary    = "Toggle color output on/off. Usage: color [on|off]"
M.permission = nil

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args[1] or args[1] == "" then
        -- Show current setting
        local status = (player.color_enabled ~= false) and "{green}on{/}" or "off"
        player:send("Color output is currently " .. status .. ".")
        player:send("Usage: color on | color off")
        return
    end

    local arg = args[1]:lower()
    if arg == "on" or arg == "yes" or arg == "true" or arg == "1" then
        player.color_enabled = true
        player:send("{green}Color output enabled.{/}")
    elseif arg == "off" or arg == "no" or arg == "false" or arg == "0" then
        player.color_enabled = false
        -- This message will be stripped since color is now off
        player:send("Color output disabled. ANSI codes will be stripped from all output.")
        player:send("This setting is saved and will persist across sessions.")
    else
        player:send("Usage: color on | color off")
    end
end

return M
