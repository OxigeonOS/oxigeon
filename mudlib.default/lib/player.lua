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

--- @class Player : Mobile
--- @field char_id string
--- @field color_enabled boolean
--- @field name string
--- @field session_id string
--- @field title string
--- @field quest_flags table
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
    -- `skills` used to live here as a parallel skill -> level map. It existed
    -- only because traits could not be sparse; a skill is a counter the
    -- character happens to hold now, so it saves under `stats` and gains
    -- clamping, bounds and a derived mastery for free. `from_save` migrates
    -- any blob still carrying the old table.
    "title",
    "race",
    "gender",
    "tags",
    "channels",        -- List of channel names the player is subscribed to
    "custom",          -- Open-ended table for game-specific data
    "page_length",     -- Lines per page for pager (0 = disabled, nil = default 40)
    "color_enabled",   -- Whether color is shown (true = default, false = stripped for screen readers)
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
    data.custom      = {}
    data.channels    = {}
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
            -- Migrate legacy string entries to instance tables
            if type(v) == "string" then
                data.inventory[i] = { template = v }
            else
                data.inventory[i] = v
            end
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
    -- Migration: a skill is a trait now, so an old blob's parallel skill map
    -- moves into `stats`. Storage is where presence comes from, so a skill
    -- lands as present the moment its number does — and if the trait file
    -- defining it has not loaded, the number sits there inert rather than
    -- failing, and starts answering when the definition arrives.
    --
    -- `data.stats` first, so a value already migrated on a previous save wins
    -- and this cannot walk a character's progress backwards.
    if type(saved.skills) == "table" then
        for k, v in pairs(saved.skills) do
            if type(k) == "string" and type(v) == "number" and data.stats[k] == nil then
                data.stats[k] = v
            end
        end
    end
    if saved.custom then
        for k, v in pairs(saved.custom) do
            data.custom[k] = v
        end
    end
    if saved.channels then
        for i, v in ipairs(saved.channels) do
            data.channels[i] = v
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
    obj.channels    = data.channels
    obj.page_length   = saved.page_length     -- nil = default 40
    obj.color_enabled = saved.color_enabled    -- nil = default true

    -- Wire up death hook to emit event
    obj.on_death = function(self)
        if DAEMON and DAEMON.event then
            DAEMON.event.emit("player.death", {
                char_id    = self.char_id,
                session_id = self.session_id,
            })
        end
    end

    -- Give them the character set — filling in any trait they have never had —
    -- then drop anything stored for a trait that is derived now and clamp the
    -- gauges into their current range. Traits outside the set, a learned skill
    -- among them, come back from their saved stats and are not seeded.
    if DAEMON and DAEMON.trait then
        local ok, err = pcall(DAEMON.trait.seed, obj, "character")
        if not ok then
            log("error", "PLAYER: could not seed traits for char "
                .. tostring(char_id) .. ": " .. tostring(err))
        end
    end
    if DAEMON and DAEMON.effect then
        local ok, err = pcall(DAEMON.effect.attach, obj)
        if not ok then
            log("error", "PLAYER: could not attach effects for char "
                .. tostring(char_id) .. ": " .. tostring(err))
        end
    end

    -- Carried items rejoin the world index, and any container's contents come
    -- back out of the entry they were folded into on the way to disk.
    if DAEMON and DAEMON.items then
        local ok, err = pcall(function()
            local Carry = require('lib.carry')
            Carry.unpack(obj.inventory)
            for _, entry in pairs(obj.equipment or {}) do
                if type(entry) == "table" then Carry.ensure_registered(entry) end
            end
        end)
        if not ok then
            log("error", "PLAYER: could not restore items for char "
                .. tostring(char_id) .. ": " .. tostring(err))
        end
    end

    -- `equip:` effects are `persist = false` and so are gone by design. What is
    -- worn is saved; the aura is derived from it, and this is where it is
    -- derived. Doing it the other way round — persisting the aura — would be a
    -- second copy of the truth that can disagree with the first.
    if DAEMON and DAEMON.effect and DAEMON.items then
        local ok, err = pcall(function()
            require('lib.equipment').refresh_all(obj)
        end)
        if not ok then
            log("error", "PLAYER: could not rebuild equipment effects for char "
                .. tostring(char_id) .. ": " .. tostring(err))
        end
    end

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

    -- A container's contents live in ITEM_D's location index, which is memory
    -- only — correct for a sword on a floor, wrong for a backpack in somebody's
    -- pack. `pack` folds them onto the entry so they are written; `unpack` in
    -- `from_save` puts them back in the index on the way in.
    if DAEMON and DAEMON.items and type(data.inventory) == "table" then
        local ok, packed = pcall(function()
            return require('lib.carry').pack(self.inventory)
        end)
        if ok then
            data.inventory = packed
        else
            log("error", "PLAYER: could not pack container contents for char "
                .. tostring(self.char_id) .. ": " .. tostring(packed))
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
---
--- The amount runs through the `xp_gained` pipeline first, so "20% more
--- experience" is an ordinary effect and needs nothing here.
--- @param amount number  XP to award, before any effect scales it
--- @param opts table|nil  { source = "kill" }
--- @return number  the XP actually awarded
function Player:award_xp(amount, opts)
    if amount <= 0 then return 0 end
    local base = amount

    if DAEMON and DAEMON.effect and DAEMON.effect.run then
        local ev = { amount = amount, scale = 0, min = 0 }
        if opts then
            for k, v in pairs(opts) do if ev[k] == nil then ev[k] = v end end
        end
        local ok, result = pcall(DAEMON.effect.run, self, "xp_gained", ev)
        if ok and not result.cancelled and type(result.amount) == "number" then
            amount = math.max(0, math.floor(result.amount))
        elseif ok and result.cancelled then
            amount = 0
        end
    end

    if amount <= 0 then return 0 end
    self.xp = (self.xp or 0) + amount

    if DAEMON and DAEMON.event then
        DAEMON.event.emit("player.xp_gained", {
            char_id     = self.char_id,
            amount      = amount,
            base_amount = base,
            total       = self.xp,
        })
    end
    return amount
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

--- Remove a quest flag entirely.
---
--- `set_quest_flag(flag, nil)` cannot do this — a missing value means `true`,
--- which is the convenient default for the common case and makes "unset it"
--- unexpressible. That is the same reason the document store needs `db_unset`
--- alongside `db_update`: Lua tables cannot hold nil, so deletion needs its own
--- verb wherever "absent" and "false" are different states.
---
--- They are different here: a quest that is *not active* and a quest that is
--- active-and-false would both read as falsey, but only the first should let
--- you take it on again.
--- @param flag string
--- @return boolean  whether anything was removed
function Player:clear_quest_flag(flag)
    if self.quest_flags[flag] == nil then return false end
    self.quest_flags[flag] = nil
    return true
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

-- ─── Communication ───────────────────────────────────────────────────────────

local strings = require('lib.strings')
local color  -- lazy-loaded to avoid circular requires

local function get_color()
    if not color then
        local ok, mod = pcall(require, 'lib.color')
        if ok then color = mod end
    end
    return color
end

--- Default wrap width when NAWS is unavailable.
Player.DEFAULT_WRAP_WIDTH = 80

--- Get the terminal width for this player's session.
-- Uses NAWS if the client reported it, otherwise falls back to DEFAULT_WRAP_WIDTH.
-- @return number  The wrap width
function Player:get_width()
    if self.session_id then
        -- Protected: `get_session` *raises* on a malformed id rather than
        -- returning nil, and this sits under every line of output the game
        -- sends. A Player holding a stale session id — one that disconnected
        -- mid-write, one restored from a save — would take down whatever was
        -- trying to talk to them, which is the worst possible moment for it.
        local ok, session = pcall(get_session, self.session_id)
        if ok and session and session.window_width and session.window_width > 0 then
            return session.window_width
        end
    end
    return Player.DEFAULT_WRAP_WIDTH
end

--- Internal: apply color (or strip) and snoop forwarding after wrapping.
-- Called as a Player method so it can check self.color_enabled.
-- @param text string  Already-wrapped text
function Player:_process_output(text)
    local c = get_color()
    if c then
        -- Respect the player's color preference (nil defaults to true)
        if self.color_enabled == false then
            text = c.strip(text)
        else
            text = c.colorize(text)
        end
    end

    send(self.session_id, text .. "\r\n")

    -- Forward to snoopers if any
    if DAEMON and DAEMON.snoop and DAEMON.snoop.is_snooped(self.session_id) then
        local snoopers = DAEMON.snoop.get_snoopers(self.session_id)
        for _, snooper_sid in ipairs(snoopers) do
            send(snooper_sid, "[SNOOP] " .. text .. "\r\n")
        end
    end
end

--- Send text to this player with automatic word wrapping and \r\n.
-- Text is wrapped to the client's terminal width. Existing line breaks
-- and paragraph separators are preserved. Color tags are translated to ANSI.
-- @param text string  The text to send
function Player:send(text)
    if self.session_id then
        local wrapped = strings.wrap(text, self:get_width())
        self:_process_output(wrapped)
    end
end

--- Send text without word wrapping (for pre-formatted content like tables, ASCII art).
-- Color tags are still processed.
-- @param text string  The text to send
function Player:send_raw(text)
    if self.session_id then
        self:_process_output(text)
    end
end

--- Send multiple lines of text to this player (each individually wrapped).
-- @param ... string  Lines to send
--- Send text through the pager, coloured to this player's preference and
--- **not** word-wrapped.
---
--- `DAEMON.pager.page` writes through the raw `send` efun, so anything handed to
--- it directly arrives with its `{colour}` tags intact and unrendered.
--- `cmds/admin/trace.lua` carried a comment warning callers not to use colour in
--- a paged body, which is the wrong end to fix it: colour is a *player*
--- preference — `color_enabled` lives here — so applying it is the Player's job
--- and not the pager's.
---
--- No wrapping, deliberately. What gets paged is listings and file contents,
--- where a wrapped line is a corrupted one; `send` and `send_lines` remain the
--- wrapping paths for prose.
---
--- `opts.literal` sends the text through untouched — no colourising, no
--- stripping. That is for showing a *file*: a mudlib source file is full of
--- `{red}` and `{/}`, and rendering them would paint the listing in the colours
--- of the code you were trying to read. Stripping them would be worse — the tags
--- would silently vanish from the source you are inspecting.
--- @param text string
--- @param opts table|nil  { page_length = n, literal = bool }
function Player:send_paged(text, opts)
    if not self.session_id then return end
    opts = opts or {}

    local c = get_color()
    if c and not opts.literal then
        if self.color_enabled == false then
            text = c.strip(text)
        else
            text = c.colorize(text)
        end
    end

    local length = opts.page_length or (self.custom and self.custom.page_length)
    if DAEMON and DAEMON.pager and length ~= 0 then
        DAEMON.pager.page(self.session_id, text, length)
    else
        -- Paging turned off (`pagesize 0`), or no pager at all.
        send(self.session_id, text .. "\r\n")
    end
end

function Player:send_lines(...)
    if not self.session_id then return end
    local width = self:get_width()
    local args = { ... }

    -- Both spellings are already in use across the mudlib —
    -- `send_lines("a", "b")` and `send_lines({ "a", "b" })` — and the second
    -- one printed `table: 0x...` to the player. `death_d` has been announcing
    -- deaths that way since it was written. Accept either.
    local lines = (#args == 1 and type(args[1]) == "table") and args[1] or args

    for _, text in ipairs(lines) do
        self:_process_output(strings.wrap(text, width))
    end
end

--- Send a message to everyone in the player's current room except this player.
-- The message is wrapped to each recipient's terminal width.
-- @param text string  The message to broadcast
function Player:message_room(text)
    if not self.char_id then return end
    local room_id = DAEMON.world.get_character_room(self.char_id)
    if room_id then
        local messaging = require('lib.messaging')
        messaging.send_to_room(room_id, text, self.char_id)
    end
end

-- ─── Movement ────────────────────────────────────────────────────────────────

--- Teleport this player to a room. Sends a room look on arrival.
-- @param room_id string  The target room ID
-- @return boolean        true if the move succeeded
function Player:move_to(room_id)
    if not self.char_id then return false end
    if not DAEMON or not DAEMON.world then return false end

    local ok = DAEMON.world.move_character(self.char_id, room_id)
    if ok and self.session_id then
        local room = DAEMON.world.get_character_room_obj(self.char_id)
        if room then
            send(self.session_id, room:get_appearance(self.session_id))
        end
    end
    return ok
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
