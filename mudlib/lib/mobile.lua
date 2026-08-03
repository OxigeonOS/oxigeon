-- mudlib/lib/mobile.lua — Mobile (NPC/Monster) base class
-- Inherits from Object. Represents any living non-player entity:
-- monsters, NPCs, shopkeepers, quest givers, wandering creatures.
--
-- Mobiles have stats, inventory, AI behaviors, and can participate
-- in the event system (echoes, patrols, combat reactions).

local Object = require('lib.object')

--- @class Mobile : Object
--- @field stats table
--- @field on_death function
--- @field echoes table
--- @field inventory table
--- @field tags table
--- @field aggressive boolean
--- @field dialogue table
--- @field skills table
--- @field race string
--- @field faction string
local Mobile = setmetatable({}, { __index = Object })
Mobile.__index = Mobile

--- Create a new Mobile from a data table.
--- @override
--- @param data table  Mobile definition
--- @return table      The new Mobile
function Mobile:new(data)
    local obj = Object.new(self, data)

    -- ─── Stats ───────────────────────────────────────────────────────────────
    -- Defaults first, then everything the caller actually gave us.
    --
    -- This used to be a fixed list of nine keys, which meant any other stat was
    -- silently dropped here on load even though `to_save` had faithfully
    -- written it — a trait named `wisdom` would vanish on every login. TRAIT_D
    -- owns the set of stats now, so the whitelist has to go.
    local stats = data.stats or {}
    obj.stats = {
        hp = 10, max_hp = 10, mp = 0, max_mp = 0,
        strength = 5, dexterity = 5, intelligence = 5, constitution = 5,
        level = 1,
    }
    for key, value in pairs(stats) do
        obj.stats[key] = value
    end
    -- The two that used to derive from each other when only one was given.
    if stats.hp and not stats.max_hp then obj.stats.max_hp = stats.hp end
    if stats.mp and not stats.max_mp then obj.stats.max_mp = stats.mp end

    -- ─── Identity ────────────────────────────────────────────────────────────
    obj.faction    = data.faction             -- For ally/enemy detection
    obj.race       = data.race               -- "human", "orc", "undead", etc.
    obj.gender     = data.gender             -- "male", "female", "neutral"
    obj.title      = data.title              -- Optional title ("the Blacksmith")

    -- ─── Inventory ───────────────────────────────────────────────────────────
    obj.inventory  = data.inventory or {}    -- Array of item instance tables: { template = "id", ... }
    obj.equipment  = data.equipment or {}    -- slot → item_id mapping

    -- ─── Behavior ────────────────────────────────────────────────────────────
    obj.aggressive = data.aggressive or false -- Attacks players on sight
    obj.stationary = data.stationary or false -- Never wanders from spawn room
    obj.unique     = data.unique or false     -- Only one can exist at a time

    -- ─── Echoes (atmospheric messages) ───────────────────────────────────────
    -- Array of { text = "...", weight = N } or just strings.
    -- Displayed periodically via TICKER_D.
    obj.echoes         = data.echoes or {}
    obj.echo_interval  = data.echo_interval or 30  -- Seconds between echo rolls

    -- ─── Patrol ──────────────────────────────────────────────────────────────
    -- Array of room_ids. Mobile walks this route in order.
    obj.patrol         = data.patrol               -- nil = no patrol
    obj.patrol_interval = data.patrol_interval or 15 -- Seconds between moves

    -- ─── Loot ────────────────────────────────────────────────────────────────
    -- Array of { item_id = "...", chance = 0.0-1.0 }
    obj.loot_table     = data.loot_table or {}

    -- ─── Respawn ─────────────────────────────────────────────────────────────
    obj.respawn_time   = data.respawn_time         -- nil = no respawn
    obj.spawn_room     = data.spawn_room           -- Room to respawn in

    -- ─── Lfun hooks ──────────────────────────────────────────────────────────
    obj.on_death    = data.on_death           -- function(mob, killer_id)
    obj.on_combat   = data.on_combat          -- function(mob, target_id)
    obj.on_interact = data.on_interact        -- function(mob, user_id, verb)
    obj.on_spawn    = data.on_spawn           -- function(mob, room_id)

    -- ─── Dialogue ────────────────────────────────────────────────────────────
    -- Table of keyword → response (string or function)
    obj.dialogue = data.dialogue or {}

    -- Skills
    obj.skills = data.skills or {}            -- skill_name → level

    -- Tags for categorization
    obj.tags = data.tags or {}                -- e.g. {"boss", "quest", "merchant"}

    return obj
end

-- ─── Stats helpers ───────────────────────────────────────────────────────────

--- The effective value of a stat, after traits and effects have their say.
---
--- `self.stats[id]` is what is *stored*; this is what is *true*. A ring of
--- strength does not change the stored number — it is an effect, and only this
--- accessor knows about it. Falls back to the raw field when TRAIT_D is not
--- loaded, so a bare Mobile still works.
--- @param id string
--- @return number
function Mobile:stat(id)
    if DAEMON and DAEMON.trait and DAEMON.trait.get_def and DAEMON.trait.get_def(id) then
        return DAEMON.trait.value(self, id)
    end
    local raw = self.stats and self.stats[id]
    return type(raw) == "number" and raw or 0
end

--- Check if the mobile is alive.
-- @return boolean
function Mobile:is_alive()
    return self:stat("hp") > 0
end

--- Run a number past this entity's effects, if there are any.
local function through_effects(entity, hook, amount, opts)
    if not (DAEMON and DAEMON.effect and DAEMON.effect.run) then
        return amount, nil
    end
    local ev = { amount = amount, scale = 0, min = 0 }
    if opts then
        for k, v in pairs(opts) do
            if ev[k] == nil then ev[k] = v end
        end
    end
    local ok, result = pcall(DAEMON.effect.run, entity, hook, ev)
    if not ok then
        log("error", "MOBILE: the '" .. hook .. "' pipeline failed: " .. tostring(result))
        return amount, nil
    end
    if result.cancelled then return 0, result.reason end
    return math.max(0, math.floor(result.amount or amount)), nil
end

--- Apply damage, after every effect on this entity has had its say.
---
--- This is where "take 15% less damage" and "negate 5 per hit" actually happen.
--- The order they apply in is decided by their phases, not by which landed
--- first — see mudlib/lib/effects.lua.
--- @param amount number  Damage before mitigation
--- @param opts table|nil { damage_type = "fire", attacker = <entity> }
--- @return number remaining HP
--- @return number the damage actually dealt
function Mobile:take_damage(amount, opts)
    local was_alive = self:stat("hp") > 0
    local dealt, reason = through_effects(self, "damage_taken", amount, opts)

    if reason and self.send then
        pcall(self.send, self, reason .. "\r\n")
    end

    if DAEMON and DAEMON.trait and DAEMON.trait.get_def and DAEMON.trait.get_def("hp") then
        DAEMON.trait.adjust(self, "hp", -dealt)
    else
        self.stats.hp = math.max(0, self.stats.hp - dealt)
    end

    -- Fire death hook when transitioning from alive to dead
    if was_alive and self:stat("hp") <= 0 and self.on_death then
        local ok, err = pcall(self.on_death, self)
        if not ok then
            log("error", "MOBILE: on_death hook failed: " .. tostring(err))
        end
    end

    return self:stat("hp"), dealt
end

--- Restore health, after every effect has had its say. Clamps to the maximum.
--- @param amount number
--- @param opts table|nil { source = "effect:regeneration" }
--- @return number new HP
--- @return number the healing actually applied
function Mobile:heal(amount, opts)
    local healed = through_effects(self, "heal_received", amount, opts)

    if DAEMON and DAEMON.trait and DAEMON.trait.get_def and DAEMON.trait.get_def("hp") then
        DAEMON.trait.adjust(self, "hp", healed)
    else
        self.stats.hp = math.min(self.stats.max_hp, self.stats.hp + healed)
    end
    return self:stat("hp"), healed
end

--- Get the mob's effective level.
-- @return number
function Mobile:get_level()
    return self:stat("level")
end

-- ─── Echo helpers ────────────────────────────────────────────────────────────

--- Pick a random echo from the echoes list, weighted by probability.
-- @return string|nil  The chosen echo text, or nil if no echoes
function Mobile:roll_echo()
    local echoes = self.echoes
    if not echoes or #echoes == 0 then return nil end

    -- Support both string arrays and { text, weight } tables
    local total_weight = 0
    local entries = {}
    for _, e in ipairs(echoes) do
        if type(e) == "string" then
            entries[#entries + 1] = { text = e, weight = 1 }
            total_weight = total_weight + 1
        elseif type(e) == "table" then
            local w = e.weight or 1
            entries[#entries + 1] = { text = e.text, weight = w }
            total_weight = total_weight + w
        end
    end

    if total_weight == 0 then return nil end

    local roll = math.random() * total_weight
    local cumulative = 0
    for _, entry in ipairs(entries) do
        cumulative = cumulative + entry.weight
        if roll <= cumulative then
            return Object.resolve(entry.text, self)
        end
    end

    return nil
end

-- ─── Inventory helpers ───────────────────────────────────────────────────────

--- Check if the mobile has an item by template ID.
-- Supports both instance tables and legacy string entries.
-- @param template_id string  The template ID to search for
-- @return boolean
function Mobile:has_item(template_id)
    for _, entry in ipairs(self.inventory) do
        local tmpl = type(entry) == "table" and entry.template or entry
        if tmpl == template_id then return true end
    end
    return false
end

--- Add a pristine item to the mobile's inventory by template ID.
-- @param template_id string
function Mobile:add_item(template_id)
    self.inventory[#self.inventory + 1] = { template = template_id }
end

--- Add an item instance table directly to inventory.
-- For items with per-instance overrides (enchantments, custom names, etc.).
-- @param instance table  Must have at least { template = "id" }
function Mobile:add_item_instance(instance)
    if type(instance) ~= "table" or not instance.template then
        log("warn", "MOBILE: add_item_instance called with invalid instance")
        return
    end
    self.inventory[#self.inventory + 1] = instance
end

--- Find the first inventory entry matching a template ID.
-- Returns the entry table and its index for direct modification.
-- @param template_id string
-- @return table|nil, number|nil  The inventory entry and its index
function Mobile:find_item(template_id)
    for i, entry in ipairs(self.inventory) do
        local tmpl = type(entry) == "table" and entry.template or entry
        if tmpl == template_id then
            return entry, i
        end
    end
    return nil, nil
end

--- Remove the first item matching a template ID from inventory.
-- @param template_id string
-- @return boolean  true if the item was found and removed
function Mobile:remove_item(template_id)
    for i, entry in ipairs(self.inventory) do
        local tmpl = type(entry) == "table" and entry.template or entry
        if tmpl == template_id then
            table.remove(self.inventory, i)
            return true
        end
    end
    return false
end

-- ─── Query methods ───────────────────────────────────────────────────────────

--- Check if this mobile has a specific tag.
-- @param tag string
-- @return boolean
function Mobile:has_tag(tag)
    for _, t in ipairs(self.tags) do
        if t == tag then return true end
    end
    return false
end

--- Check if this mobile is aggressive toward a target.
-- @return boolean
function Mobile:is_aggressive()
    return self.aggressive == true
end

--- Get dialogue response for a keyword.
-- @param keyword string
-- @return string|nil  The response text, or nil
function Mobile:get_dialogue(keyword)
    local response = self.dialogue[keyword]
    if response then
        return Object.resolve(response, self)
    end
    return nil
end

--- Get a skill level.
-- @param skill string  The skill name
-- @return number       Skill level (0 if not learned)
function Mobile:get_skill(skill)
    return self.skills[skill] or 0
end

--- Set a skill level.
-- @param skill string  The skill name
-- @param level number  The new level
function Mobile:set_skill(skill, level)
    self.skills[skill] = level
end

--- Get the full examination text.
-- @return string
function Mobile:examine()
    local parts = {}
    local resolve = Object.resolve

    parts[#parts + 1] = resolve(self.short, self) or "Something"
    parts[#parts + 1] = resolve(self.description, self) or "You see nothing special."

    if self.race then
        parts[#parts + 1] = "Race: " .. self.race
    end
    if self.faction then
        parts[#parts + 1] = "Faction: " .. self.faction
    end
    if self.stats.level > 1 then
        parts[#parts + 1] = "Level: " .. self.stats.level
    end

    return table.concat(parts, "\r\n") .. "\r\n"
end

return Mobile
