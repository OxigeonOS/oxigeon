-- mudlib/lib/utils.lua — Shared Lua utilities for Oxigeon mudlibs

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

--- Wrap text to a given width
function M.wrap(text, width)
    width = width or 78
    local result = {}
    local line = ""
    for word in text:gmatch("%S+") do
        if #line + #word + 1 > width then
            table.insert(result, line)
            line = word
        else
            line = line == "" and word or line .. " " .. word
        end
    end
    if line ~= "" then
        table.insert(result, line)
    end
    return table.concat(result, "\r\n")
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

--- Check if a value is in a table
function M.contains(t, val)
    for _, v in ipairs(t) do
        if v == val then return true end
    end
    return false
end

--- Deep copy a table
function M.deepcopy(t)
    if type(t) ~= "table" then return t end
    local copy = {}
    for k, v in pairs(t) do
        copy[k] = M.deepcopy(v)
    end
    return copy
end

return M
