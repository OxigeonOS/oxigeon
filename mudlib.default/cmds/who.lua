-- mudlib/cmds/who.lua — List connected players

local M = {}

M.name       = "who"
M.aliases    = {}
M.category   = "general"
M.summary    = "Show who is currently connected."
M.permission = nil

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    local sessions = all_sessions()
    local playing = {}
    local total = #sessions

    for _, sid in ipairs(sessions) do
        local s = get_session(sid)
        if s and s.state == "playing" and s.character_id then
            local char = get_character(s.character_id)
            if char then
                playing[#playing + 1] = char.name
            end
        end
    end

    local lines = {}
    if #playing == 0 then
        table.insert(lines, "No players are currently in the game.")
    else
        table.insert(lines, "{cyan}Players in the game{/} ({yellow}" .. #playing .. "{/}):")
        for _, name in ipairs(playing) do
            table.insert(lines, "  " .. name)
        end
    end
    table.insert(lines, "{cyan}Total connections:{/} {yellow}" .. total .. "{/}")

    player:send(table.concat(lines, "\r\n"))
end

return M
