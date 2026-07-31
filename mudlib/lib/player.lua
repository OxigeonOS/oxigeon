-- mudlib/lib/player.lua — Player class
-- Inherits from Mobile. Represents a logged-in player character.
--
-- Players bridge the gap between live game objects and database persistence:
--   Login:    CHARACTER_D.load(char_id) → hydrates a Player from saved JSON
--   Gameplay: The Player IS the live state — stats, inventory, equipment
--   Save:     player:to_save() → serializes persistent fields back to JSON
--
-- Non-persistent fields (session_id, combat target, etc.) are transient
-- and will not be written to the database.

local Mobile = require('lib.mobile')

local Player = setmetatable({}, { __index = Mobile })
Player.__index = Player

-- ─── Persistent field declarations ───────────────────────────────────────────
-- Only these fields are serialized to the database via to_save().
-- Everything else (session_id, combat state, event subscriptions) is transient.

Player.SAVE_FIELDS = {
    "stats",
    "inventory",
    "equipment",
    "gold",
    "xp",
    "quest_flags",
    "skills",
    "title",
    "race",
    "gender",
    "tags",
    "custom",          -- Open-ended table for game-specific data
}

-- ─── Default starting stats for new characters ──────────────────────────────

Player.DEFAULTS = {
    stats = {
        hp           = 100,
        max_hp       = 100,
        mp           = 50,
        max_mp       = 50,
        strength     = 10,
        dexterity    = 10,
        intelligence = 10,
        constitution = 10,
        level        = 1,
    },
    gold        = 0,
    xp          = 0,
    inventory   = {},
    equipment   = {},
    quest_flags = {},
    skills      = {},
    custom      = {},
}

-- ─── Constructor ─────────────────────────────────────────────────────────────

--- Hydrate a Player from database records.
-- @param char_id number          The character ID (from the database)
-- @param char_info table         From get_character(): { id, name, account_id }
-- @param saved table|nil         From load_character_data(): the JSON blob (or {} for new chars)
-- @return table                  The Player object
function Player:from_save(char_id, char_info, saved)
    saved = saved or {}

    -- Build a data table by layering: defaults → saved data → identity
    local data = {}

    -- Start with defaults (deep copy stats/tables so each player is independent)
    data.stats = {}
    for k, v in pairs(Player.DEFAULTS.stats) do
        data.stats[k] = v
    end

    data.inventory   = {}
    data.equipment   = {}
    data.quest_flags = {}
    data.skills      = {}
    data.custom      = {}
    data.gold        = Player.DEFAULTS.gold
    data.xp          = Player.DEFAULTS.xp

    -- Layer saved data over defaults
    if saved.stats then
        for k, v in pairs(saved.stats) do
            data.stats[k] = v
        end
    end
    if saved.inventory then
        for i, v in ipairs(saved.inventory) do
            data.inventory[i] = v
        end
    end
    if saved.equipment then
        for k, v in pairs(saved.equipment) do
            data.equipment[k] = v
        end
    end
    if saved.quest_flags then
        for k, v in pairs(saved.quest_flags) do
            data.quest_flags[k] = v
        end
    end
    if saved.skills then
        for k, v in pairs(saved.skills) do
            data.skills[k] = v
        end
    end
    if saved.custom then
        for k, v in pairs(saved.custom) do
            data.custom[k] = v
        end
    end
    if saved.gold then data.gold = saved.gold end
    if saved.xp then data.xp = saved.xp end

    -- Scalar saved fields
    data.title  = saved.title  or char_info.name
    data.race   = saved.race
    data.gender = saved.gender
    data.tags   = saved.tags or {}

    -- Identity (from the character DB record, not the JSON blob)
    data.id    = "player." .. char_id
    data.short = char_info.name

    -- Description defaults to something sensible
    data.description = saved.description or ("You see " .. char_info.name .. ".")

    -- Create the Mobile (and thus Object) via the chain
    local obj = Mobile.new(self, data)

    -- ─── Player-specific transient fields ────────────────────────────────────
    obj.char_id    = char_id
    obj.account_id = char_info.account_id
    obj.name       = char_info.name

    -- Persistent fields stored directly (not in Mobile's base)
    obj.gold        = data.gold
    obj.xp          = data.xp
    obj.quest_flags = data.quest_flags
    obj.custom      = data.custom

    return obj
end

-- ─── Serialization ───────────────────────────────────────────────────────────

--- Serialize persistent fields to a flat table for database storage.
-- Only fields listed in SAVE_FIELDS are included.
-- @return table  JSON-safe data table
function Player:to_save()
    local data = {}
    for _, field in ipairs(Player.SAVE_FIELDS) do
        local value = self[field]
        if value ~= nil then
            -- Deep copy tables to avoid accidental mutation after save
            if type(value) == "table" then
                data[field] = Player._deep_copy(value)
            else
                data[field] = value
            end
        end
    end
    return data
end

--- Convenience: save this player via CHARACTER_D.
-- @return boolean  true if save succeeded
function Player:save()
    if DAEMON and DAEMON.character then
        return DAEMON.character.save(self.char_id)
    end
    log("error", "PLAYER: Cannot save — CHARACTER_D not available")
    return false
end

-- ─── XP & Leveling ──────────────────────────────────────────────────────────

--- Award XP to the player. Emits "player.xp_gained" event.
-- @param amount number  XP to award
function Player:award_xp(amount)
    if amount <= 0 then return end
    self.xp = (self.xp or 0) + amount

    if DAEMON and DAEMON.event then
        DAEMON.event.emit("player.xp_gained", {
            char_id = self.char_id,
            amount  = amount,
            total   = self.xp,
        })
    end
end

--- Award gold to the player.
-- @param amount number  Gold to award
function Player:award_gold(amount)
    self.gold = (self.gold or 0) + amount
end

--- Deduct gold from the player.
-- @param amount number  Gold to deduct
-- @return boolean       true if the player had enough gold
function Player:spend_gold(amount)
    if (self.gold or 0) < amount then
        return false
    end
    self.gold = self.gold - amount
    return true
end

-- ─── Quest flags ─────────────────────────────────────────────────────────────

--- Set a quest flag.
-- @param flag string   The flag name
-- @param value any     The value (default true)
function Player:set_quest_flag(flag, value)
    if value == nil then value = true end
    self.quest_flags[flag] = value
end

--- Get a quest flag.
-- @param flag string   The flag name
-- @return any          The value, or nil
function Player:get_quest_flag(flag)
    return self.quest_flags[flag]
end

--- Check if a quest flag is set (truthy).
-- @param flag string
-- @return boolean
function Player:has_quest_flag(flag)
    return self.quest_flags[flag] ~= nil and self.quest_flags[flag] ~= false
end

-- ─── Display ─────────────────────────────────────────────────────────────────

--- Get a short display string for who/look listing.
-- @return string
function Player:display_name()
    local name = self.name or "Someone"
    if self.title and self.title ~= self.name then
        return name .. " " .. self.title
    end
    return name
end

--- Get the full examination text (for "look at <player>").
-- @return string
function Player:examine()
    local resolve = require('lib.object').resolve
    local parts = {}

    parts[#parts + 1] = self:display_name()
    parts[#parts + 1] = resolve(self.description, self) or "You see nothing special."

    if self.race then
        parts[#parts + 1] = "Race: " .. self.race
    end
    parts[#parts + 1] = "Level: " .. (self.stats.level or 1)

    return table.concat(parts, "\r\n") .. "\r\n"
end

-- ─── Utility ─────────────────────────────────────────────────────────────────

--- Deep copy a table (for serialization safety).
-- Does not copy functions or metatables.
function Player._deep_copy(orig)
    if type(orig) ~= "table" then return orig end
    local copy = {}
    for k, v in pairs(orig) do
        if type(v) == "table" then
            copy[k] = Player._deep_copy(v)
        elseif type(v) ~= "function" then
            copy[k] = v
        end
    end
    return copy
end

return Player
