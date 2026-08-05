-- mudlib/components/requires.lua — The `requires` component and the one check over it.
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

--- Component identity, for `components/init.lua`.
--- `component` is the field this owns on an item; `order` is where its
--- lines sort in `examine`.
M.component = "requires"
M.order = 90


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

--- Does the item carry a requirement at all?
---
--- Absence is still the real predicate — `M.met` answers `true` for an item
--- with no component, so nothing has to ask first. This exists because the
--- component index needs a uniform way to ask, and it is the same question.
--- @param item table
--- @return boolean
function M.is(item)
    return type(item) == "table" and type(item.requires) == "table"
end

--- The requirement, as a sentence.
--- @param item table
--- @return string|nil  nil when nothing is required
function M.summary(item)
    if not M.is(item) then return nil end
    local req = item.requires

    local parts = {}
    for _, c in ipairs(CHECKS) do
        if type(req[c.key]) == "number" then
            parts[#parts + 1] = c.label .. " " .. req[c.key]
        end
    end
    if #parts == 0 then return nil end
    return "Requires: " .. table.concat(parts, ", ")
end

--- The `examine` lines. Unindented — the caller owns layout.
---
--- Green when the viewer meets it, red when they do not — which is why this
--- takes a viewer at all. Without one it still says what is required, because
--- `examine` on a shop's stock is a fair question with no viewer in it.
--- @param item table
--- @param ctx table|nil  { viewer = table|nil }
--- @return table  array of strings
function M.describe(item, ctx)
    local summary = M.summary(item)
    if not summary then return {} end

    local viewer = type(ctx) == "table" and ctx.viewer or nil
    if not viewer then return { summary } end

    local met = M.met(item, viewer)
    return { (met and "{green}" or "{red}") .. summary .. "{/}" }
end

return M
