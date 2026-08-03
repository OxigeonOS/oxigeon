-- game/cmds/board.lua — Read and write the notice board.
--
-- A game command, in `game/cmds/`, because the board is content. It is also the
-- first command in the game layer at all, which is worth noticing: everything
-- else so far has been the mudlib's, and `game.command_paths` searching the
-- game root first is what makes this possible without any registration.

local Board = require('daemons.board_d')

local M = {}
M.name = 'board'
M.aliases = { 'notices' }
M.category = 'communication'
M.summary = 'Read and post notices.'
M.usage = {
    "board                       the latest notices",
    "board <category>            news, trade, help or rp",
    "board read <id>             one notice in full",
    "board post <cat> <subject> | <body>",
    "board search <text>         subjects and bodies",
    "board mine                  what you have posted",
    "board remove <id>           take one of yours down",
}
M.permission = nil

--- "3 hours ago" beats a timestamp on a board: the useful question is how stale
--- a notice is, not what o'clock it was written.
local function ago(seconds)
    local d = os_time() - (seconds or 0)
    if d < 60 then return "just now" end
    if d < 3600 then return math.floor(d / 60) .. "m ago" end
    if d < 86400 then return math.floor(d / 3600) .. "h ago" end
    return math.floor(d / 86400) .. "d ago"
end

local function show_list(player, rows, heading)
    if #rows == 0 then
        player:send("{yellow}Nothing on the board.{/}")
        return
    end

    local lines = { "{cyan}" .. heading .. "{/} (" .. #rows .. ")", "" }
    lines[#lines + 1] = string.format("  {yellow}%-14s %-8s %-30s %-12s %5s{/}",
        "id", "cat", "subject", "by", "when")
    for _, row in ipairs(rows) do
        lines[#lines + 1] = string.format("  %-14s %-8s %-30s %-12s %5s",
            row.id, row.category or "?", (row.subject or ""):sub(1, 30),
            (row.author or "?"):sub(1, 12), ago(row.posted))
    end
    lines[#lines + 1] = ""
    lines[#lines + 1] = "Read one with {cyan}board read <id>{/}."
    player:send_lines(lines)
end

local function show_one(player, id)
    local doc = Board.read(id)
    if not doc then
        player:send("{red}There is no notice '" .. id .. "'.{/}")
        return
    end

    local lines = {
        "{cyan}" .. (doc.subject or "(no subject)") .. "{/}",
        string.format("  {yellow}%s{/} by {yellow}%s{/}, %s — %d view(s)",
            doc.category or "?", doc.author or "?", ago(doc.posted), doc.views or 0),
        "",
    }
    for line in ((doc.body or "") .. "\n"):gmatch("(.-)\n") do
        lines[#lines + 1] = "  " .. line
    end
    if doc.edited then
        lines[#lines + 1] = ""
        lines[#lines + 1] = "  {cyan}(edited " .. ago(doc.edited) .. "){/}"
    end
    player:send_lines(lines)
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    local verb = (args[1] or ""):lower()

    if verb == "" then
        show_list(player, Board.list(nil, {}), "Notice board")
        return
    end

    if verb == "read" or verb == "r" then
        if not args[2] then player:send("{cyan}Read which notice?{/}") return end
        show_one(player, args[2])
        return
    end

    if verb == "post" then
        -- `board post trade Selling ore | Two silver a bar, ask in the market.`
        -- A pipe rather than a second command, because a board that needs an
        -- editor session to post one line is a board nobody posts to.
        local rest = args_str:gsub("^%s*post%s+", "")
        local category, remainder = rest:match("^(%S+)%s+(.+)$")
        if not category then
            player:send("{cyan}Usage: board post <category> <subject> | <body>{/}")
            player:send("Categories: " .. table.concat(Board.CATEGORIES, ", "))
            return
        end
        local subject, body = remainder:match("^(.-)%s*|%s*(.+)$")
        if not subject then
            -- No pipe: the whole thing is the subject and the body repeats it.
            -- Better than refusing — a one-line notice is a normal thing to want.
            subject, body = remainder, remainder
        end

        local id, why = Board.post(player, category, subject, body)
        if not id then
            player:send("{red}" .. (why or "It would not stick.") .. "{/}")
            return
        end
        player:send("{green}Posted as " .. id .. ".{/}")
        return
    end

    if verb == "search" then
        local text = args_str:gsub("^%s*search%s+", "")
        if text == "" or text == args_str then
            player:send("{cyan}Search for what?{/}")
            return
        end
        show_list(player, Board.search(text), "Notices matching '" .. text .. "'")
        return
    end

    if verb == "mine" then
        show_list(player, Board.by_authors({ player.name }), "Your notices")
        return
    end

    if verb == "remove" or verb == "delete" then
        if not args[2] then player:send("{cyan}Remove which notice?{/}") return end
        local is_staff = type(has_permission) == "function"
            and has_permission(session_id, "board.moderate")
        local ok, why = Board.remove(player, args[2], is_staff)
        player:send(ok and "{green}Taken down.{/}"
            or ("{red}" .. (why or "It would not come down.") .. "{/}"))
        return
    end

    if verb == "edit" then
        local rest = args_str:gsub("^%s*edit%s+", "")
        local id, remainder = rest:match("^(%S+)%s+(.+)$")
        if not id then
            player:send("{cyan}Usage: board edit <id> <subject> | <body>{/}")
            return
        end
        local subject, body = remainder:match("^(.-)%s*|%s*(.+)$")
        local ok, why = Board.edit(player, id, subject or remainder, body)
        player:send(ok and "{green}Changed.{/}"
            or ("{red}" .. (why or "It would not change.") .. "{/}"))
        return
    end

    -- Anything else is treated as a category, which is what people try first.
    if Board.is_category(verb) then
        show_list(player, Board.list(verb, {}), "Notice board — " .. verb)
        return
    end

    player:send("{red}Unknown option '" .. verb .. "'.{/}")
    player:send_lines(M.usage)
end

return M
