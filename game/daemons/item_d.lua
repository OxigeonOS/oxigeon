-- game/daemons/item_d.lua — Item Registry Daemon
-- Central registry mapping item IDs to their Item object definitions.
-- When game code needs to know what "purple_potion" is, it asks ITEM_D.
--
-- Usage:
--   DAEMON.items.register(item_obj)           -- register an Item object
--   DAEMON.items.get("purple_potion")         -- retrieve by ID
--   DAEMON.items.find_by_name("purple", inv)  -- search inventory by name

local M = {}

-- Registry: item_id → Item object
M._items = {}

--- Helper: log errors to both log() and journal_d.
local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then
        DAEMON.journal.error(message)
    end
end

--- Register an Item definition in the registry.
-- @param item table  An Item object (must have item.id)
function M.register(item)
    if not item or not item.id then
        log_error("ITEM_D: Attempted to register item without an id")
        return
    end
    M._items[item.id] = item
    log("debug", "ITEM_D: Registered item '" .. item.id .. "'")
end

--- Register multiple items at once.
-- @param items table  Array of Item objects
function M.register_all(items)
    for _, item in ipairs(items) do
        M.register(item)
    end
end

--- Get an Item definition by ID.
-- @param item_id string  The item ID
-- @return table|nil      The Item object, or nil
function M.get(item_id)
    return M._items[item_id]
end

--- Resolve an inventory entry (instance table) against its template.
-- Merges instance overrides onto the template, producing a full item view.
-- For pristine items (no overrides), returns the template directly.
-- @param entry table  An inventory entry: { template = "id", ... }
-- @return table|nil   The resolved Item (template + overrides), or nil
function M.resolve(entry)
    if type(entry) == "string" then
        -- Legacy string format
        return M._items[entry]
    end
    if type(entry) ~= "table" or not entry.template then
        return nil
    end
    local template = M._items[entry.template]
    if not template then return nil end

    -- Check for any overrides beyond the "template" key
    local has_overrides = false
    for k, _ in pairs(entry) do
        if k ~= "template" then
            has_overrides = true
            break
        end
    end

    -- Pristine item — just return the template
    if not has_overrides then return template end

    -- Modified item — shallow overlay: instance fields win
    local resolved = {}
    for k, v in pairs(template) do resolved[k] = v end
    for k, v in pairs(entry) do
        if k ~= "template" then
            resolved[k] = v
        end
    end
    -- Preserve Item metatable for method access
    setmetatable(resolved, getmetatable(template))
    return resolved
end

--- Search a player's inventory for an item matching a name string.
-- Matches against item.short and item.id using case-insensitive substring matching.
-- Supports both instance tables and legacy string entries.
-- @param name      string  The search string (e.g. "purple potion")
-- @param inventory table   Array of inventory entries
-- @return string|nil, table|nil  The template_id and resolved Item object, or nil
function M.find_by_name(name, inventory)
    if not name or not inventory then return nil, nil end
    name = name:lower()

    for _, entry in ipairs(inventory) do
        -- Extract template ID from entry
        local template_id
        if type(entry) == "string" then
            template_id = entry
        elseif type(entry) == "table" then
            template_id = entry.template
        end

        if template_id then
            local item = M.resolve(entry)
            if item then
                -- Match against short description (resolved, may have instance override)
                local short = item.short
                if type(short) == "string" and short:lower():find(name, 1, true) then
                    return template_id, item
                end
                -- Match against template ID (e.g. "purple_potion" matches "purple")
                if template_id:lower():gsub("_", " "):find(name, 1, true) then
                    return template_id, item
                end
            end
        end
    end
    return nil, nil
end

--- Get all registered items.
-- @return table  item_id → Item mapping
function M.all()
    return M._items
end

return M
