-- mudlib/cmds/perform.lua — Do the thing.
--
-- The generic verb, in the mudlib, because *having* abilities is a property of
-- the engine and calling them "spells" is a property of one game. `cast` is
-- this game's spell-flavoured alias over the same call.
--
-- Deliberately not `use`: `cmds/use.lua` has meant "use an item" since items
-- existed, and quietly widening it is how a player types `use lantern` and casts
-- something. Deliberately not `cast` either, for the same reason in reverse — a
-- game with no magic still has abilities.

local M = {}

M.name       = "perform"
M.aliases    = { "ability", "perf" }
M.category   = "combat"
M.summary    = "Use an ability."
M.usage      = {
    "perform <ability>",
    "perform <ability> at <target>",
    "perform <ability> <target>",
    "{dim}`abilities` lists what you know.{/}",
}
M.permission = nil

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not (DAEMON and DAEMON.ability) then
        player:send("{red}Abilities are unavailable (ability_d is not loaded).{/}")
        return
    end

    args_str = (args_str or ""):gsub("^%s+", ""):gsub("%s+$", "")
    if args_str == "" then
        player:send_lines(M.usage)
        return
    end

    -- `at` is optional, because people leave it out. Same parse as `cast`, so
    -- the two verbs cannot disagree about what you typed.
    local id, target = args_str:match("^(%S+)%s+at%s+(.+)$")
    if not id then id, target = args_str:match("^(%S+)%s+(.+)$") end
    if not id then id = args_str end

    local ok, why = DAEMON.ability.use(player, id:lower(), { target = target })
    if not ok then
        player:send("{red}" .. (why or "Nothing happens.") .. "{/}")
    end
end

return M
