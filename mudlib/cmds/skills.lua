-- mudlib/cmds/skills.lua — What this character has learned.
--
-- The counterpart to `score`, over `category == "skill"` rather than
-- `category == "stat"`. A skill is a trait the character happens to hold; not
-- having swordsmanship is the point of sparseness, so this list is short at
-- character creation and grows as things are learned.

local Strings = require('lib.strings')

local M = {}
M.name = 'skills'
M.aliases = { 'skill', 'sk' }
M.category = 'information'
M.summary = 'List the skills you have learned.'
M.permission = nil

--- A skill's line. Bounded skills get a bar-style `n / max`, unbounded ones
--- just the number; a modified value explains the difference the way `score`
--- does, because the reason a number moved is the useful part.
local function render(trait)
    local line
    if trait.max then
        line = string.format("  %-18s %5s / %-5s", trait.label,
            Strings.number(trait.value), Strings.number(trait.max))
    else
        line = string.format("  %-18s %5s", trait.label, Strings.number(trait.value))
    end
    if trait.value ~= trait.base then
        -- `%s`, not `%d` — see score.lua: a `round = "none"` trait can carry a
        -- fraction, and `%d` on one raises from 5.3 on.
        local delta = trait.value - trait.base
        line = line .. string.format("  {yellow}(%s%s from effects){/}",
            delta > 0 and "+" or "", Strings.number(delta))
    elseif trait.kind == "derived" then
        line = line .. "  {cyan}(derived){/}"
    end
    if trait.failed then
        line = line .. "  {red}(broken: " .. tostring(trait.failed) .. "){/}"
    end
    return line
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not (DAEMON and DAEMON.trait) then
        player:send("{red}The trait system is not available.{/}")
        return
    end

    local skills = DAEMON.trait.all(player, "skill")

    local visible = {}
    for _, trait in ipairs(skills) do
        if not trait.hidden then visible[#visible + 1] = trait end
    end

    if #visible == 0 then
        player:send("{yellow}You have not learned any skills yet.{/}")
        return
    end

    -- `group` sorts within the command — "weapon", "craft", "magic". It is
    -- presentational and always has been; `category` is what decided this
    -- command shows these traits at all.
    local groups, names = {}, {}
    for _, trait in ipairs(visible) do
        local g = trait.group or "general"
        if not groups[g] then
            groups[g] = {}
            names[#names + 1] = g
        end
        table.insert(groups[g], trait)
    end
    table.sort(names)

    local lines = { "{cyan}Skills{/}", "" }
    for _, name in ipairs(names) do
        lines[#lines + 1] = "{yellow}" .. name:sub(1, 1):upper() .. name:sub(2) .. "{/}"
        for _, trait in ipairs(groups[name]) do
            lines[#lines + 1] = render(trait)
        end
        lines[#lines + 1] = ""
    end

    player:send_lines(lines)
end

return M
