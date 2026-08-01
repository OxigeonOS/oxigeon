-- mudlib/lib/tables.lua — Table manipulation utilities

local M = {}

--- Check if a value is in a sequential table
function M.contains(t, val)
    for _, v in ipairs(t) do
        if v == val then return true end
    end
    return false
end

--- Deep copy a table (does not copy metatables)
function M.deepcopy(t)
    if type(t) ~= "table" then return t end
    local copy = {}
    for k, v in pairs(t) do
        copy[k] = M.deepcopy(v)
    end
    return copy
end

return M
