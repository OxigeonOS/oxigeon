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

-- ─── Authoring: flat data in, a built Item out ───────────────────────────────
--
-- `Weapon{...}` is the hand-authoring front door and stays that way. It is also
-- a one-way function: `Item:new` plus `from_data` cannot be run backwards, so an
-- Item cannot be written back to a file. Everything OLC reads and writes is the
-- *flat authoring form* — the table `Weapon{...}` takes — and this is where that
-- form is turned into an object.

--- Every component name, in display order.
--- @return table  array of strings
function M.names()
    local out = {}
    for _, mod in ipairs(discover()) do out[#out + 1] = mod.component end
    return out
end

--- Each component's authoring schema, keyed by component name.
---
--- For `lib/schema.lua`, which flattens these into the item schema. Assembled
--- from the modules rather than held anywhere, so a new component file is
--- authorable the moment it exists.
--- @return table  { [component] = { fields, implicit, order, hand_written } }
function M.schemas()
    local out = {}
    for _, mod in ipairs(discover()) do
        if type(mod.fields) == "table" then
            out[mod.component] = {
                fields       = mod.fields,
                implicit     = mod.implicit == true,
                order        = mod.order or DEFAULT_ORDER,
                hand_written = mod.hand_written or {},
            }
        end
    end
    return out
end

--- Which components this flat authoring data asks for.
---
--- Everything named in `data.components`, plus every component declaring
--- `implicit` that finds one of its fields present.
---
--- **Explicit rather than inferred**, and that is the load-bearing decision.
--- Inferring from field presence would make `speed = 1.1` silently weaponise a
--- lantern, and clearing `damage` silently un-weapon a sword. `from_data` also
--- fills every default unconditionally, so it is not injective: without the
--- declaration there is no way to tell "no weapon component" from "a weapon with
--- every default".
--- @param data table
--- @return table  array of modules, in order
function M.claimed(data)
    if type(data) ~= "table" then return {} end

    local named = {}
    for _, name in ipairs(data.components or {}) do
        named[tostring(name)] = true
    end

    local out = {}
    for _, mod in ipairs(discover()) do
        local wanted = named[mod.component]
        if not wanted and mod.implicit and type(mod.fields) == "table" then
            for _, f in ipairs(mod.fields) do
                if data[f.name] ~= nil then wanted = true break end
            end
        end
        if wanted then out[#out + 1] = mod end
    end
    return out
end

--- Flat authoring data in, an Item carrying its components out.
---
--- Uses only `from_data`, which every component has. `new` and `apply` are the
--- hand-authoring doors and differ between components — `weapon` has `new`,
--- `drinkable` has `apply`, `requires` has neither. This is the *loader's* door
--- and has to work for all of them.
--- @param data table
--- @return table|nil item, string|nil err
function M.build(data)
    if type(data) ~= "table" then return nil, "component data must be a table" end
    if not data.id then return nil, "an item needs an id" end

    local claimed = M.claimed(data)

    -- Component defaults first, so `Item:new` sees the same input the archetype
    -- would have handed it. This is where `weapon.new`'s `slot = "weapon"` and
    -- `armor.new`'s `slot = "chest"` live now.
    local merged = {}
    for k, v in pairs(data) do merged[k] = v end
    for _, mod in ipairs(claimed) do
        for k, v in pairs(mod.item_defaults or {}) do
            if merged[k] == nil then merged[k] = v end
        end
    end

    local Item = require('lib.item')
    local ok, item = pcall(Item.new, Item, merged)
    if not ok then return nil, tostring(item) end

    for _, mod in ipairs(claimed) do
        local built_ok, built = pcall(mod.from_data, merged)
        if not built_ok then
            return nil, mod.component .. ".from_data raised: " .. tostring(built)
        end
        -- `requires.from_data` returns nil when nothing is required, and that
        -- nil is the answer: an empty table would be indistinguishable from a
        -- real constraint of zero.
        item[mod.component] = built

        -- ─── The hand-written fields, carried across ────────────────────────
        --
        -- A component's `hand_written` names are functions, so `from_data`
        -- cannot return them — and `Item:new` copies a *fixed list* of hooks
        -- (`on_use`, `on_pickup`, `on_drop`, `on_equip`, `on_remove`) that
        -- naturally does not know about them.
        --
        -- So `drinkable`'s `on_drink` reached an item only through the
        -- archetype path, where `drinkable.apply` assigns it to an
        -- already-built object. Authored as flat data — which is what a
        -- generated `items.lua` plus a `custom.lua` patch *is* — it was read
        -- off the data, merged correctly by `patch.apply`, and then silently
        -- dropped here. The potion was drinkable and did nothing.
        --
        -- Driven off `hand_written` rather than a second list of hook names,
        -- because a second list is the thing that rots: `to_data` already asks
        -- the component this same question in the opposite direction.
        for _, name in ipairs(mod.hand_written or {}) do
            if merged[name] ~= nil then item[name] = merged[name] end
        end
    end

    return item
end

--- A built Item back to flat authoring data, and everything that will not fit.
---
--- The return that matters is the second one. A field that cannot survive the
--- trip has to be *reported*, not dropped — dropping it silently is the bug
--- class this whole design exists to end, and `olc adopt` is built on this.
--- @param item table
--- @return table data, table lossy  lossy = { { path = "weapon.hit_message", why = "function" } }
function M.to_data(item)
    local data, lossy = {}, {}
    if type(item) ~= "table" then return data, lossy end

    local claimed = {}
    for _, mod in ipairs(discover()) do
        local ok, yes = pcall(mod.is, item)
        if ok and yes then claimed[#claimed + 1] = mod end
    end

    if #claimed > 0 then
        data.components = {}
        for _, mod in ipairs(claimed) do
            -- An implicit component is not named in `components`: it is
            -- re-derived from its own fields, and naming it too would emit a
            -- declaration the author never wrote.
            if not mod.implicit then
                data.components[#data.components + 1] = mod.component
            end
        end
        if #data.components == 0 then data.components = nil end
    end

    for _, mod in ipairs(claimed) do
        if type(mod.to_data) == "function" then
            local ok, flat = pcall(mod.to_data, item)
            if ok and type(flat) == "table" then
                for k, v in pairs(flat) do
                    if type(v) == "function" then
                        lossy[#lossy + 1] = { path = mod.component .. "." .. k, why = "function" }
                    else
                        data[k] = v
                    end
                end
            end
        end
        for _, name in ipairs(mod.hand_written or {}) do
            if item[name] ~= nil then
                lossy[#lossy + 1] = { path = name, why = "hand-written" }
            end
        end
    end

    return data, lossy
end

--- Drop the cache so a hot reload picks up a new or edited component.
--- Called from `mudlib/init.lua`'s `on_load`, next to `commands.flush_cache`.
function M.flush_cache()
    _list = nil
end

return M
