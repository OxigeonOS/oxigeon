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
    if not text or text == "" then return text end

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
