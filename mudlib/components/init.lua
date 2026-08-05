-- mudlib/components/init.lua — The component index.
--
-- `cmds/examine.lua` has claimed since it was written that "a new component
-- describes itself by existing rather than by editing this file", and then
-- hard-coded four `describe` calls. This is what makes that true.
--
-- Discovery is the same shape as `lib/commands.lua`: `list_dir` over this
-- directory, `require` each module, keep the ones that look like a component.
-- A component is any module in here exposing `component` (its name) and `is`.
--
-- Order is declared, not discovered — `M.order` on each module — because the
-- filesystem has no opinion about whether a weapon's damage should print above
-- or below its strength requirement, and the answer should not change when
-- somebody renames a file.
--
--   local components = require('components')
--   for _, line in ipairs(components.describe(item, ctx)) do ... end
--
-- See docs/src/lua-api/components.md.

local M = {}

--- Where a component with no stated opinion sorts.
local DEFAULT_ORDER = 50

local _list = nil

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then
        pcall(DAEMON.journal.error, message)
    end
end

--- Load every component module in this directory, ordered.
--- @return table  array of modules
local function discover()
    if _list then return _list end
    _list = {}

    if type(list_dir) ~= "function" then
        log_error("COMPONENTS: list_dir is unavailable; no components loaded")
        return _list
    end

    local ok, entries = pcall(list_dir, "components")
    if not ok or type(entries) ~= "table" then
        log_error("COMPONENTS: list_dir('components') failed: " .. tostring(entries))
        return _list
    end

    for _, entry in ipairs(entries) do
        local file = type(entry) == "table" and entry.name
        local is_dir = type(entry) == "table" and entry.is_dir
        local name = (not is_dir) and file and file:match("^(.+)%.lua$")
        -- `init` is this file. Requiring it from inside itself would recurse.
        if name and name ~= "init" then
            local rok, mod = pcall(require, "components." .. name)
            if not rok then
                log_error("COMPONENTS: failed to load '" .. name .. "': " .. tostring(mod))
            elseif type(mod) == "table" and type(mod.is) == "function" then
                -- `component` names the field on the item, which is not always
                -- the file name: `armor.lua` owns `item.armour`.
                mod.component = mod.component or name
                _list[#_list + 1] = mod
            end
        end
    end

    table.sort(_list, function(a, b)
        local ao, bo = a.order or DEFAULT_ORDER, b.order or DEFAULT_ORDER
        if ao ~= bo then return ao < bo end
        return tostring(a.component) < tostring(b.component)
    end)

    return _list
end

--- Every loaded component module, in display order.
--- @return table  array of modules
function M.all()
    return discover()
end

--- One component module by name, or nil.
--- @param name string
--- @return table|nil
function M.get(name)
    for _, mod in ipairs(discover()) do
        if mod.component == name then return mod end
    end
    return nil
end

--- Which components this item has.
--- @param item table
--- @return table  array of modules
function M.on(item)
    local out = {}
    if type(item) ~= "table" then return out end
    for _, mod in ipairs(discover()) do
        local ok, yes = pcall(mod.is, item)
        if ok and yes then out[#out + 1] = mod end
    end
    return out
end

--- Every line every component on this item has to say about itself.
---
--- `ctx` carries what a component may need beyond the item itself and is
--- passed through untouched: `instance_id` for anything reading per-instance
--- object state (a container's open/locked flags), and `viewer` for anything
--- whose answer depends on who is asking (whether you meet a requirement).
--- @param item table
--- @param ctx table|nil  { instance_id = string|nil, viewer = table|nil }
--- @return table  array of strings
function M.describe(item, ctx)
    local lines = {}
    if type(item) ~= "table" then return lines end
    ctx = ctx or {}

    for _, mod in ipairs(discover()) do
        if type(mod.describe) == "function" then
            -- A component that raises must not stop the others being read.
            local ok, said = pcall(mod.describe, item, ctx)
            if not ok then
                log_error("COMPONENTS: " .. tostring(mod.component)
                    .. ".describe raised: " .. tostring(said))
            elseif type(said) == "table" then
                for _, line in ipairs(said) do lines[#lines + 1] = line end
            elseif type(said) == "string" then
                lines[#lines + 1] = said
            end
        end
    end
    return lines
end

--- The effect specs every component on this item contributes when it is worn.
---
--- `ctx` must supply the two factories, because the effect definitions are
--- `lib/equipment.lua`'s to create and a component must not reach back into it:
---   ctx.trait_effect(trait_id) -> def_id|nil
---   ctx.protection_effect()    -> def_id|nil
--- @param item table
--- @param ctx table
--- @return table  array of `set_source_effects` specs
function M.equip_specs(item, ctx)
    local specs = {}
    if type(item) ~= "table" then return specs end

    for _, mod in ipairs(discover()) do
        if type(mod.equip_specs) == "function" then
            local ok, said = pcall(mod.equip_specs, item, ctx)
            if not ok then
                log_error("COMPONENTS: " .. tostring(mod.component)
                    .. ".equip_specs raised: " .. tostring(said))
            elseif type(said) == "table" then
                for _, spec in ipairs(said) do specs[#specs + 1] = spec end
            end
        end
    end
    return specs
end

--- Drop the cache so a hot reload picks up a new or edited component.
--- Called from `mudlib/init.lua`'s `on_load`, next to `commands.flush_cache`.
function M.flush_cache()
    _list = nil
end

return M
