-- mudlib/lib/mobile.lua — Mobile (NPC/Monster) base class
-- Inherits from Object. Represents any living non-player entity:
-- monsters, NPCs, shopkeepers, quest givers, wandering creatures.
--
-- Mobiles have stats, inventory, AI behaviors, and can participate
-- in the event system (echoes, patrols, combat reactions).

local Object = require('lib.object')

local Mobile = setmetatable({}, { __index = Object })
Mobile.__index = Mobile

--- Create a new Mobile from a data table.
-- @param data table  Mobile definition
-- @return table      The new Mobile
function Mobile:new(data)
    local obj = Object.new(self, data)

    -- ─── Stats ───────────────────────────────────────────────────────────────
    local stats = data.stats or {}
    obj.stats = {
        hp         = stats.hp or 10,
        max_hp     = stats.max_hp or stats.hp or 10,
        mp         = stats.mp or 0,
        max_mp     = stats.max_mp or stats.mp or 0,
        strength   = stats.strength or 5,
        dexterity  = stats.dexterity or 5,
        intelligence = stats.intelligence or 5,
        constitution = stats.constitution or 5,
        level      = stats.level or 1,
    }

    -- ─── Identity ────────────────────────────────────────────────────────────
    obj.faction    = data.faction             -- For ally/enemy detection
    obj.race       = data.race               -- "human", "orc", "undead", etc.
    obj.gender     = data.gender             -- "male", "female", "neutral"
    obj.title      = data.title              -- Optional title ("the Blacksmith")

    -- ─── Inventory ───────────────────────────────────────────────────────────
    obj.inventory  = data.inventory or {}    -- Array of item IDs
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

--- Check if the mobile is alive.
-- @return boolean
function Mobile:is_alive()
    return self.stats.hp > 0
end

--- Apply damage to the mobile. Clamps HP to 0.
-- @param amount number  Damage to apply
-- @return number        Remaining HP
function Mobile:take_damage(amount)
    self.stats.hp = math.max(0, self.stats.hp - amount)
    return self.stats.hp
end

--- Heal the mobile. Clamps to max_hp.
-- @param amount number  HP to restore
-- @return number        New HP
function Mobile:heal(amount)
    self.stats.hp = math.min(self.stats.max_hp, self.stats.hp + amount)
    return self.stats.hp
end

--- Get the mob's effective level.
-- @return number
function Mobile:get_level()
    return self.stats.level
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

--- Check if the mobile has a specific item.
-- @param item_id string
-- @return boolean
function Mobile:has_item(item_id)
    for _, id in ipairs(self.inventory) do
        if id == item_id then return true end
    end
    return false
end

--- Add an item to the mobile's inventory.
-- @param item_id string
function Mobile:add_item(item_id)
    self.inventory[#self.inventory + 1] = item_id
end

--- Remove an item from the mobile's inventory.
-- @param item_id string
-- @return boolean  true if the item was found and removed
function Mobile:remove_item(item_id)
    for i, id in ipairs(self.inventory) do
        if id == item_id then
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
