-- mudlib/cmds/traits.lua — Everything, with the axes shown separately.
--
-- `score` shows `category == "stat"`, `skills` shows `category == "skill"`, and
-- a trait in a category no command names appears nowhere. That is the correct
-- default — a new category should not silently leak into `score` — but it needs
-- somewhere to be findable, and this is it. A mis-categorised trait shows up
-- here and nowhere else, which is the point.
--
--   traits              your own traits
--   traits <name>       another online character's
--   traits defs         every trait the game defines, present or not

local M = {}
M.name = 'traits'
M.aliases = { '@traits' }
M.category = 'admin'
M.summary = 'Inspect traits by kind, category and presence.'
M.permission = "cmd.traits"

local function sets_of(def)
    if type(def.sets) ~= "table" then return "-" end
    local names = {}
    for name in pairs(def.sets) do names[#names + 1] = name end
    if #names == 0 then return "-" end
    table.sort(names)
    return table.concat(names, ",")
end

--- Find an online character by name. Deliberately exact-match rather than
--- prefix: an admin command that guesses which character it meant is a bug
--- waiting for two players whose names share three letters.
local function find_character(name)
    local want = name:lower()
    for _, sid in ipairs(all_sessions()) do
        local s = get_session(sid)
        if s and s.state == "playing" and s.character_id then
            local p = DAEMON.character and DAEMON.character.get(s.character_id)
            if p and p.name and p.name:lower() == want then return p end
        end
    end
    return nil
end

--- The registry, not any entity: what the game *defines*, so a trait nobody
--- holds is still discoverable. This is the answer to "why does nothing show
--- my new trait" when the answer is that its definition failed to seal.
local function show_defs(player)
    local defs = DAEMON.trait.defs()
    local failed = DAEMON.trait.errors()

    local ids = {}
    for id in pairs(defs) do ids[#ids + 1] = id end
    if #ids == 0 then
        player:send("{yellow}No traits are defined in this game.{/}")
        return
    end
    -- By rank, which is evaluation order — the order a dependency problem is
    -- easiest to read in. `seal` gives failed traits a rank after everything
    -- else, so they collect at the bottom.
    table.sort(ids, function(a, b)
        return (defs[a].rank or math.huge) < (defs[b].rank or math.huge)
    end)

    local lines = {
        "{cyan}Defined traits{/} (" .. #ids .. ")",
        string.format("  {yellow}%-18s %-10s %-12s %-12s %-10s{/}",
            "id", "kind", "category", "group", "sets"),
    }
    for _, id in ipairs(ids) do
        local def = defs[id]
        local line = string.format("  %-18s %-10s %-12s %-12s %-10s",
            id, def.kind, def.category, def.group, sets_of(def))
        if def.hidden then line = line .. " {cyan}hidden{/}" end
        if failed[id] then line = line .. " {red}broken: " .. failed[id] .. "{/}" end
        lines[#lines + 1] = line
    end
    player:send_lines(lines)
end

--- One entity's traits, grouped by category rather than by group — because the
--- question this command answers is "what category did that end up in".
local function show_entity(player, target, label)
    local rows = DAEMON.trait.all(target)
    if #rows == 0 then
        player:send("{yellow}" .. label .. " holds no traits.{/}")
        return
    end

    local by_category, categories = {}, {}
    for _, row in ipairs(rows) do
        local c = row.category or "stat"
        if not by_category[c] then
            by_category[c] = {}
            categories[#categories + 1] = c
        end
        table.insert(by_category[c], row)
    end
    table.sort(categories)

    local defs = DAEMON.trait.defs()
    local lines = { "{cyan}" .. label .. "{/} — " .. #rows .. " present, "
        .. #categories .. " categor" .. (#categories == 1 and "y" or "ies"), "" }

    for _, category in ipairs(categories) do
        lines[#lines + 1] = "{yellow}" .. category .. "{/}"
        for _, row in ipairs(by_category[category]) do
            local def = defs[row.id] or {}
            local line = string.format("  %-18s %-10s %-12s base %-7s value %-7s",
                row.id, row.kind, row.group,
                tostring(row.base), tostring(row.value))
            if row.max then line = line .. " max " .. tostring(row.max) end
            if row.hidden then line = line .. " {cyan}hidden{/}" end
            if def.always then line = line .. " {cyan}always{/}" end
            if row.failed then line = line .. " {red}broken: " .. row.failed .. "{/}" end
            lines[#lines + 1] = line
        end
        lines[#lines + 1] = ""
    end

    -- Presence is derived from storage, so the interesting number is how much
    -- of the registry this entity does *not* hold. That gap is the whole point
    -- of sparse traits and is worth being able to see.
    local total = 0
    for _ in pairs(defs) do total = total + 1 end
    lines[#lines + 1] = string.format("{cyan}%d of %d defined traits present.{/}", #rows, total)

    player:send_lines(lines)
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not (DAEMON and DAEMON.trait) then
        player:send("{red}The trait system is not available.{/}")
        return
    end

    local arg = args and args[1]

    if arg == nil then
        show_entity(player, player, player.name or "You")
        return
    end

    if arg:lower() == "defs" or arg:lower() == "all" then
        show_defs(player)
        return
    end

    local target = find_character(arg)
    if not target then
        player:send("{red}No online character named '{yellow}" .. arg .. "{red}'."
            .. " Try `traits defs` for the registry.{/}")
        return
    end
    show_entity(player, target, target.name)
end

return M
