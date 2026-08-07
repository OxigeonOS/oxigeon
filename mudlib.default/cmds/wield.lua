-- mudlib/cmds/wield.lua — Take a weapon in hand.
--
-- The same operation as `wear` with a different word and a different refusal;
-- see `wear.lua`. A two-handed weapon clears the offhand on the way in, and
-- putting something in the offhand clears a two-handed weapon on the way out —
-- otherwise a shield keeps working while both hands are on a greatsword.

-- The shared implementation lives in `wear`, which is the command that has to
-- handle `all` as well. Requiring a sibling command module is unusual and is
-- the point: these two are one verb wearing two words, and a copy of the
-- refusal logic here is a copy that would drift.
local Wear = require('cmds.wear')

local M = {}
M.name = 'wield'
M.aliases = { 'hold' }
M.category = 'items'
M.summary = 'Take a weapon in hand.'
M.usage = { "wield <weapon>" }
M.permission = nil

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end
    if not (DAEMON and DAEMON.items) then
        player:send("{red}You cannot equip anything here.{/}")
        return
    end
    Wear.perform(player, args_str, "wield", "weapon")
end

return M
