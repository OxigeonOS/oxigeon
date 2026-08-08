-- mudlib/lib/markdown.lua — Markdown for a terminal, in the four constructs
-- that survive the trip.
--
-- A help page is prose somebody wrote in a text editor; a MUD screen is 80-odd
-- columns of monospace with no tables, no images and no hyperlinks. Almost all
-- of Markdown has nowhere to go. What is left is the part that carries
-- *structure*: headings, bullets, and the blank line between paragraphs.
--
--   # Title        {bold}{white}===[ Title ]==={/}
--   ## Section     {bold}{white}=== Section ==={/}
--   ### Sub        {bold}{white}Sub{/}
--   - item         an indented bullet, wrapped with a hanging indent
--   <blank>        a blank line
--   anything else  paragraph text
--
-- Two rules decide everything not in that list, and both are the rule
-- `CLAUDE.md` states for message tokens — an unknown token survives verbatim:
--
--   * **Nothing is eaten.** A code fence, a `*emphasis*`, a `[link](x)` and a
--     pipe table all arrive as ordinary paragraph text. A visible ``` is a typo
--     an author can see and fix; a block that silently vanished is a bug they
--     will stare at the source of.
--   * **`{colour}` tags pass through.** A help page is authored *content*, so
--     `{red}Danger.{/}` is a feature, not a leak. This is the opposite of
--     `cmds/building/cat.lua`, which pages files `literal` precisely because a
--     source file's tags must not be rendered — the difference is whether the
--     thing on screen is prose or code.
--
-- This module returns a string carrying tags, never ANSI. Colour is a *player*
-- preference (`color_enabled` lives on Player), so translating it is
-- `Player:send_paged`'s job and not this file's. Same reason `lib/color.lua`
-- is not required here.

local strings = require('lib.strings')

local M = {}

--- Headings are the one thing that is not wrapped.
---
--- Wrapping `===[ A Very Long Title ]===` breaks the box across two lines and
--- leaves a stray `]===`, which reads as corruption rather than as a long
--- title. An overlong heading is instead left for the client to fold — ugly in
--- a way that is obviously the author's doing.
local function heading(level, title)
    if level == 1 then
        return "{bold}{white}===[ " .. title .. " ]==={/}"
    elseif level == 2 then
        return "{bold}{white}=== " .. title .. " ==={/}"
    end
    -- `####` and deeper degrade to `###` rather than printing their hashes.
    return "{bold}{white}" .. title .. "{/}"
end

--- Split into lines with the line endings normalised. `\r` alone counts: a file
--- written on a classic Mac, or mangled in transit, would otherwise be one line.
local function lines_of(text)
    local out = {}
    text = text:gsub("\r\n", "\n"):gsub("\r", "\n")
    for line in (text .. "\n"):gmatch("([^\n]*)\n") do
        out[#out + 1] = line
    end
    -- `gmatch` over `text .. "\n"` yields a trailing empty line for a file that
    -- already ended in one.
    if #out > 0 and out[#out] == "" then out[#out] = nil end
    return out
end

--- Render Markdown source for a terminal `width` columns wide.
--- @param text  string
--- @param width number|nil  default 80
--- @return string  `\r\n`-joined, tagged, wrapped
function M.render(text, width)
    if type(text) ~= "string" then return "" end
    width = width or 80

    local out, para = {}, {}

    --- Consecutive non-blank lines are one paragraph, joined and re-wrapped.
    --- That is what makes a hard-wrapped source file reflow to the reader's
    --- terminal instead of wrapping twice — an 80-column file read at 60
    --- columns would otherwise come out as alternating long and stub lines.
    local function flush_para()
        if #para == 0 then return end
        out[#out + 1] = strings.wrap_tagged(table.concat(para, " "), width, 0)
        para = {}
    end

    --- A blank line, but never two: runs of them collapse, and one at the top
    --- of the file is dropped.
    local function blank()
        flush_para()
        if #out > 0 and out[#out] ~= "" then out[#out + 1] = "" end
    end

    for _, line in ipairs(lines_of(text)) do
        local hashes, title = line:match("^%s*(#+)%s+(.*)$")
        local lead, item    = line:match("^(%s*)%-%s+(.*)$")

        if line:match("^%s*$") then
            blank()

        elseif hashes then
            -- A heading always opens a block, whether or not the author left a
            -- blank line above it.
            blank()
            out[#out + 1] = heading(#hashes, strings.trim(title))

        elseif lead then
            -- `- item` only, with the space. `---` and `-item` are not lists
            -- and fall through to paragraph text, which is what an author who
            -- typed either of those wants to see.
            flush_para()
            local depth  = math.floor(#(lead:gsub("\t", "  ")) / 2)
            local margin = 2 + depth * 2
            out[#out + 1] = strings.wrap_tagged(
                string.rep(" ", margin) .. "- " .. strings.trim(item),
                width, margin + 2)

        else
            para[#para + 1] = strings.trim(line)
        end
    end
    flush_para()

    while #out > 0 and out[#out] == "" do out[#out] = nil end
    return table.concat(out, "\r\n")
end

--- Wrap a file that is not Markdown — no parsing, hard line breaks kept.
---
--- A topic with no extension is already laid out the way its author wanted, so
--- joining its lines into paragraphs would destroy an ASCII map or a table.
--- Only the overlong lines move, and only because the alternative is the
--- client folding them at a width nothing else on screen agrees with.
--- @param text  string
--- @param width number|nil  default 80
--- @return string
function M.plain(text, width)
    if type(text) ~= "string" then return "" end
    width = width or 80

    local out = {}
    for _, line in ipairs(lines_of(text)) do
        out[#out + 1] = strings.wrap_tagged(line, width, 0)
    end

    while #out > 0 and out[#out] == "" do out[#out] = nil end
    return table.concat(out, "\r\n")
end

return M
