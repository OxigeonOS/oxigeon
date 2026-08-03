local M = {}
M.name = 'score'
M.aliases = { 'sc', 'stats' }
M.category = 'information'
M.summary = 'Show your attributes.'
M.permission = nil

local ORDER = { "vitals", "attributes", "derived", "general" }

--- Show base and effective side by side, so a modified number explains itself.
local function render(trait)
    local line = string.format("  %-14s %5s", trait.label, tostring(trait.value))
    if trait.kind == "gauge" and trait.max then
        line = string.format("  %-14s %5s / %-5s", trait.label,
            tostring(trait.value), tostring(trait.max))
    elseif trait.value ~= trait.base then
        local delta = trait.value - trait.base
        line = line .. string.format("  {yellow}(%s%d from %s){/}",
            delta > 0 and "+" or "", delta,
            trait.kind == "derived" and "effects" or "effects")
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

    -- `category == "stat"` and not everything: once a character knows forty
    -- skills, an unfiltered score is a wall of text. Everything defined before
    -- categories existed defaults to "stat", so this shows exactly what it
    -- always did. `skills` and `traits` name what they show the same way.
    local traits = DAEMON.trait.all(player, "stat")
    if #traits == 0 then
        player:send("{yellow}No attributes are defined in this game.{/}")
        return
    end

    local groups = {}
    for _, trait in ipairs(traits) do
        if not trait.hidden then
            groups[trait.group] = groups[trait.group] or {}
            table.insert(groups[trait.group], trait)
        end
    end

    local lines = { "{cyan}" .. (player.name or "You") .. "{/}" }
    if player.title then lines[#lines + 1] = "  " .. player.title end
    lines[#lines + 1] = ""

    local seen = {}
    local function emit_group(name)
        local list = groups[name]
        if not list or seen[name] then return end
        seen[name] = true
        lines[#lines + 1] = "{yellow}" .. name:sub(1, 1):upper() .. name:sub(2) .. "{/}"
        for _, trait in ipairs(list) do
            lines[#lines + 1] = render(trait)
        end
        lines[#lines + 1] = ""
    end

    for _, name in ipairs(ORDER) do emit_group(name) end
    -- Anything the game invented that is not in the standard order.
    local extra = {}
    for name in pairs(groups) do
        if not seen[name] then extra[#extra + 1] = name end
    end
    table.sort(extra)
    for _, name in ipairs(extra) do emit_group(name) end

    lines[#lines + 1] = string.format("  %-14s %5s", "Gold", tostring(player.gold or 0))
    lines[#lines + 1] = string.format("  %-14s %5s", "Experience", tostring(player.xp or 0))

    if DAEMON.effect then
        local ok, active = pcall(DAEMON.effect.active, player)
        if ok and #active > 0 then
            lines[#lines + 1] = ""
            lines[#lines + 1] = "{green}You are affected by " .. #active
                .. " effect(s) — see `effects`.{/}"
        end
    end

    player:send_lines(lines)
end

return M
