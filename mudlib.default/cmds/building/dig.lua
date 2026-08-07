-- mudlib/cmds/building/dig.lua — A new room, and the passage to it.
--
-- The one shortcut OLC has always had, and the reason is that digging is two
-- exits and a room: doing it with `olc new room` plus two `olc set exits.*`
-- is four commands to express one intention, and one of the four is easy to
-- forget in a way that leaves a one-way passage.
--
-- Everything it changes goes into the session's **draft**. It used to write both
-- rooms to disk on every dig, discard the result of the write, and mutate the
-- live Room objects alongside — so a refused write left the world and the files
-- disagreeing with nothing reported. `olc save` is the only thing that writes.

local movement = require('lib.movement')
local olc      = require('lib.olc')

local M = {}

M.name       = "dig"
M.aliases    = {}
M.category   = "building"
M.summary    = "Create a room in a direction, with the passage back."
M.usage      = {
    "dig <direction> <room>     e.g. `dig n hall`, `dig down crypt.cistern`",
    "dig <direction> <existing> link to a room that already exists",
}
M.permission = "cmd.dig"

--- Direction names, abbreviations and opposites all come from `lib/movement.lua`.
---
--- This file used to hold private `EXPAND` and `REVERSE` tables — a third copy,
--- after `movement.OPPOSITES` and `cmds/directions.lua`'s list — while
--- `docs/src/lua-api/olc.md` claimed the reverse direction came "from the same
--- table `movement.lua` uses". It did not, and the private `REVERSE` had no
--- entry for `in` or `out`: digging either way made a one-way passage and said
--- nothing about it.
local EXPAND, REVERSE = movement.ABBREVIATIONS, movement.OPPOSITES

--- "dark_laboratory" → "Dark Laboratory"
local function humanize(name)
    return (name:gsub("_", " "):gsub("(%a)([%w_']*)", function(first, rest)
        return first:upper() .. rest
    end))
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    local function fail(message) player:send("{red}" .. message .. "{/}") end

    if not (DAEMON.olc and DAEMON.olc.is_active(session_id)) then
        return fail("You must be building first. `olc <area>` to start.")
    end
    local area = DAEMON.olc.get_state(session_id).area_name

    local direction, target = (args_str or ""):match("^(%S+)%s+(%S+)%s*$")
    if not direction then
        return player:send_lines(M.usage)
    end

    direction = movement.expand(direction)
    if not direction then
        return fail("'" .. args_str:match("^(%S+)") .. "' is not a direction. "
            .. table.concat(movement.ORDER, " "))
    end
    local back = REVERSE[direction]

    -- Where you dig *from* is where you are standing, not the cursor. Digging is
    -- a spatial act and the cursor deliberately does not follow movement, so
    -- tying it to the cursor would let you dig an exit out of a room on the
    -- other side of the area without noticing.
    local here = DAEMON.world and DAEMON.world.get_character_room(player.char_id)
    if not here then return fail("You are nowhere.") end

    local from, from_err = olc.draft(session_id, "room", here)
    if not from then return fail(tostring(from_err)) end

    -- A bare name belongs to the area being built. `olc new room` agrees.
    if not target:find("%.") then target = area .. "." .. target end

    from.exits = from.exits or {}
    if from.exits[direction] then
        return fail(here .. " already has a " .. direction .. " exit, to "
            .. tostring(from.exits[direction]) .. ".")
    end

    -- Existing or new. Linking to an existing room is the second half of a loop
    -- and has to work, or an area can only ever be a tree.
    local existed = DAEMON.world.get_room(target) ~= nil
    local to
    if existed then
        to = olc.draft(session_id, "room", target)
    else
        local _, room_name = target:match("^(.+)%.([^%.]+)$")
        local err
        to, err = olc.create(session_id, "room", target)
        if not to then return fail(tostring(err)) end
        to.short = humanize(room_name or target)
    end

    from.exits[direction] = target
    to.exits = to.exits or {}

    local linked_back = false
    if not to.exits[back] then
        to.exits[back] = here
        linked_back = true
    end

    for _, spec in ipairs({ { here, from }, { target, to } }) do
        DAEMON.olc.touch(session_id, "room", spec[1])
        local ok, err = olc.apply_live("room", spec[2])
        if not ok then
            return fail("Could not rebuild " .. spec[1] .. ": " .. tostring(err))
        end
    end

    -- The cursor follows a dig — unlike movement — because you have just
    -- explicitly created that room and the next thing you do is describe it.
    DAEMON.olc.set_cursor(session_id, "room", target)

    player:send("{green}[OLC]{/} " .. (existed and "Linked to" or "Created")
        .. " room {yellow}" .. target .. "{/}")
    player:send("  " .. here .. "  {cyan}" .. direction .. "{/} → " .. target)
    if linked_back then
        player:send("  " .. target .. "  {cyan}" .. back .. "{/} → " .. here
            .. "  {dim}(the way back){/}")
    else
        player:send("  {yellow}" .. target .. " already had a " .. back
            .. " exit, so the passage is one-way.{/}")
    end
    player:send("  {dim}Cursor: " .. target .. ". Unsaved — `olc save` to write.{/}")

    if DAEMON.world then
        pcall(DAEMON.world.move_character, player.char_id, target)
        local room = DAEMON.world.get_room(target)
        if room and room.get_appearance then
            player:send(room:get_appearance(session_id))
        end
    end
end

return M
