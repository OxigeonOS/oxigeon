-- mudlib/lib/requires.lua — The `requires` component and the one check over it.
--
-- Weapon and Armor each carried their own `meets_requirements`. The level and
-- strength tests were identical; Armor also tested dexterity, and Weapon did
-- not, for no reason anyone wrote down. Two copies of a rule is two rules, so
-- there is one of them now and every kind of item gets all three tests.
--
-- Component shape — absent fields are unconstrained:
--
--   item.requires = { level = 3, strength = 16, dexterity = 12 }
--
-- An item with no `requires` component is always usable, so a caller never has
-- to special-case its absence.

local M = {}

-- Ordered, so a character short on two counts is always told about the same one
-- first rather than whichever `pairs` happened to reach.
local CHECKS = {
    { key = "level",     label = "Level",     refusal = "Requires level %d"    },
    { key = "strength",  label = "Strength",  refusal = "Requires %d strength" },
    { key = "dexterity", label = "Dexterity", refusal = "Requires %d dexterity"},
}

--- Read one trait, preferring the effective value over the stored one.
--- Accepts an entity (anything answering `:trait(id)`), a plain stats table, or
--- an entity with a `.stats` table. The entity form is the correct one:
--- `entity.stats[id]` is what is *stored*, which for a buffed or derived trait
--- is the wrong answer. See CLAUDE.md.
--- @param source table   entity or stats table
--- @param id string      trait id
--- @return number
function M._read(source, id)
    if type(source) ~= "table" then return 0 end

    if type(source.trait) == "function" then
        local ok, v = pcall(source.trait, source, id)
        if ok and type(v) == "number" then return v end
    end

    if type(source[id]) == "number" then return source[id] end
    if type(source.stats) == "table" and type(source.stats[id]) == "number" then
        return source.stats[id]
    end
    return 0
end

--- Does this entity meet the item's requirements?
--- @param item table    anything that may carry a `requires` component
--- @param source table  entity or stats table to read the values from
--- @return boolean, string|nil  false and a player-facing reason when refused
function M.met(item, source)
    local req = type(item) == "table" and item.requires
    if type(req) ~= "table" then return true end

    for _, c in ipairs(CHECKS) do
        local needed = req[c.key]
        if type(needed) == "number" and M._read(source, c.key) < needed then
            return false, string.format(c.refusal, needed)
        end
    end
    return true
end

--- Build the component from flat authoring fields (`required_level`, ...).
--- Returns nil when nothing is required, so `item.requires` stays absent rather
--- than becoming an empty table indistinguishable from a real constraint.
--- @param data table
--- @return table|nil
function M.from_data(data)
    if type(data) ~= "table" then return nil end

    local req
    for _, c in ipairs(CHECKS) do
        local v = data["required_" .. c.key]
        if type(v) == "number" then
            req = req or {}
            req[c.key] = v
        end
    end
    return req
end

--- The `examine` line, or nil when there is nothing to say.
--- @param item table
--- @return string|nil
function M.describe(item)
    local req = type(item) == "table" and item.requires
    if type(req) ~= "table" then return nil end

    local parts = {}
    for _, c in ipairs(CHECKS) do
        if type(req[c.key]) == "number" then
            parts[#parts + 1] = c.label .. " " .. req[c.key]
        end
    end
    if #parts == 0 then return nil end
    return "Requires: " .. table.concat(parts, ", ")
end

return M
