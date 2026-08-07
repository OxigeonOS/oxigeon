-- mudlib/daemons/pager_d.lua — Long output, one screen at a time.
--
-- ─── Every path here ends its output with a newline ──────────────────────────
--
-- `Player:_process_output` appends `\r\n` to everything it sends, and
-- `send_prompt` adds no leading newline — so the prompt sits on the line after
-- whatever was last sent. This daemon writes through the raw `send` efun and so
-- has to keep that contract itself.
--
-- The short-text path did not, for as long as it existed: text under one page
-- was sent verbatim and the prompt landed **on** its last line. It showed up
-- worst in `olc`, which pages more than anything else and whose output is
-- almost always shorter than a screen — so the paged path, which did append,
-- was almost never the one taken.

local M = {}

local _paging = {}

local function log_error(msg)
    log("error", msg)
    if DAEMON and DAEMON.journal then
        local ok = pcall(function() DAEMON.journal.error(msg) end)
    end
end

function M.page(session_id, text, page_length)
    if not text then return end
    page_length = page_length or 20
    
    local lines = {}
    for line in string.gmatch(text .. "\n", "(.-)\r?\n") do
        table.insert(lines, line)
    end
    
    if #lines <= page_length then
        -- Terminated, like `_send_page` below and like `Player:_process_output`.
        -- Unconditionally rather than only-when-missing: the two paths in this
        -- file have to agree, and a caller that wanted a blank line should say
        -- so rather than depend on whether its text happened to fit a screen.
        local ok, err = pcall(function() send(session_id, text .. "\r\n") end)
        if not ok then log_error("pager_d page err: " .. tostring(err)) end
        return
    end
    
    _paging[session_id] = {
        lines = lines,
        current_line = 1,
        page_length = page_length
    }
    
    M._send_page(session_id)
end

function M._send_page(session_id)
    local p = _paging[session_id]
    if not p then return end
    
    local end_line = math.min(p.current_line + p.page_length - 1, #p.lines)
    local out = {}
    for i = p.current_line, end_line do
        table.insert(out, p.lines[i])
    end
    
    p.current_line = end_line + 1
    
    local out_str = table.concat(out, "\r\n") .. "\r\n"
    local ok, err = pcall(function() send(session_id, out_str) end)
    if not ok then log_error("pager_d send err: " .. tostring(err)) end
    
    if p.current_line > #p.lines then
        _paging[session_id] = nil
    else
        local p_ok, p_err = pcall(function() send_prompt(session_id, "--More-- (Enter=next, q=quit, a=all) ") end)
        if not p_ok then log_error("pager_d prompt err: " .. tostring(p_err)) end
    end
end

function M.is_paging(session_id)
    return _paging[session_id] ~= nil
end

function M.handle_input(session_id, input)
    local p = _paging[session_id]
    if not p then return false end
    
    input = input or ""
    
    if input == "" or input == " " then
        M._send_page(session_id)
        return true
    elseif input == "q" or input == "Q" then
        M.stop(session_id)
        return true
    elseif input == "a" or input == "A" then
        p.page_length = #p.lines
        M._send_page(session_id)
        return true
    end
    
    return true
end

function M.stop(session_id)
    _paging[session_id] = nil
end

log("info", "pager_d loaded")
return M
