-- mudlib/lib/strings.lua — String manipulation utilities

local M = {}

--- Trim whitespace from both ends of a string
function M.trim(s)
    return s:gsub("^%s+", ""):gsub("%s+$", "")
end

--- Split a string by a delimiter
function M.split(s, sep)
    local parts = {}
    local pattern = "([^" .. sep .. "]*)" .. sep .. "?"
    for part in s:gmatch(pattern) do
        if part ~= "" then
            table.insert(parts, part)
        end
    end
    return parts
end

--- Pad a string to a fixed width
function M.pad_right(s, width)
    s = tostring(s)
    while #s < width do
        s = s .. " "
    end
    return s:sub(1, width)
end

--- Pad a string on the left to a fixed width
function M.pad_left(s, width)
    s = tostring(s)
    while #s < width do
        s = " " .. s
    end
    return s:sub(-width)
end

--- Word-wrap text to a given width, preserving existing structure.
-- Handles:
--   - Existing hard line breaks (\n or \r\n) are preserved
--   - Blank lines (paragraph separators) are preserved
--   - Only lines exceeding `width` are wrapped at word boundaries
--   - Output uses \r\n line endings (MUD convention)
-- @param text  string  The text to wrap
-- @param width number  Maximum line width (default 80)
-- @return string       The wrapped text
function M.wrap(text, width)
    width = width or 80
    if text == nil or text == "" then return text end
    if type(text) ~= "string" then text = tostring(text) end

    -- Normalize line endings to \n for processing
    text = text:gsub("\r\n", "\n")

    -- Split into lines, preserving blank lines
    local input_lines = {}
    for line in (text .. "\n"):gmatch("([^\n]*)\n") do
        input_lines[#input_lines + 1] = line
    end

    local output = {}
    for _, line in ipairs(input_lines) do
        -- Blank lines pass through (paragraph separators)
        if line:match("^%s*$") then
            output[#output + 1] = ""
        elseif #line <= width then
            -- Line fits, pass through
            output[#output + 1] = line
        else
            -- Line is too long — wrap at word boundaries
            local current = ""
            for word in line:gmatch("%S+") do
                if current == "" then
                    current = word
                elseif #current + 1 + #word > width then
                    output[#output + 1] = current
                    current = word
                else
                    current = current .. " " .. word
                end
            end
            if current ~= "" then
                output[#output + 1] = current
            end
        end
    end

    -- Remove trailing blank line if the input didn't end with one
    if #output > 0 and output[#output] == "" and not text:match("\n$") then
        output[#output] = nil
    end

    return table.concat(output, "\r\n")
end

--- The width `text` occupies on a terminal, with `{colour}` tags costing zero.
--- @param text string
--- @return number
function M.visible_width(text)
    if type(text) ~= "string" then return 0 end
    return #(text:gsub("{.-}", ""))
end

--- Word-wrap text whose `{colour}` tags do not occupy screen columns.
---
--- `wrap` above counts every character, tags included, because it wraps before
--- anything colourises and has no view on which runs are invisible. Every
--- caller that puts a tag at the *start* of a line lives with that, and the
--- error is small there. It is not small for generated layout: a line carrying
--- `{bold}{white}…{/}` loses seventeen columns, so a heading and the paragraph
--- under it wrap at different widths and stop lining up.
---
--- Two other differences from `wrap`, both for lists:
---   * leading whitespace is preserved rather than eaten, so an indented line
---     stays indented;
---   * continuation lines are indented by `indent` columns, which is what makes
---     a wrapped bullet hang under its own text instead of under the dash.
---
--- `wrap` is deliberately untouched — this is an addition, not a replacement.
--- @param text   string
--- @param width  number   maximum visible columns (default 80)
--- @param indent number|nil  columns to indent continuation lines (default 0)
--- @return string
function M.wrap_tagged(text, width, indent)
    width  = width or 80
    indent = indent or 0
    if text == nil or text == "" then return text end
    if type(text) ~= "string" then text = tostring(text) end

    text = text:gsub("\r\n", "\n")

    local output = {}
    for line in (text .. "\n"):gmatch("([^\n]*)\n") do
        if line:match("^%s*$") then
            output[#output + 1] = ""
        elseif M.visible_width(line) <= width then
            -- Verbatim, spacing and all. Re-joining the words of a line that
            -- already fits would collapse `  a  |  b` to `a | b` — and a table
            -- or an ASCII diagram is exactly the thing somebody hands to a
            -- wrapper hoping it will be left alone. `wrap` has the same fast
            -- path for the same reason.
            output[#output + 1] = line
        else
            local lead = line:match("^(%s*)") or ""
            local current, used = nil, 0
            for word in line:gmatch("%S+") do
                local w = M.visible_width(word)
                if current == nil then
                    current, used = lead .. word, #lead + w
                elseif used + 1 + w > width then
                    output[#output + 1] = current
                    current, used = string.rep(" ", indent) .. word, indent + w
                else
                    current, used = current .. " " .. word, used + 1 + w
                end
            end
            if current then output[#output + 1] = current end
        end
    end

    if #output > 0 and output[#output] == "" and not text:match("\n$") then
        output[#output] = nil
    end

    return table.concat(output, "\r\n")
end

--- Render a number for a player, identically on every Lua.
---
--- `tostring(6.0)` is `"6"` on LuaJIT — where every number is a double — and
--- `"6.0"` from Lua 5.3 on, where integers are a real subtype. A trait declared
--- `round = "none"` genuinely holds a float, so `score` printed
--- `Attunement 6.0` on one runtime and `Attunement 6` on the other for the same
--- character. This is the seam, in one place.
---
--- Integral values print without a decimal point; a real fraction keeps enough
--- digits to be worth reading and no more.
--- @param n number
--- @return string
function M.number(n)
    if type(n) ~= "number" then return tostring(n) end
    -- NaN and the infinities have no integral form and must not reach `%d`.
    if n ~= n or n == math.huge or n == -math.huge then return tostring(n) end
    if n % 1 == 0 then
        return string.format("%d", math.floor(n))
    end
    return string.format("%.4g", n)
end

--- Format a number with thousands separators
function M.format_number(n)
    local s = tostring(math.floor(n))
    local result = ""
    local len = #s
    for i = 1, len do
        result = result .. s:sub(i, i)
        if (len - i) % 3 == 0 and i < len then
            result = result .. ","
        end
    end
    return result
end

return M
