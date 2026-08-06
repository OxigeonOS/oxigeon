-- mudlib/daemons/editor_d.lua — Typing more than one line.
--
-- A room description is six lines of prose, and until now there was no way to
-- enter one. `pager_d` is output-only; `login.lua` is a state machine reachable
-- from `on_input` and not from a command; `game/cmds/board.lua` fakes a
-- two-field post with a `|` pipe and says so in a comment — *"a board that needs
-- an editor session to post one line is a board nobody posts to."*
--
-- `docs/src/lua-api/olc.md` argued against building one: *"An in-game editor for
-- prose would be a worse text editor than the one you already have."* That is
-- correct about a **text editor** and wrong about **a way to type six lines**.
-- The alternatives are a `|`-delimited one-liner, which becomes unusable around
-- line four, and "go and edit the file", which is what OLC exists to avoid.
--
-- ─── The shape ───────────────────────────────────────────────────────────────
--
-- Exactly `pager_d`'s: a per-session table, an `is_editing` predicate, a
-- `handle_input`, and one interception at the top of `Commands.dispatch` —
-- immediately *after* the pager's. Two interceptors in a fixed order is easier
-- to reason about than one that arbitrates between them.
--
-- Commands are dot-prefixed so a line of prose is never eaten. `quit` typed in
-- here is *text*, deliberately: a description containing the word is ordinary,
-- and losing an hour's writing to it would not be. `.q` is the only way out.

local M = {}

-- session_id → { title, lines = {}, on_save, on_abort }
local _editing = {}

--- A bound on one buffer. Prose, not a file: a description forty lines long is
--- a mistake, and a runaway paste should stop rather than grow without limit.
M.MAX_LINES = 200

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

local HELP = {
    "{cyan}Editor{/}  — type prose; lines beginning with a dot are commands.",
    "  {yellow}.s{/} / {yellow}.save{/}    save and close",
    "  {yellow}.q{/} / {yellow}.abort{/}   discard and close",
    "  {yellow}.l{/}            list with line numbers",
    "  {yellow}.d <n>{/}        delete line n",
    "  {yellow}.i <n> <text>{/} insert before line n",
    "  {yellow}.c{/}            clear the buffer",
    "  {yellow}.h{/}            this help",
    "  {yellow}..{/}            a literal line starting with a dot",
}

--- Speak to the session, whether or not a player is behind it.
---
--- `get_player` *raises* on an id that is not a real session, so it is guarded
--- rather than merely checked. The editor is usable from a daemon and from a
--- test with a synthetic id, and a helper that only worked for a logged-in
--- player would make both impossible.
local function send_to(session_id, text)
    local ok, player = pcall(get_player, session_id)
    if ok and player then
        player:send(text)
    else
        pcall(send, session_id, text .. "\r\n")
    end
end

--- Show the buffer with line numbers.
local function list(session_id, state)
    if #state.lines == 0 then
        send_to(session_id, "{yellow}(empty){/}")
        return
    end
    local out = {}
    for i, line in ipairs(state.lines) do
        out[#out + 1] = string.format("{yellow}%3d{/}] %s", i, line)
    end
    local ok, player = pcall(get_player, session_id)
    if ok and player then
        -- `literal`, because the buffer is the builder's prose: a `{red}` they
        -- typed is text they typed, not markup for us to render.
        player:send_paged(table.concat(out, "\r\n"), { literal = true })
    else
        pcall(send, session_id, table.concat(out, "\r\n") .. "\r\n")
    end
end

--- Open an editing session.
---
--- `initial` is pre-loaded, so editing a description is editing rather than
--- retyping. That is most of the difference between an editor somebody uses and
--- one they work around.
--- @param session_id string
--- @param opts table  { title, initial = string|table, on_save, on_abort }
--- @return boolean
function M.open(session_id, opts)
    opts = opts or {}
    if _editing[session_id] then
        send_to(session_id, "{red}You are already editing something.{/}")
        return false
    end

    local lines = {}
    if type(opts.initial) == "string" and opts.initial ~= "" then
        for line in (opts.initial .. "\n"):gmatch("(.-)\r?\n") do
            lines[#lines + 1] = line
        end
        -- Drop the empty tail `gmatch` leaves on a string ending in a newline.
        if #lines > 0 and lines[#lines] == "" then lines[#lines] = nil end
    elseif type(opts.initial) == "table" then
        for _, line in ipairs(opts.initial) do lines[#lines + 1] = line end
    end

    _editing[session_id] = {
        title    = opts.title or "text",
        lines    = lines,
        on_save  = opts.on_save,
        on_abort = opts.on_abort,
    }

    send_to(session_id, "{cyan}[EDITOR]{/} " .. (opts.title or "text")
        .. " — " .. #lines .. " line" .. (#lines == 1 and "" or "s")
        .. ".  {yellow}.s{/} save  {yellow}.q{/} abort  {yellow}.h{/} help")
    if #lines > 0 then list(session_id, _editing[session_id]) end
    return true
end

--- Is this session mid-edit?
--- @param session_id string
--- @return boolean
function M.is_editing(session_id)
    return _editing[session_id] ~= nil
end

--- Close without saving, running no callback. For cleanup paths.
--- @param session_id string
function M.stop(session_id)
    _editing[session_id] = nil
end

--- Close and run `on_abort`.
--- @param session_id string
function M.abort(session_id)
    local state = _editing[session_id]
    _editing[session_id] = nil
    if state and type(state.on_abort) == "function" then
        local ok, err = pcall(state.on_abort)
        if not ok then log_error("EDITOR_D: on_abort raised: " .. tostring(err)) end
    end
end

--- Close and run `on_save` with the joined text.
--- @param session_id string
function M.save(session_id)
    local state = _editing[session_id]
    if not state then return end
    _editing[session_id] = nil

    local text = table.concat(state.lines, "\n")
    send_to(session_id, "{green}[EDITOR]{/} Saved " .. #state.lines .. " line"
        .. (#state.lines == 1 and "" or "s") .. ", " .. #text .. " characters.")

    if type(state.on_save) == "function" then
        local ok, err = pcall(state.on_save, text, state.lines)
        if not ok then
            log_error("EDITOR_D: on_save raised: " .. tostring(err))
            send_to(session_id, "{red}[EDITOR] Saving failed. See the log.{/}")
        end
    end
end

--- One line of input, while editing.
---
--- Called from `Commands.dispatch` before anything else looks at the text, so a
--- line of prose is never parsed as a verb.
--- @param session_id string
--- @param text string
function M.handle_input(session_id, text)
    local state = _editing[session_id]
    if not state then return end
    text = text or ""

    -- `..` escapes a literal leading dot, so a line of prose may begin with one.
    if text:sub(1, 2) == ".." then
        text = text:sub(2)
    elseif text:sub(1, 1) == "." then
        local cmd, rest = text:match("^%.(%a*)%s*(.*)$")
        cmd = (cmd or ""):lower()

        if cmd == "s" or cmd == "save" then
            return M.save(session_id)

        elseif cmd == "q" or cmd == "abort" then
            send_to(session_id, "{yellow}[EDITOR]{/} Discarded.")
            return M.abort(session_id)

        elseif cmd == "l" or cmd == "list" then
            list(session_id, state)
            return

        elseif cmd == "c" or cmd == "clear" then
            state.lines = {}
            send_to(session_id, "{yellow}[EDITOR]{/} Buffer cleared.")
            return

        elseif cmd == "h" or cmd == "help" then
            local ok, player = pcall(get_player, session_id)
            if ok and player then player:send_lines(HELP) end
            return

        elseif cmd == "d" then
            local n = tonumber(rest)
            if not n or not state.lines[n] then
                send_to(session_id, "{red}[EDITOR] No line " .. tostring(rest) .. ".{/}")
            else
                table.remove(state.lines, n)
                send_to(session_id, "{yellow}[EDITOR]{/} Deleted line " .. n .. ".")
            end
            return

        elseif cmd == "i" then
            local n, body = rest:match("^(%d+)%s?(.*)$")
            n = tonumber(n)
            if not n or n < 1 or n > #state.lines + 1 then
                send_to(session_id, "{red}[EDITOR] Usage: .i <line> <text>{/}")
            else
                table.insert(state.lines, n, body or "")
                send_to(session_id, "{yellow}[EDITOR]{/} Inserted before line " .. n .. ".")
            end
            return

        else
            send_to(session_id, "{red}[EDITOR] Unknown command '." .. cmd
                .. "'. `.h` for help, `..` for a literal dot.{/}")
            return
        end
    end

    if #state.lines >= M.MAX_LINES then
        send_to(session_id, "{red}[EDITOR] " .. M.MAX_LINES
            .. " lines is the limit. `.s` to save, `.d <n>` to make room.{/}")
        return
    end

    state.lines[#state.lines + 1] = text
end

--- Forget a session. Called from on_disconnect.
---
--- An editor left open by a dropped connection would wedge that session for
--- ever: every subsequent line would be buffered as prose into a buffer nobody
--- will ever save.
--- @param session_id string
function M.cleanup(session_id)
    _editing[session_id] = nil
end

log("debug", "EDITOR_D: daemon loaded")

return M
