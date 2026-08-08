-- mudlib/cmds/help.lua — Two levels: the categories, then what is in one.
--
-- This was one flat list of every command you could run. It was derived rather
-- than hand-maintained, which is the only interesting property a help file has,
-- and it stays derived — but a single list had two problems a derivation cannot
-- fix. It ran to about fifty lines in one un-paged burst, so the top scrolled
-- away; and the topic space was exactly the set of registered verbs, so there
-- was no way to write a page about *stances* or *how death works*. A game's
-- help could only ever be a list of its verbs.
--
--   help                      the categories
--   help <category>           the commands and topics in one
--   help <command>            one command in detail
--   help <topic>              an authored page from game/docs/
--   help <category>/<topic>   when two categories hold the same topic name
--   help all                  every command, including ones you cannot use
--
-- Categories come from two places and merge by name. A command declares its own
-- with `M.category`; a game contributes one by making a directory under
-- `game/docs/`, and the files in it are that category's topics. So
-- `game/docs/combat/stances.md` puts a `stances` topic beside `attack` and
-- `flee`, and the player is never shown which half came from where.

local Commands = require('lib.commands')
local Markdown = require('lib.markdown')
local strings  = require('lib.strings')

local M = {}

M.name       = "help"
M.aliases    = { "?", "commands" }
M.category   = "general"
M.summary    = "Show help by category, on one command, or on a topic."
M.permission = nil
M.usage      = {
    "help                      the categories",
    "help <category>           what is in one",
    "help <command>            one command in detail",
    "help <topic>              an authored help page",
    "help <category>/<topic>   when the topic name is ambiguous",
    "help all                  every command, including ones you cannot use",
}

--- The game layer's documentation tree, and the reason the prefix is spelled.
---
--- `list_dir("docs")` unprefixed searches **both roots**, game first, the same
--- way `require` does — so it would sweep up a creator's `mudlib/docs/`, which
--- is the system layer's own documentation and a different thing entirely. The
--- prefix is not decoration; it is the whole rule. Same for the `read_file`
--- that opens a topic.
local DOC_ROOT = "game:docs"

--- The same tree as `permissions.toml` and the `ls`/`cd` shell spell it, for
--- asking `dir_permission` about it. Two spellings of one path because the two
--- APIs disagree, not because they are two paths.
local DOC_VPATH = "/game/docs"

-- Categories a player is most likely to want first. Anything a game invents is
-- appended in alphabetical order after these, so a new category — from a
-- command or from a docs directory — shows up without editing this file.
--
-- `navigation`, not `movement`: this said `movement`, which no command has ever
-- used, so the thirteen commands a new player needs first fell into the
-- alphabetical overflow and printed *after* the admin block.
local CATEGORY_ORDER = {
    "navigation", "information", "communication", "items", "combat",
    "settings", "general", "building", "admin",
}

--- Both places, because an admin reading the console and an admin reading
--- `journal` are looking for the same failure in two different windows.
local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then
        pcall(DAEMON.journal.error, message)
    end
end

-- ─── Permission ──────────────────────────────────────────────────────────────

--- Can this session run the command at all? A help list full of things that
--- answer "you don't have permission to do that" is worse than a shorter one.
local function permitted(session_id, mod)
    if not mod.permission then return true end
    if type(has_permission) ~= "function" then return true end
    local ok, allowed = pcall(has_permission, session_id, mod.permission)
    return ok and allowed
end

--- Can this session read that directory? Nothing in the shipped
--- `permissions.toml` gates `/game/docs`, and `read_file`/`list_dir` are
--- ungated efuns, so on a default install this is always true and help needs no
--- permission at all — which is correct for a help system. A creator who adds a
--- `/game/docs/staff` rule gets it hidden rather than listed-and-then-refused.
local function readable(session_id, vpath)
    if type(dir_permission) ~= "function" then return true end
    local ok, needed = pcall(dir_permission, vpath, "read")
    if not ok or not needed then return true end
    if type(has_permission) ~= "function" then return true end
    local allowed
    ok, allowed = pcall(has_permission, session_id, needed)
    return ok and allowed
end

-- ─── Discovery ───────────────────────────────────────────────────────────────

--- Warned once per boot, not once per `help`. A creator who leaves a README in
--- `game/docs/` should hear about it; every player who types `help` should not
--- put another copy of it in the journal.
local warned_loose = false

--- The topics in one docs category, sorted.
--- Each is `{ name, file, markdown }` — `name` is what a player types
--- (lowercased, `.md` removed), `file` is what is actually on disk.
local function topics_in(dir)
    local ok, entries = pcall(list_dir, DOC_ROOT .. "/" .. dir)
    if not ok or type(entries) ~= "table" then return {} end

    local out = {}
    for _, e in ipairs(entries) do
        -- Only files, and nothing hidden: a `.gitkeep` is not a help topic.
        if type(e) == "table" and e.name and not e.is_dir
            and e.name:sub(1, 1) ~= "." then
            local base = e.name:match("^(.+)%.md$")
            out[#out + 1] = {
                name     = (base or e.name):lower(),
                file     = e.name,
                markdown = base ~= nil,
            }
        end
    end
    table.sort(out, function(a, b) return a.name < b.name end)
    return out
end

--- Every docs category this session may read: `{ [name] = { dir, topics } }`.
---
--- One `list_dir` for the tree and one per category. No cache: that is two
--- syscalls for a command a player types by hand, and a cache would have to be
--- invalidated on `reload` and on every file a builder writes — more machinery
--- than the thing it saves.
local function doc_categories(session_id)
    local out = {}
    if type(list_dir) ~= "function" then return out end

    local ok, entries = pcall(list_dir, DOC_ROOT)
    if not ok or type(entries) ~= "table" then return out end

    local loose = 0
    for _, e in ipairs(entries) do
        if type(e) == "table" and e.name then
            if e.is_dir then
                if e.name:sub(1, 1) ~= "."
                    and readable(session_id, DOC_VPATH .. "/" .. e.name) then
                    local topics = topics_in(e.name)
                    -- An empty directory is not a category. Listing one would
                    -- promise a page and then show nothing.
                    if #topics > 0 then
                        out[e.name:lower()] = { dir = e.name, topics = topics }
                    end
                end
            elseif e.name:sub(1, 1) ~= "." then
                loose = loose + 1
            end
        end
    end

    if loose > 0 and not warned_loose then
        warned_loose = true
        log_error(string.format(
            "HELP: %d file(s) sit directly in %s and are not reachable — a topic "
            .. "must live in a category directory, e.g. %s/<category>/<topic>.md",
            loose, DOC_ROOT, DOC_ROOT))
    end

    return out
end

--- Commands and topics folded together under one set of category names.
--- @return table categories  `{ [name] = { commands = {…}, topics = {…} } }`
--- @return number hidden     commands filtered out for want of permission
local function gather(session_id, docs, include_denied)
    local cats, hidden = {}, 0

    local function slot(name)
        cats[name] = cats[name] or { commands = {}, topics = {} }
        return cats[name]
    end

    for _, mod in pairs(Commands.registry()) do
        if type(mod) == "table" and mod.name then
            if include_denied or permitted(session_id, mod) then
                table.insert(slot((mod.category or "general"):lower()).commands, mod)
            else
                hidden = hidden + 1
            end
        end
    end

    for name, cat in pairs(docs) do
        slot(name).topics = cat.topics
    end

    return cats, hidden
end

--- The preferred order, then everything else alphabetically.
local function ordered(cats)
    local known, out, extra = {}, {}, {}
    for _, name in ipairs(CATEGORY_ORDER) do known[name] = true end
    for _, name in ipairs(CATEGORY_ORDER) do
        if cats[name] then out[#out + 1] = name end
    end
    for name in pairs(cats) do
        if not known[name] then extra[#extra + 1] = name end
    end
    table.sort(extra)
    for _, name in ipairs(extra) do out[#out + 1] = name end
    return out
end

-- ─── Rendering ───────────────────────────────────────────────────────────────

local function titlecase(s)
    return s:sub(1, 1):upper() .. s:sub(2)
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

--- One `left    right` row, wrapped so the right column keeps its column.
---
--- Not by padding the row and wrapping the result: a row wider than the
--- terminal gets re-flowed as one long sentence, and the padding — which is the
--- only thing making it a table — collapses to single spaces. `olc` and
--- `announce` did exactly that, so the listing stopped being a table on
--- precisely the rows whose summaries were long enough to be worth reading.
---
--- Nor by `pad_right`, which truncates: `perform, ability, perform` was cut to
--- twenty-two characters and ran straight into its own summary. A verb list too
--- long to share a line is given its own instead — losing a column is a layout
--- choice, losing an alias is a lie about what you can type.
local function row(margin, left, right, width)
    local lead = string.rep(" ", margin)
    if right == nil or right == "" then return lead .. left end

    local col   = 24
    local start = margin + col
    local hang  = string.rep(" ", start)

    if strings.visible_width(left) >= col then
        return lead .. left .. "\r\n" .. strings.wrap_tagged(hang .. right, width, start)
    end

    -- Only the right column is wrapped, so the left one never moves.
    local wrapped = strings.wrap_tagged(right, math.max(20, width - start), 0)
    local out = {}
    for line in (wrapped .. "\r\n"):gmatch("(.-)\r\n") do
        if #out == 0 then
            out[1] = lead .. left
                .. string.rep(" ", col - strings.visible_width(left)) .. line
        else
            out[#out + 1] = hang .. line
        end
    end
    return table.concat(out, "\r\n")
end

--- Everything leaves through here.
---
--- `send_paged` and not `send_lines`, which is what this used to use and which
--- never pages — so the old fifty-line list scrolled off the top of an eighty
--- by twenty-four terminal every time. It also colourises or strips per the
--- player's `color` setting, and chunks per their `pagesize`, so both of those
--- come for free. Not `literal`: this text is ours and its tags are meant to be
--- rendered.
---
--- No wrapping here: every line handed in has already been fitted by whoever
--- knows what shape it is. A second pass would re-flow the rows `row` just
--- laid out, which is the bug it exists to avoid.
local function emit(player, lines)
    -- Blocks end with a blank line so the next one has room, which leaves a
    -- stray one at the bottom when the last block is also the last thing. The
    -- pager appends its own newline, so it would show as two.
    while #lines > 0 and lines[#lines] == "" do lines[#lines] = nil end
    player:send_paged(table.concat(lines, "\r\n"))
end

--- Prose, wrapped to the player's terminal. `wrap_tagged` and not `wrap`
--- because `{cyan}` costs no screen columns, and a line counted with its tags
--- folds early enough to look like a mistake.
local function fit(text, width, indent)
    return strings.wrap_tagged(text, width, indent or 0)
end

--- The top level: what categories exist and how big each is.
local function show_categories(player, cats, hidden)
    local width = player:get_width()
    local lines = {
        "{cyan}Help{/}",
        "",
        fit("  Type {cyan}help <category>{/} for what is in one, {cyan}help <command>{/} "
            .. "for one command, or {cyan}help <topic>{/} for a page.", width, 2),
        "",
    }

    for _, name in ipairs(ordered(cats)) do
        local cat = cats[name]
        local counts = {}
        if #cat.commands > 0 then
            counts[#counts + 1] = #cat.commands .. " command"
                .. (#cat.commands == 1 and "" or "s")
        end
        if #cat.topics > 0 then
            counts[#counts + 1] = #cat.topics .. " topic"
                .. (#cat.topics == 1 and "" or "s")
        end
        lines[#lines + 1] = row(2, "{yellow}" .. titlecase(name) .. "{/}",
            table.concat(counts, ", "), width)
    end

    lines[#lines + 1] = ""
    if hidden > 0 then
        lines[#lines + 1] = fit(string.format(
            "{cyan}%d command(s) hidden — you lack the permission. `help all` lists them.{/}",
            hidden), width, 0)
    end

    emit(player, lines)
end

--- One category: its commands, then its topics.
local function show_category(player, name, cat)
    local width = player:get_width()
    local lines = { "{yellow}" .. titlecase(name) .. "{/}", "" }

    if #cat.commands > 0 then
        table.sort(cat.commands, function(a, b) return a.name < b.name end)
        if #cat.topics > 0 then
            lines[#lines + 1] = "  {yellow}Commands{/}"
        end
        for _, mod in ipairs(cat.commands) do
            lines[#lines + 1] = row(4, verb_list(mod), mod.summary, width)
        end
        lines[#lines + 1] = ""
    end

    if #cat.topics > 0 then
        if #cat.commands > 0 then
            lines[#lines + 1] = "  {yellow}Topics{/}"
        end
        for _, topic in ipairs(cat.topics) do
            lines[#lines + 1] = "    " .. topic.name
        end
        lines[#lines + 1] = ""
    end

    lines[#lines + 1] = "Type {cyan}help <command-or-topic>{/} for detail."
    emit(player, lines)
end

--- Today's flat list of everything, kept for `help all`. Commands only — this
--- is the "what can I type" view, and it deliberately includes the ones you
--- cannot, which is the whole reason to ask for it.
local function show_all(player, cats)
    local width = player:get_width()
    local lines = {}
    local total = 0

    for _, name in ipairs(ordered(cats)) do
        local list = cats[name].commands
        if #list > 0 then
            table.sort(list, function(a, b) return a.name < b.name end)
            lines[#lines + 1] = "{yellow}" .. titlecase(name) .. "{/}"
            for _, mod in ipairs(list) do
                lines[#lines + 1] = row(2, verb_list(mod), mod.summary, width)
                total = total + 1
            end
            lines[#lines + 1] = ""
        end
    end

    table.insert(lines, 1, "")
    table.insert(lines, 1, "{cyan}Commands (" .. total .. "){/}")
    lines[#lines + 1] = "Type {cyan}help <command>{/} for detail on one of them."
    emit(player, lines)
end

--- One command in detail. `usage` and `help` are optional on the module; when
--- neither is set the summary is all there is to say, and saying so beats
--- inventing filler.
local function show_one(player, mod, docs)
    local width = player:get_width()
    local lines = {
        "{cyan}" .. mod.name .. "{/}" .. (mod.permission and
            ("  {red}(requires " .. mod.permission .. "){/}") or ""),
        "",
        fit("  " .. (mod.summary or "No summary."), width, 2),
    }

    if #(mod.aliases or {}) > 0 then
        lines[#lines + 1] = fit(
            "  {yellow}Aliases:{/} " .. table.concat(mod.aliases, ", "), width, 4)
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
            lines[#lines + 1] = fit("  " .. line, width, 2)
        end
    end

    -- A command wins a name clash with a topic, because `help <verb>` has to
    -- keep describing the verb you would type. The page is one line away rather
    -- than unreachable.
    local seen = {}
    for _, name in ipairs(ordered(docs)) do
        for _, topic in ipairs(docs[name].topics) do
            if topic.name == mod.name and not seen[name] then
                seen[name] = true
                lines[#lines + 1] = ""
                lines[#lines + 1] = "  {yellow}See also:{/} {cyan}help "
                    .. name .. "/" .. topic.name .. "{/}"
            end
        end
    end

    emit(player, lines)
end

--- An authored page. Markdown if it is named `.md`, otherwise wrapped as-is.
local function show_topic(player, cat, topic)
    -- The path is built from what `list_dir` reported, never from what the
    -- player typed — the typed name is matched against the discovered list
    -- first. The jail would refuse a `..` anyway; not constructing one means
    -- never finding out whether it would.
    local path = DOC_ROOT .. "/" .. cat.dir .. "/" .. topic.file

    local ok, content = pcall(read_file, path)
    if not ok or type(content) ~= "string" then
        log_error("HELP: cannot read '" .. path .. "': " .. tostring(content))
        player:send("{red}That topic exists but could not be read.{/}")
        return
    end

    local width = player:get_width()
    player:send_paged(topic.markdown and Markdown.render(content, width)
        or Markdown.plain(content, width))
end

--- Every `<category>, <topic>` pair whose topic is called `name`.
local function find_topic(docs, name)
    local hits = {}
    for _, cat_name in ipairs(ordered(docs)) do
        for _, topic in ipairs(docs[cat_name].topics) do
            if topic.name == name then
                hits[#hits + 1] = { cat_name = cat_name, cat = docs[cat_name], topic = topic }
            end
        end
    end
    return hits
end

-- ─── Dispatch ────────────────────────────────────────────────────────────────

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    local docs  = doc_categories(session_id)
    local topic = args[1] and args[1]:lower()

    if topic == nil then
        local cats, hidden = gather(session_id, docs, false)
        show_categories(player, cats, hidden)
        return
    end

    if topic == "all" then
        show_all(player, (gather(session_id, docs, true)))
        return
    end

    -- `help combat/stances` — the explicit form, and the only one that always
    -- works. Everything below it can be ambiguous.
    local cat_name, topic_name = topic:match("^([^/]+)/(.+)$")
    if cat_name then
        local cat = docs[cat_name]
        if cat then
            for _, t in ipairs(cat.topics) do
                if t.name == topic_name then
                    show_topic(player, cat, t)
                    return
                end
            end
        end
        player:send("{red}No topic '" .. topic .. "'.{/} Type {cyan}help "
            .. cat_name .. "{/} for what is in that category.")
        return
    end

    -- A command name or one of its aliases, resolved through the dispatcher's
    -- own alias table so `help l` and `help look` agree with what typing `l`
    -- actually does — two lookups that could disagree would be a help system
    -- that lies.
    local canonical = Commands.resolve(topic)
    local mod = canonical and Commands.registry()[canonical]
    if mod then
        show_one(player, mod, docs)
        return
    end

    local cats = (gather(session_id, docs, false))
    if cats[topic] then
        show_category(player, topic, cats[topic])
        return
    end

    -- A bare topic name. One hit shows it; several refuse and print the list,
    -- the same rule `lib/matching.lua` follows for `2.rat` and for the same
    -- reason — guessing wrong costs the player their next command, and the list
    -- tells them exactly what to type instead.
    local hits = find_topic(docs, topic)
    if #hits == 1 then
        show_topic(player, hits[1].cat, hits[1].topic)
        return
    elseif #hits > 1 then
        local lines = { "{yellow}Which '" .. topic .. "'?{/}", "" }
        for _, hit in ipairs(hits) do
            lines[#lines + 1] = "  {cyan}help " .. hit.cat_name .. "/" .. topic .. "{/}"
        end
        emit(player, lines)
        return
    end

    player:send("{red}No command or topic '" .. topic .. "'.{/} Type {cyan}help{/} for the categories.")
end

return M
