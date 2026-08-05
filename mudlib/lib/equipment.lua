-- mudlib/lib/equipment.lua — Wearing and wielding, and what that does to your
-- numbers.
--
-- `Mobile.equipment` was a `slot -> item` map that **nothing ever wrote**, and
-- the `armour` component's `defense`, `resist` and `stat_bonus` fields had no
-- reader anywhere. Combat ran the `damage_taken` pipeline faithfully and no
-- armour handler was ever registered in it, so armour never mitigated anything.
--
-- Equipping goes through the documented `equip:` source pattern rather than
-- through a second mechanism:
--
--     DAEMON.effect.set_source_effects(entity, "equip:chest", specs)
--
-- *The effects from this source are now exactly these.* Idempotent, so it is
-- safe on every login and every slot change without working out what it did
-- last time, and `persist = false` means nothing is ever written — the aura is
-- rebuilt from what is worn, which is the only copy that can be wrong.
--
-- ─── Why the effect definitions are generated ────────────────────────────────
--
-- An effect's hooks are fixed when it is *defined*, and a trait modifier is a
-- `trait:<id>` hook. A single "equipment aura" definition therefore cannot
-- modify strength on one character and wisdom on another — the hook name is
-- part of the definition, not of the instance.
--
-- So one definition is generated per trait any gear actually touches,
-- `equip_trait_<id>`, with the amount carried per instance in `state`. The
-- population is bounded by how many distinct traits the game's gear modifies,
-- which is small, and `effect_d.define` already handles being called at runtime
-- (it bumps the generation and invalidates every memo).
--
-- Protection is different and needs only one definition: `damage_taken` is a
-- single hook whatever the damage type, so `equip_protection` reads the worn
-- piece's defence and resist table out of its own instance state.

local Armor      = require('components.armor')
local Weapon     = require('components.weapon')
local Container  = require('components.container')
local Requires   = require('components.requires')
local Components = require('components')

local M = {}

--- The slots this mudlib knows about, in the order `equipment` lists them.
--- A game may use fewer; using more means adding one here, which is the point
--- of it being a mudlib list rather than a game one.
M.SLOTS = {
    "head", "neck", "chest", "back", "hands", "waist",
    "legs", "feet", "weapon", "offhand", "light", "ring",
}

local SLOT_SET = {}
for _, slot in ipairs(M.SLOTS) do SLOT_SET[slot] = true end

--- Slots a two-handed weapon occupies. Wielding one has to clear the offhand,
--- or a shield keeps working while both hands are on a greatsword.
M.TWO_HANDED_SLOTS = { "weapon", "offhand" }

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

--- @param slot any
--- @return boolean
function M.is_slot(slot)
    return type(slot) == "string" and SLOT_SET[slot] == true
end

-- ─── Generated effect definitions ────────────────────────────────────────────

--- The definition id for the aura that modifies one trait.
local function trait_def_id(trait_id)
    return "equip_trait_" .. trait_id
end

--- Make sure an effect exists that adds `state.amount` to one trait.
---
--- Defined on demand rather than up front, because the mudlib does not know
--- which traits a game's gear will touch and enumerating every defined trait
--- would create hundreds of effect definitions nobody uses.
--- @param trait_id string
--- @return string|nil  the definition id, or nil if it could not be defined
local function ensure_trait_effect(trait_id)
    if not (DAEMON and DAEMON.effect) then return nil end
    local id = trait_def_id(trait_id)
    if DAEMON.effect.get_def(id) then return id end

    -- A gauge or a counter cannot be modified — `effect_d.define` refuses a
    -- `trait:hp` hook by name, at registration time. Checking here turns that
    -- refusal into a message naming the item's field rather than a warning
    -- about a definition the author never wrote.
    if DAEMON.trait and DAEMON.trait.get_def then
        local def = DAEMON.trait.get_def(trait_id)
        if def and (def.kind == "gauge" or def.kind == "counter") then
            log_error("EQUIPMENT: a stat_bonus cannot target '" .. trait_id
                .. "', which is a " .. def.kind
                .. " — raise the trait that is its max instead")
            return nil
        end
    end

    local ok = DAEMON.effect.define({
        id      = id,
        label   = "Equipment",
        -- Never written. The aura is derived from what is worn, and what is
        -- worn is saved; persisting the aura as well would be a second copy
        -- that can disagree with the first.
        persist = false,
        hooks   = {
            [trait_id] = {
                hook  = "trait:" .. trait_id,
                phase = "add",
                fn    = function(ev, ctx)
                    local amount = ctx.inst.state and ctx.inst.state.amount
                    if type(amount) == "number" then
                        ev.amount = (ev.amount or 0) + amount
                    end
                end,
            },
        },
    })
    return ok and id or nil
end

--- The one definition that turns worn armour into real mitigation (G4).
---
--- `reduce` phase, so a percentage multiplier applied by something else lands
--- first — which is the ordering `effects.md` argues for and the reason a
--- 30-point hit against stoneskin yields the documented 20 and not 21.
local function ensure_protection_effect()
    if not (DAEMON and DAEMON.effect) then return nil end
    if DAEMON.effect.get_def("equip_protection") then return "equip_protection" end

    local ok = DAEMON.effect.define({
        id      = "equip_protection",
        label   = "Armour",
        persist = false,
        hooks   = {
            damage_taken = {
                hook  = "damage_taken",
                phase = "reduce",
                fn    = function(ev, ctx)
                    local state = ctx.inst.state
                    if type(state) ~= "table" then return end

                    local reduction = tonumber(state.defense) or 0
                    -- Resist is looked up by the damage type on the event, so
                    -- a warded cloak blunts the wisp and does nothing at all
                    -- against a sword. A negative entry is a weakness and
                    -- increases the number, which is the same arithmetic.
                    local by_type = type(state.resist) == "table"
                        and tonumber(state.resist[ev.damage_type or "physical"]) or nil
                    if by_type then reduction = reduction + by_type end

                    ev.amount = (ev.amount or 0) - reduction
                end,
            },
        },
    })
    return ok and "equip_protection" or nil
end

-- ─── What one worn piece contributes ─────────────────────────────────────────

--- The effect specs a single equipped item produces.
--- @param item table  the resolved item
--- @return table  array of `set_source_effects` specs
local function specs_for(item)
    local specs = {}
    if type(item) ~= "table" then return specs end

    -- Whatever the item's components contribute. Nothing here names a
    -- component: armour's mitigation lives in `components/armor.lua` where the
    -- rest of armour does, and a component added later is picked up by
    -- existing. The two effect definitions stay here because they are
    -- equipment's to create, so they are handed over rather than reached for.
    for _, spec in ipairs(Components.equip_specs(item, {
        trait_effect      = ensure_trait_effect,
        protection_effect = ensure_protection_effect,
    })) do
        specs[#specs + 1] = spec
    end

    -- A weapon may carry stat bonuses too — a sword of strength is an ordinary
    -- thing and needs no separate mechanism.
    if type(item.stat_bonus) == "table" then
        for trait_id, amount in pairs(item.stat_bonus) do
            if type(amount) == "number" and amount ~= 0 then
                local def_id = ensure_trait_effect(trait_id)
                if def_id then
                    specs[#specs + 1] = { def = def_id, state = { amount = amount } }
                end
            end
        end
    end

    return specs
end

--- Rebuild one slot's aura from what is in it.
---
--- Cleared first and then set, in two calls. `set_source_effects` matches by
--- definition id, so swapping one chest piece for another that modifies the
--- same trait would otherwise see "an instance of `equip_trait_strength` from
--- `equip:chest` already exists" and keep the *old* item's amount.
--- @param entity table
--- @param slot string
--- @param item table|nil  nil clears the slot's aura
function M.refresh_slot(entity, slot, item)
    if not (DAEMON and DAEMON.effect) then return end
    local source = "equip:" .. slot

    local ok, err = pcall(DAEMON.effect.set_source_effects, entity, source, {})
    if not ok then
        log_error("EQUIPMENT: clearing '" .. source .. "' failed: " .. tostring(err))
        return
    end
    if not item then return end

    local specs = specs_for(item)
    if #specs == 0 then return end

    local sok, serr = pcall(DAEMON.effect.set_source_effects, entity, source, specs)
    if not sok then
        log_error("EQUIPMENT: applying '" .. source .. "' failed: " .. tostring(serr))
    end
end

--- Rebuild every slot's aura from scratch.
---
--- Called on login, because `equip:` effects are `persist = false` and so are
--- gone by design — what is worn is saved, and the aura is derived from it.
--- @param entity table
function M.refresh_all(entity)
    if type(entity) ~= "table" or type(entity.equipment) ~= "table" then return end
    for _, slot in ipairs(M.SLOTS) do
        local entry = entity.equipment[slot]
        local item = entry and DAEMON and DAEMON.items and DAEMON.items.resolve(entry)
        M.refresh_slot(entity, slot, item)
    end
end

-- ─── The two verbs, as one operation each ────────────────────────────────────

--- Which slot an item goes in, and what else it displaces.
--- @param item table
--- @return string|nil slot, table|nil also_occupies
function M.slot_for(item)
    if type(item) ~= "table" then return nil end
    local slot = item.slot
    if not M.is_slot(slot) then return nil end
    if slot == "weapon" and Weapon.is(item) and item.weapon.two_handed then
        return slot, { "offhand" }
    end
    return slot, nil
end

--- Put an item on.
---
--- Returns rather than sends: the caller knows whether it is `wear` or `wield`
--- and what the room should be told, and a library that wrote to a socket would
--- be untestable without one.
--- @param entity table
--- @param entry table   the instance
--- @param item table    the resolved item
--- @return boolean ok, string|nil why, table|nil displaced  array of entries removed
function M.equip(entity, entry, item)
    local slot, also = M.slot_for(item)
    if not slot then
        return false, "You cannot wear or wield that."
    end
    if not item.equippable then
        return false, "You cannot wear or wield that."
    end

    -- The one refusal path, for level, strength and dexterity alike. Read
    -- through the entity rather than its stored stats, so a strength buff
    -- genuinely lets you lift the greatsword.
    local met, why = Requires.met(item, entity)
    if not met then return false, why end

    entity.equipment = entity.equipment or {}

    -- Anything already in the way comes off first, including the offhand when
    -- a two-handed weapon goes on.
    local displaced = {}
    local to_clear = { slot }
    for _, extra in ipairs(also or {}) do to_clear[#to_clear + 1] = extra end
    -- A shield in the offhand also blocks a two-handed weapon in reverse: if
    -- the weapon slot holds one, filling the offhand has to clear it.
    if slot == "offhand" then
        local held = entity.equipment.weapon
        local held_item = held and DAEMON and DAEMON.items and DAEMON.items.resolve(held)
        if held_item and Weapon.is(held_item) and held_item.weapon.two_handed then
            to_clear[#to_clear + 1] = "weapon"
        end
    end

    for _, s in ipairs(to_clear) do
        local occupant = entity.equipment[s]
        if occupant then
            local removed, rwhy = M.unequip(entity, s)
            if not removed then return false, rwhy end
            displaced[#displaced + 1] = occupant
        end
    end

    -- Out of the inventory array. An item cannot be both worn and loose in the
    -- pack: leaving it in both meant `inventory` listed what you were wearing,
    -- and — worse — that `drop sword` could put a wielded sword on the floor
    -- while `equipment` went on reporting it in your hand.
    if type(entity.inventory) == "table" then
        for i, e in ipairs(entity.inventory) do
            if e == entry then
                table.remove(entity.inventory, i)
                break
            end
        end
    end

    entity.equipment[slot] = entry
    M.refresh_slot(entity, slot, item)

    local _, hook_ran = pcall(function()
        return require('lib.carry').fire_hook(item, "on_equip", entity.char_id)
    end)
    local _ = hook_ran

    if DAEMON and DAEMON.event then
        pcall(DAEMON.event.emit, "item.equipped", {
            char_id = entity.char_id, slot = slot,
            instance_id = type(entry) == "table" and entry.id,
            template_id = type(entry) == "table" and entry.template,
        })
    end

    return true, nil, displaced
end

--- Take an item off and put it back in the inventory.
--- @param entity table
--- @param slot string
--- @return boolean ok, string|nil why, table|nil entry
function M.unequip(entity, slot)
    if type(entity.equipment) ~= "table" then return false, "You are not wearing that." end
    local entry = entity.equipment[slot]
    if not entry then return false, "You are not wearing anything there." end

    local item = DAEMON and DAEMON.items and DAEMON.items.resolve(entry)

    -- A cursed item would refuse here. Nothing does yet; the hook is where it
    -- would go, and it goes before the state changes so a refusal leaves
    -- nothing half-done.
    entity.equipment[slot] = nil
    M.refresh_slot(entity, slot, nil)

    entity.inventory = entity.inventory or {}
    entity.inventory[#entity.inventory + 1] = entry

    if item then
        require('lib.carry').fire_hook(item, "on_remove", entity.char_id)
    end
    if DAEMON and DAEMON.event then
        pcall(DAEMON.event.emit, "item.unequipped", {
            char_id = entity.char_id, slot = slot,
            instance_id = type(entry) == "table" and entry.id,
            template_id = type(entry) == "table" and entry.template,
        })
    end

    return true, nil, entry
end

--- The equipped item in one slot, resolved.
--- @return table|nil entry, table|nil item
function M.worn(entity, slot)
    local entry = type(entity.equipment) == "table" and entity.equipment[slot]
    if not entry then return nil, nil end
    return entry, DAEMON and DAEMON.items and DAEMON.items.resolve(entry)
end

--- Every occupied slot, in `M.SLOTS` order.
--- @return table  array of { slot, entry, item }
function M.all_worn(entity)
    local out = {}
    for _, slot in ipairs(M.SLOTS) do
        local entry, item = M.worn(entity, slot)
        if entry then out[#out + 1] = { slot = slot, entry = entry, item = item } end
    end
    return out
end

--- Total encumbrance from worn armour, for whatever a game wants to do with it.
--- @return number
function M.encumbrance(entity)
    local total = 0
    for _, worn in ipairs(M.all_worn(entity)) do
        total = total + (Armor.encumbrance(worn.item) or 0)
    end
    return total
end

-- Re-exported so a command needing "is this a container" has one import.
M.Container = Container

return M
