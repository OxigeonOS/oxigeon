-- game/cmds/prompt.lua — Set your custom prompt template
-- Variables: %h (hp), %H (max hp), %m (mp), %M (max mp), %g (gold),
--            %x (xp), %l (level), %r (room), %n (name), %% (literal %)

local M = {}

M.name        = "prompt"
M.aliases     = {}
M.category    = "general"
M.summary     = "Set your custom prompt. Usage: prompt <template>"
M.permission  = nil  -- Available to all players

local HELP_TEXT = table.concat({
    "{cyan}Usage:{/} prompt <template>",
    "       prompt reset      — Reset to default",
    "       prompt            — Show current and help",
    "",
    "{yellow}Variables:{/}",
    "  %h  Current HP        %H  Max HP",
    "  %m  Current MP        %M  Max MP",
    "  %g  Gold              %x  XP",
    "  %l  Level             %n  Your name",
    "  %r  Current room      %%  Literal %",
    "",
    "{yellow}Examples:{/}",
    "  prompt %h/%H hp %m/%M mp >",
    "  prompt [%l] %n %h/%Hhp >",
}, "\r\n")

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not DAEMON or not DAEMON.prompt then
        player:send("{red}Prompt system not available.{/}")
        return
    end

    local session = get_session(session_id)
    if not session or not session.character_id then
        player:send("{red}You must be logged in.{/}")
        return
    end

    local char_id = session.character_id

    -- No args: show current + help
    if not args[1] then
        local current = DAEMON.prompt.get_template(char_id) or "> "
        local lines = {}
        table.insert(lines, "{green}Current prompt:{/} " .. current)
        table.insert(lines, HELP_TEXT)
        player:send(table.concat(lines, "\r\n"))
        return
    end

    -- Reset
    if args[1]:lower() == "reset" then
        DAEMON.prompt.set_template(char_id, nil)
        player:send("{green}Prompt reset to default.{/}")
        return
    end

    -- Set new template (use full args_str to preserve spacing)
    -- Ensure it ends with a space for readability
    local template = args_str
    if not template:match("%s$") then
        template = template .. " "
    end

    DAEMON.prompt.set_template(char_id, template)
    player:send("{green}Prompt set to:{/} " .. template)
end

return M
