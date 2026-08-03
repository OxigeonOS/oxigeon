-- mudlib/cmds/help.lua — The command list, generated from the registry.
--
-- This used to be a hardcoded string listing about twenty of the forty-nine
-- commands, and advertising a `stat` command that did not exist. Its own
-- comment asked for this: every command module already sets `name`, `aliases`,
-- `category`, `summary` and `permission`, so the list can be derived rather
-- than maintained — and a list that is derived cannot drift out of date, which
-- is the only interesting property a help file has.
--
--   help                 every command you can use, by category
--   help <command>       what one command does
--   help all             include commands you lack permission for

local Commands = require('lib.commands')

local M = {}

M.name       = "help"
M.aliases    = { "?", "commands" }
M.category   = "general"
M.summary    = "Show available commands, or help on one of them."
M.permission = nil

-- Categories a player is most likely to want first. Anything a game invents is
-- appended in alphabetical order after these, so a new category shows up
-- without editing this file.
local CATEGORY_ORDER = {
    "movement", "information", "communication", "items", "combat",
    "general", "admin",
}

--- Can this session run the command at all? A help list full of things that
--- answer "you don't have permission to do that" is worse than a shorter one.
local function permitted(session_id, mod)
    if not mod.permission then return true end
    if type(has_permission) ~= "function" then return true end
    local ok, allowed = pcall(has_permission, session_id, mod.permission)
    return ok and allowed
end

--- "look, l" — the canonical name first, then its aliases, so the thing you
--- would type in a bug report is the thing that leads.
local function verb_list(mod)
    local parts = { mod.name }
    for _, alias in ipairs(mod.aliases or {}) do
        parts[#parts + 1] = alias
    end
    return table.concat(parts, ", ")
end

--- One command in detail. `usage` and `help` are optional on the module; when
--- neither is set the summary is all there is to say, and saying so beats
--- inventing filler.
local function show_one(player, mod)
    local lines = {
        "{cyan}" .. mod.name .. "{/}" .. (mod.permission and
            ("  {red}(requires " .. mod.permission .. "){/}") or ""),
        "",
        "  " .. (mod.summary or "No summary."),
    }

    if #(mod.aliases or {}) > 0 then
        lines[#lines + 1] = "  {yellow}Aliases:{/} " .. table.concat(mod.aliases, ", ")
    end
    lines[#lines + 1] = "  {yellow}Category:{/} " .. (mod.category or "general")

    if type(mod.usage) == "string" then
        lines[#lines + 1] = ""
        lines[#lines + 1] = "  {yellow}Usage:{/} " .. mod.usage
    elseif type(mod.usage) == "table" then
        lines[#lines + 1] = ""
        lines[#lines + 1] = "  {yellow}Usage:{/}"
        for _, u in ipairs(mod.usage) do lines[#lines + 1] = "    " .. u end
    end

    if type(mod.help) == "string" then
        lines[#lines + 1] = ""
        for line in (mod.help .. "\n"):gmatch("(.-)\n") do
            lines[#lines + 1] = "  " .. line
        end
    end

    player:send_lines(lines)
end

--- The whole list, grouped by category.
local function show_all(player, session_id, include_denied)
    local registry = Commands.registry()

    local by_category, extra = {}, {}
    local known = {}
    for _, name in ipairs(CATEGORY_ORDER) do known[name] = true end

    local total, hidden = 0, 0
    for _, mod in pairs(registry) do
        if type(mod) == "table" and mod.name then
            if include_denied or permitted(session_id, mod) then
                local c = mod.category or "general"
                if not by_category[c] then
                    by_category[c] = {}
                    if not known[c] then extra[#extra + 1] = c end
                end
                table.insert(by_category[c], mod)
                total = total + 1
            else
                hidden = hidden + 1
            end
        end
    end
    table.sort(extra)

    local lines = { "{cyan}Commands{/} (" .. total .. ")", "" }

    local function emit(category)
        local list = by_category[category]
        if not list then return end
        table.sort(list, function(a, b) return a.name < b.name end)

        local heading = category:sub(1, 1):upper() .. category:sub(2)
        lines[#lines + 1] = "{yellow}" .. heading .. "{/}"
        for _, mod in ipairs(list) do
            lines[#lines + 1] = string.format("  %-22s %s",
                verb_list(mod), mod.summary or "")
        end
        lines[#lines + 1] = ""
    end

    for _, category in ipairs(CATEGORY_ORDER) do emit(category) end
    for _, category in ipairs(extra) do emit(category) end

    if hidden > 0 then
        lines[#lines + 1] = string.format(
            "{cyan}%d command(s) hidden — you lack the permission. `help all` lists them.{/}",
            hidden)
    end
    lines[#lines + 1] = "Type {cyan}help <command>{/} for detail on one of them."

    player:send_lines(lines)
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    local topic = args[1] and args[1]:lower()

    if topic == nil then
        show_all(player, session_id, false)
        return
    end

    if topic == "all" then
        show_all(player, session_id, true)
        return
    end

    -- A topic is a command name or one of its aliases. Resolving through the
    -- dispatcher's own alias table means `help l` and `help look` agree with
    -- what typing `l` actually does — two lookups that could disagree would be
    -- a help system that lies.
    local canonical = Commands.resolve(topic)
    local mod = canonical and Commands.registry()[canonical]

    if not mod then
        player:send("{red}No command '" .. topic .. "'.{/} Type {cyan}help{/} for the list.")
        return
    end

    show_one(player, mod)
end

return M
