-- mudlib/cmds/directions.lua — Every direction you can walk, as commands.
--
-- This was twelve files. Each was the same eleven lines with one string
-- changed, which meant adding a direction was a copy-paste and getting one
-- wrong was invisible until somebody walked into it.
--
-- One file declaring many commands is why `lib/commands.lua` understands a
-- `commands` array: the loader registers each entry the same way it registers a
-- single module, so nothing downstream — `help`, `resolve`, dispatch, the
-- permission check — knows the difference.
--
-- Named `directions.lua` rather than `movement.lua` deliberately: `lib/movement.lua`
-- is the *system* these all call, and two files of the same name one directory
-- apart, one requiring the other, is a grep nobody enjoys.
--
-- The set is deliberately the same as `movement.OPPOSITES`. A direction that
-- can be authored as an exit and cannot be walked is a stair only an admin with
-- `goto` can climb — which is exactly what `up`, `down`, `in` and `out` were.

local movement = require('lib.movement')

-- Direction names, their order and their short forms all come from
-- `lib/movement.lua`. They used to be a private copy here, and `dig.lua` had a
-- third — which is how `docs/src/lua-api/olc.md` came to claim `dig` took its
-- reverse direction "from the same table `movement.lua` uses" when it did not.
--
-- `in` and `out` take no single-letter alias: `i` is `inventory` and has been
-- for as long as MUDs have had one. `u` is `up` — `use` used to claim it too,
-- and which of the two you got depended on the order the filesystem happened to
-- list them in.
local ALIASES = {}
for short, long in pairs(movement.ABBREVIATIONS) do
    ALIASES[long] = ALIASES[long] or {}
    ALIASES[long][#ALIASES[long] + 1] = short
end
for _, list in pairs(ALIASES) do table.sort(list) end

local M = {}

--- The loader registers each of these exactly as it would a single-command file.
M.commands = {}

for _, direction in ipairs(movement.ORDER) do
    local aliases = ALIASES[direction] or {}
    M.commands[#M.commands + 1] = {
        name       = direction,
        aliases    = aliases,
        category   = 'navigation',
        summary    = 'Go ' .. direction .. '.',
        permission = nil,
        -- A closure over `direction` rather than twelve copies of one function.
        execute    = function(session_id, args_str, args)
            movement.move(session_id, direction)
        end,
    }
end

return M
