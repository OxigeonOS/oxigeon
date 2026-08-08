-- mudlib/cmds/abilities.lua — What you can do, and why.
--
-- `skills` lists traits in the `skill` category and `score` lists the `stat`
-- ones; this is the same shape for abilities, grouped by `category` — which is a
-- freeform lens that never changes behaviour, exactly as it is on a trait.
--
-- The column worth having is the last one: **where it came from**. An ability
-- you have because you are wielding something behaves identically to one you
-- learned, right up to the moment you take the sword off, and nothing else in
-- the game would tell you which is which.

local Abilities = require('lib.abilities')

local M = {}

M.name       = "abilities"
M.aliases    = { "abils" }
-- `information`, spelled out: `info` is not a category anything else uses, so
-- this command fell into `help`'s alphabetical overflow and printed under its
-- own heading below the admin block. The same typo `navigation` was fixed for.
M.category   = "information"
M.summary    = "Everything you can do, with its cost and whether it is ready."
M.usage      = {
    "abilities            everything",
    "abilities <category> just one kind — spell, technique",
}
M.permission = nil

--- "8 mana", "10 stamina, 4 mana", or "-".
local function cost_of(spec)
    local parts = {}
    for _, c in ipairs(spec.cost or {}) do
        local amount = c.amount
        if type(amount) == "table" then
            amount = tostring(amount.min or "?") .. "+"
        elseif type(amount) == "function" then
            amount = "*"
        end
        local def = DAEMON.trait and DAEMON.trait.get_def(c.trait)
        parts[#parts + 1] = tostring(amount) .. " "
            .. tostring((def and def.label or c.trait)):lower()
    end
    if #parts == 0 then return "{dim}free{/}" end
    return table.concat(parts, ", ")
end

--- "ready", "(6s)", or why you cannot.
local function state_of(player, entry)
    local spec = entry.spec
    if entry.rank < spec.min_rank then return "{dim}rank " .. spec.min_rank .. "{/}" end

    if DAEMON.cooldown and (spec.cooldown.seconds or 0) > 0 then
        local left = DAEMON.cooldown.remaining(player, Abilities.cooldown_key(spec))
        if left > 0 then return "{red}(" .. math.ceil(left) .. "s){/}" end
    end
    return "{green}ready{/}"
end

--- Where it came from, in the words that mean something to a player.
local function source_of(entry)
    local out = {}
    for _, s in ipairs(entry.sources) do
        if s == "open" then
            out[#out + 1] = "known"
        elseif s:match("^trait:") then
            out[#out + 1] = "learned"
        elseif s:match("^equip:") then
            out[#out + 1] = "from what you are wearing"
        else
            out[#out + 1] = s
        end
    end
    return table.concat(out, ", ")
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not (DAEMON and DAEMON.ability) then
        player:send("{red}Abilities are unavailable (ability_d is not loaded).{/}")
        return
    end

    local filter = (args_str or ""):gsub("^%s+", ""):gsub("%s+$", ""):lower()
    local known = DAEMON.ability.known(player, filter ~= "" and filter or nil)

    if #known == 0 then
        player:send(filter ~= ""
            and ("{yellow}You know no " .. filter .. ".{/}")
            or "{yellow}You can do nothing worth listing.{/}")
        return
    end

    -- Grouped by category, categories in a stable order. Presentation only —
    -- `category` decides which list a thing appears in and nothing else.
    local groups, order = {}, {}
    for _, entry in ipairs(known) do
        local cat = entry.spec.category
        if not groups[cat] then groups[cat] = {}; order[#order + 1] = cat end
        table.insert(groups[cat], entry)
    end
    table.sort(order)

    local lines = {}
    for _, cat in ipairs(order) do
        lines[#lines + 1] = "{cyan}" .. cat .. "{/}"
        for _, entry in ipairs(groups[cat]) do
            local rank = entry.spec.rank_trait and (" {dim}r" .. entry.rank .. "{/}") or ""
            lines[#lines + 1] = string.format("  %-14s %-18s %-10s %s%s",
                entry.id, cost_of(entry.spec), state_of(player, entry),
                entry.spec.summary, rank)
            local source = source_of(entry)
            if source ~= "" and source ~= "known" then
                lines[#lines + 1] = "  {dim}" .. string.rep(" ", 14) .. source .. "{/}"
            end
        end
        lines[#lines + 1] = ""
    end
    lines[#lines + 1] = "{dim}`perform <id> at <target>` to use one.{/}"

    player:send_paged(table.concat(lines, "\r\n"))
end

return M
