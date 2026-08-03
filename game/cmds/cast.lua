-- game/cmds/cast.lua — Cast a spell.
--
--   cast              what you can cast
--   cast mend
--   cast emberlance at lurker

local Spell = require('daemons.spell_d')

local M = {}
M.name = 'cast'
M.aliases = { 'c' }
M.category = 'combat'
M.summary = 'Cast a spell.'
M.usage = {
    "cast                     what you know",
    "cast <spell>",
    "cast <spell> at <target>",
}
M.permission = nil

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not args_str or args_str == "" then
        local known = Spell.known(player)
        if #known == 0 then
            player:send("{yellow}You know nothing worth saying out loud.{/}")
            return
        end
        local lines = { "{cyan}You know:{/}", "" }
        for _, spell in ipairs(known) do
            local ready = ""
            if spell.cooldown > 0 and DAEMON.cooldown
                and not DAEMON.cooldown.ready(player.char_id, "spell." .. spell.id) then
                ready = string.format("  {red}(%ds){/}",
                    math.ceil(DAEMON.cooldown.remaining(player.char_id, "spell." .. spell.id)))
            end
            lines[#lines + 1] = string.format("  %-14s %3d mana  %s%s",
                spell.id, spell.cost, spell.summary, ready)
        end
        lines[#lines + 1] = ""
        lines[#lines + 1] = "Mana: {cyan}" .. player:trait("mp") .. "{/} / "
            .. player:trait("max_mp") .. "  Spell power: {cyan}"
            .. player:trait("spell_power") .. "{/}"
        player:send_lines(lines)
        return
    end

    -- `at` is optional, because people leave it out.
    local id, target = args_str:match("^(%S+)%s+at%s+(.+)$")
    if not id then id, target = args_str:match("^(%S+)%s+(.+)$") end
    if not id then id = args_str end

    local ok, why = Spell.cast(player, id:lower(), target)
    if not ok then
        player:send("{red}" .. (why or "Nothing happens.") .. "{/}")
    end
end

return M
