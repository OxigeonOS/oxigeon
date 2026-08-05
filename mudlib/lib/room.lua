-- mudlib/lib/room.lua — Room class
-- Inherits from Object. Adds exits, contents, actions, items, and appearance.
-- Properties (short, long, smell, sound) support the lfun pattern via Object.resolve().

local Object = require('lib.object')

local Room = setmetatable({}, { __index = Object })
Room.__index = Room

function Room:new(data)
    -- Initialize base Object fields
    local obj = Object.new(self, data)

    -- Room-specific fields
    obj.long        = data.long or data.description or "You are in a room."
    obj.light_level = data.light_level or 2
    obj.smell       = data.smell
    obj.sound       = data.sound
    obj.exits       = data.exits or {}
    obj.contents    = data.contents or {}
    obj.actions     = data.actions or {}   -- { verb = { func = fn, hint = "..." }, ... }
    obj.items       = data.items or {}     -- { keyword = description, ... }
    -- Tags, as items and mobiles have. The forward question ("is this room
    -- outdoors?") is a scan over this short list; the backward one ("which
    -- rooms are outdoors?") goes through `tag_d`, which indexes it.
    obj.tags        = data.tags or {}

    return obj
end

--- How light it actually is here, right now.
---
--- `light_level` is what the *room* is; this is what it *is like*, which a
--- weather daemon, a spell or a burning building may have an opinion about.
--- Every reader that cares whether you can see should use this, so a game can
--- have weather without editing the mudlib.
---
--- The hook is a game daemon exposing `light_for(room)`. There is no such
--- daemon in the mudlib and there should not be: whether it rains is content.
--- @return number  0 (pitch dark) to 3 (bright)
function Room:effective_light()
    local base = self.light_level or 2
    if DAEMON and DAEMON.weather and DAEMON.weather.light_for then
        local ok, adjusted = pcall(DAEMON.weather.light_for, self)
        if ok and type(adjusted) == "number" then return adjusted end
    end
    return base
end

--- Does this room carry a tag?
--- @param tag string
--- @return boolean
function Room:has_tag(tag)
    for _, t in ipairs(self.tags) do
        if t == tag then return true end
    end
    return false
end

--- Render the full room appearance for a player.
-- All text properties are resolved via Object.resolve(), supporting both
-- static strings and dynamic lfun-style functions.
function Room:get_appearance(session_id)
    local parts = {}
    local resolve = Object.resolve

    -- Title (resolved — can be dynamic)
    parts[#parts + 1] = resolve(self.short, self)
    -- Long description (resolved — can be dynamic)
    parts[#parts + 1] = resolve(self.long, self)

    -- Obvious exits (skip hidden exits)
    local exit_list = {}
    for dir, exit in pairs(self.exits) do
        local hidden = type(exit) == "table" and exit.hidden
        if not hidden then
            exit_list[#exit_list + 1] = dir
        end
    end
    if #exit_list > 0 then
        table.sort(exit_list)
        parts[#parts + 1] = "Obvious exits: " .. table.concat(exit_list, ", ")
    else
        parts[#parts + 1] = "Obvious exits: none"
    end

    -- Smell and sound (resolved — can be dynamic)
    local smell = resolve(self.smell, self)
    if smell then
        parts[#parts + 1] = "Smell: " .. smell
    end
    local sound = resolve(self.sound, self)
    if sound then
        parts[#parts + 1] = "Sound: " .. sound
    end

    -- Action hints (things the player can try)
    local hints = self:get_action_hints()
    if #hints > 0 then
        parts[#parts + 1] = "You could try: " .. table.concat(hints, ", ")
    end

    -- Other players present
    local my_session = get_session(session_id)
    local my_char_id = my_session and my_session.character_id or nil

    local chars_here = {}
    for _, char_id in ipairs(self.contents) do
        if char_id ~= my_char_id then
            local char_data = get_character(char_id)
            if char_data then
                chars_here[#chars_here + 1] = char_data.name .. " is here."
            end
        end
    end
    if #chars_here > 0 then
        parts[#parts + 1] = table.concat(chars_here, "\r\n")
    end

    -- Items on the floor. Before the creatures, because loot is scenery and a
    -- creature is a decision — the last thing you read before the prompt should
    -- be whatever might be about to bite you.
    --
    -- Grouped by template with a count, so a room where a fight happened reads
    -- as "three rusted daggers" rather than as three consecutive lines.
    if DAEMON and DAEMON.items and DAEMON.items.in_room then
        local ok, ground = pcall(DAEMON.items.in_room, self.id)
        if ok and #ground > 0 then
            local counts, order = {}, {}
            for _, entry in ipairs(ground) do
                local item = DAEMON.items.resolve(entry)
                local name = item and (resolve(item.short, item) or item.short)
                if type(name) == "string" then
                    if not counts[name] then
                        counts[name] = 0
                        order[#order + 1] = name
                    end
                    counts[name] = counts[name] + 1
                end
            end
            local lines = {}
            for _, name in ipairs(order) do
                lines[#lines + 1] = counts[name] > 1
                    and ("  " .. name .. " (x" .. counts[name] .. ")")
                    or ("  " .. name)
            end
            if #lines > 0 then
                parts[#parts + 1] = "Lying here:\r\n" .. table.concat(lines, "\r\n")
            end
        end
    end

    -- Creatures. After the players, so the last thing before the prompt is
    -- whatever might be about to bite you.
    if DAEMON and DAEMON.mobs then
        local ok, mobs = pcall(DAEMON.mobs.describe_room, self.id)
        if ok and mobs then parts[#parts + 1] = mobs end
    end

    return table.concat(parts, "\r\n") .. "\r\n"
end

-- ─── Character contents ──────────────────────────────────────────────────────

function Room:add_character(char_id)
    for _, id in ipairs(self.contents) do
        if id == char_id then return end
    end
    self.contents[#self.contents + 1] = char_id
end

function Room:remove_character(char_id)
    for i, id in ipairs(self.contents) do
        if id == char_id then
            table.remove(self.contents, i)
            break
        end
    end
end

function Room:get_characters()
    local copy = {}
    for _, id in ipairs(self.contents) do
        copy[#copy + 1] = id
    end
    return copy
end

-- ─── Exits ───────────────────────────────────────────────────────────────────
-- Exits can be either:
--   Simple:  exits = { north = "area.room_id" }
--   Rich:    exits = { north = { target = "area.room_id", check = fn, on_traverse = fn, ... } }

--- Check if an exit exists in a direction (including hidden exits).
function Room:has_exit(direction)
    return self.exits[direction] ~= nil
end

--- Get the target room ID for an exit direction.
-- Works with both string exits and table exits.
-- @param direction string
-- @return string|nil  The target room ID
function Room:get_exit(direction)
    local exit = self.exits[direction]
    if type(exit) == "string" then
        return exit
    elseif type(exit) == "table" then
        return exit.target
    end
    return nil
end

--- Get the full exit info table for a direction.
-- Returns the raw exit value — either a string or a table with
-- target, check, on_traverse, hidden, locked_desc, etc.
-- @param direction string
-- @return string|table|nil
function Room:get_exit_info(direction)
    return self.exits[direction]
end

-- ─── Actions (room-scoped commands) ──────────────────────────────────────────

--- Add a room-scoped command action.
function Room:add_action(verb, func, hint)
    self.actions[verb] = { func = func, hint = hint }
end

--- Get the action table for a verb, or nil.
function Room:get_action(verb)
    return self.actions[verb]
end

--- Get an array of hint strings for all actions that have hints.
function Room:get_action_hints()
    local hints = {}
    for verb, action in pairs(self.actions) do
        if action.hint then
            hints[#hints + 1] = action.hint
        else
            hints[#hints + 1] = verb
        end
    end
    return hints
end

-- ─── Items (examinable objects) ──────────────────────────────────────────────

--- Add an examinable item.
function Room:add_item(keyword, description)
    self.items[keyword] = description
end

--- Get an item description by keyword, or nil.
function Room:get_item(keyword)
    return self.items[keyword]
end

return Room
