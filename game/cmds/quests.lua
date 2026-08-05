-- game/cmds/quests.lua — What you are doing and what you have done.

local Quest = require('daemons.quest_d')

local M = {}
M.name = 'quests'
-- Not `journal`: that is a canonical command of its own, and `lazy_load` checks
-- the registry before the alias table, so this one never resolved to anything.
M.aliases = { 'qlog', 'qq' }
M.category = 'information'
M.summary = 'List the tasks you have taken on.'
M.permission = nil

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    local entries = Quest.journal(player)
    if #entries == 0 then
        player:send("{yellow}You are not doing anything in particular.{/}")
        player:send("Ask around — try {cyan}talk <someone>{/}.")
        return
    end

    local active, done = {}, {}
    for _, e in ipairs(entries) do
        table.insert(e.active and active or done, e)
    end

    local lines = {}

    if #active > 0 then
        lines[#lines + 1] = "{cyan}In progress{/}"
        for _, e in ipairs(active) do
            local o = e.quest.objective
            local mark = e.ready and "{green}[ready]{/}" or
                string.format("{yellow}[%d/%d]{/}", e.progress, o.count)
            lines[#lines + 1] = string.format("  %-34s %s", e.quest.name, mark)
            lines[#lines + 1] = "      " .. e.quest.summary
        end
        lines[#lines + 1] = ""
    end

    if #done > 0 then
        lines[#lines + 1] = "{cyan}Finished{/}"
        for _, e in ipairs(done) do
            -- A repeatable one that is off cooldown is worth saying so about,
            -- because "finished" and "finished and available again" are
            -- different states and a journal that conflates them is a journal
            -- nobody checks.
            local again = ""
            if e.quest.repeatable then
                local ok = Quest.can_accept(player, e.quest.id)
                again = ok and "  {green}(available again){/}" or "  {cyan}(repeatable){/}"
            end
            lines[#lines + 1] = "  " .. e.quest.name .. again
        end
    end

    player:send_lines(lines)
end

return M
