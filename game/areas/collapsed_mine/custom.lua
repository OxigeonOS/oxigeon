-- game/areas/collapsed_mine/custom.lua - hand-written. OLC never reads or
-- writes this file.
--
-- What this area exists to prove:
--
--   light 0            you need a lantern. `Room.light_level` was a field
--                      nothing read; `lib/light.lua` reads it now.
--   a locked door      object state on a room, which survives a `reload` and
--                      is cleared by an area reset — which is exactly right for
--                      a door and exactly wrong for a daily gate
--   a lever puzzle     three levers, an order, and a timed reset on a ticker
--   the boss           `unique`, drops a corpse container, and lays a curse
--                      that `survives_death`
--   the area reset     the puzzle clears; the daily herb gate in the marsh
--                      does not. That contrast is the point.
--
-- ─── The two checked exits ───────────────────────────────────────────────────
--
-- `exits` is a schema `map`, so a patch of it **merges direction by direction**:
-- the plain exits stay in `rooms.lua` where OLC can see and edit them, and only
-- the two carrying a `check` function are here.
--
-- Each of those restates its `target` beside the `check`. It has to:
-- `patch.merge_one` replaces an `of_record` value *whole* rather than merging
-- into it, so a patch giving only `check` would leave the exit with no
-- destination. This is also why the conversion was done by hand — `olc adopt`
-- classifies the whole `exits` map as lossy the moment one direction holds a
-- function, and would have dropped both rooms' plain exits with it.

local LEVER_ROOM = "collapsed_mine.pump_house"
local DOOR_ROOM  = "collapsed_mine.second_level"

--- The order the levers have to be pulled in. Written down here rather than
--- derived, because a puzzle whose answer is computed is a puzzle nobody can
--- write a hint for.
local LEVER_ORDER = { "left", "middle", "right" }

--- How long you have to finish before it resets itself. Short enough to be a
--- puzzle rather than a chore.
local LEVER_TIMEOUT = 60

-- ─── The locked door ─────────────────────────────────────────────────────────

local function try_door(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if get_object_state(DOOR_ROOM, "door_open") then
        player:send("The grille is already open.")
        return
    end

    if not get_object_state(DOOR_ROOM, "door_unlocked") then
        -- The key, or the lockpick and enough dexterity. Two routes, because a
        -- door with exactly one key is a door that is really a switch.
        local Carry = require('lib.carry')
        local has_key = select(1, Carry.find(player, "brass key",
            { inventory = true, room = false })) ~= nil

        if has_key then
            set_object_state(DOOR_ROOM, "door_unlocked", true)
            player:send("The brass key turns, badly, and the lock gives.")
        else
            local has_pick = select(1, Carry.find(player, "lockpick",
                { inventory = true, room = false })) ~= nil
            if not has_pick then
                player:send("The grille is locked. There is a keyhole, and it is "
                    .. "the size of a thumb.")
                return
            end
            if player:trait("dexterity") < 13 then
                player:send("You get the pick in and feel the wards, and then the "
                    .. "pick comes out again without them.")
                return
            end
            set_object_state(DOOR_ROOM, "door_unlocked", true)
            player:send("Three wards, and the third one takes a while. The lock gives.")
        end
    end

    set_object_state(DOOR_ROOM, "door_open", true)
    player:send("You haul the grille up on its runners. Cold air comes out.")
    player:message_room(player.name .. " opens the grille.")
end

-- ─── The lever puzzle ────────────────────────────────────────────────────────

local function reset_levers(quiet)
    set_object_state(LEVER_ROOM, "lever_step", 0)
    set_object_state(LEVER_ROOM, "pump_running", false)
    if not quiet and DAEMON and DAEMON.world then
        local messaging = require('lib.messaging')
        pcall(messaging.send_to_room, LEVER_ROOM,
            "{yellow}Somewhere below, something heavy resets itself.{/}", nil)
    end
end

local function pull_lever(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    local which = (args[1] or ""):lower()
    if which == "" then
        player:send("Pull which lever? There are three: left, middle and right.")
        return
    end

    if get_object_state(LEVER_ROOM, "pump_running") then
        player:send("The pump is already running. The water is going down.")
        return
    end

    local step = get_object_state(LEVER_ROOM, "lever_step") or 0
    local expected = LEVER_ORDER[step + 1]

    if which ~= expected then
        if step > 0 then
            player:send_lines(
                "The lever goes over and something under the floor lets go with a",
                "sound you feel rather than hear. The other levers spring back.")
            reset_levers(true)
        else
            player:send("The lever goes over and comes straight back. Nothing else happens.")
        end
        return
    end

    step = step + 1
    set_object_state(LEVER_ROOM, "lever_step", step)

    if step < #LEVER_ORDER then
        player:send("The " .. which .. " lever goes over and stays over. Something "
            .. "under the floor takes up the slack.")
        player:message_room(player.name .. " pulls the " .. which .. " lever.")

        -- A timed reset, on the ticker, replacing itself by id — so pulling the
        -- first lever twice re-arms one timer rather than stacking two.
        if DAEMON and DAEMON.ticker then
            DAEMON.ticker.after(LEVER_TIMEOUT, "mine.levers.reset", function()
                if (get_object_state(LEVER_ROOM, "lever_step") or 0) > 0
                    and not get_object_state(LEVER_ROOM, "pump_running") then
                    reset_levers(false)
                end
            end)
        end
        return
    end

    -- Done.
    set_object_state(LEVER_ROOM, "pump_running", true)
    set_object_state(LEVER_ROOM, "lever_step", #LEVER_ORDER)
    if DAEMON and DAEMON.ticker then
        pcall(DAEMON.ticker.remove, "mine.levers.reset")
    end

    player:send_lines(
        "The third lever goes over and the pump takes. Water starts moving in the",
        "pipes overhead, and a long way down something that was under water stops",
        "being under water.")
    player:message_room("The pump starts. " .. player.name .. " looks pleased.")

    if DAEMON and DAEMON.event then
        pcall(DAEMON.event.emit, "mine.pump_started", { char_id = player.char_id })
    end
end

-- ─── The ore node ────────────────────────────────────────────────────────────

local function mine_ore(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    local ROOM = "collapsed_mine.first_level"

    -- Depleted is *room* state, not per-character: everyone shares one seam,
    -- and an area reset refilling it is correct. That is the difference from
    -- the marsh's herb bed, where the gate is per character and must survive a
    -- reset.
    if get_object_state(ROOM, "seam_depleted") then
        player:send("The seam here is worked out. Give the mine time.")
        return
    end

    set_object_state(ROOM, "seam_depleted", true)
    player:send_lines(
        "You get the pick in behind a plate of shale and lever. A hand's worth of",
        "ore comes away with it.")
    player:message_room(player.name .. " works a seam loose.")
    player:add_item("iron_ore")

    if DAEMON and DAEMON.trait and DAEMON.trait.has(player, "mining") then
        DAEMON.trait.adjust(player, "mining", 1)
    end
end

-- ─── Descriptions and gates that read the puzzle's state ─────────────────────
--
-- Hoisted for the same reason the actions above are: a room file should read as
-- what the rooms *are*, not as prose with programs threaded through it. Each of
-- these is a plain lfun, resolved by `Object.resolve` like a string would be.

local SECOND_LEVEL = "collapsed_mine.second_level"
local PUMP_HOUSE   = "collapsed_mine.pump_house"

local function grille_is_up()
    return get_object_state(SECOND_LEVEL, "door_open") and true or false
end

local function pump_is_running()
    return get_object_state(PUMP_HOUSE, "pump_running") and true or false
end

local function second_level_description(room)
    local base = [[
The gallery widens into a junction with a cast-iron grille across the west
passage, hung on runners and fitted with a lock somebody paid real money for.
Pipes run along the ceiling toward the pump house.]]
    if grille_is_up() then
        return base .. "\r\n\r\nThe grille is up. Cold air comes through it."
    end
    return base .. "\r\n\r\nThe grille is down and the lock is engaged."
end

local function through_the_grille(player)
    if grille_is_up() then return true end
    return false, "The grille is down."
end

local function pump_house_sound()
    if pump_is_running() then
        return "The pump, working. It is enormously loud."
    end
    return "Nothing. The pump is stopped and the silence has weight."
end

local function pump_house_description(room)
    local base = [[
A chamber cut square around a beam engine three times the height of a person.
Three levers stand in a rack by the wall, each as long as an arm, and each with
a plate above it that the damp has taken.]]
    local step = get_object_state(PUMP_HOUSE, "lever_step") or 0
    if pump_is_running() then
        return base .. "\r\n\r\nThe engine is working. The beam comes over "
            .. "and goes back, and the floor moves with it."
    elseif step > 0 then
        return base .. "\r\n\r\n" .. step .. " of the levers are over and "
            .. "holding. Something underneath is under tension."
    end
    return base .. "\r\n\r\nAll three levers stand upright."
end

local function deep_workings_description(room)
    local base = [[
Past the grille the workings stop following the seam and start following
something else. The cut is rounder here and the tool marks are wrong — too
broad, and angled as if made by something working with its whole arm.]]
    if pump_is_running() then
        return base .. "\r\n\r\nThe water has gone down. There is a shaft in "
            .. "the floor that was not visible before, going further down."
    end
    return base .. "\r\n\r\nThe floor is under two feet of black water."
end

local function down_the_shaft(player)
    if pump_is_running() then return true end
    return false, "The shaft is under water."
end

-- ─── The boss, from mobs.lua ───────────────────────────────

local messaging = require('lib.messaging')

--- What the corpse holds. Named so the drop list is readable as a list.
local DELVER_LOOT = { "delvers_claw", "iron_ore", "iron_ore", "brass_key" }

--- How long a corpse lasts before it comes apart, in seconds.
local CORPSE_ROT_SECONDS = 600

--- A curse that outlives dying, laid at random during the fight.
local function delver_on_combat(mob, target)
    if DAEMON and DAEMON.effect and math.random(100) <= 20 then
        DAEMON.effect.apply(target, "delvers_regard", {
            source = "mob:" .. tostring(mob.id),
        })
    end
end

--- Arm the timer that takes one corpse apart again.
---
--- A ticker rather than a task, because it is one corpse and not a recurring
--- job — and keyed by the corpse's id, so a second boss's corpse arms its own
--- timer rather than replacing this one.
local function rot_later(corpse_id, room_id)
    if not DAEMON.ticker then return end
    DAEMON.ticker.after(CORPSE_ROT_SECONDS, "corpse.rot." .. corpse_id, function()
        local still = DAEMON.items.get_instance(corpse_id)
        if not still then return end
        pcall(messaging.send_to_room, room_id,
            "The corpse comes apart into things that are not a corpse.", nil)
        DAEMON.items.destroy(still)
    end)
end

--- Death drops a **corpse**, which is a container with the loot in it.
---
--- The reason a boss gets one and a rat does not: a rat drops one thing and a
--- boss drops six, and six items on a floor is a wall of text where a corpse is
--- one line and a decision. It is also the third kind of container — not
--- carried, not fixed, and it goes away.
local function delver_on_death(mob)
    if not (DAEMON and DAEMON.items and mob.room_id) then return end

    local corpse = DAEMON.items.spawn("delver_corpse",
        DAEMON.items.location("room", mob.room_id))
    if not corpse then return end

    local into = DAEMON.items.location("item", corpse.id)
    for _, template in ipairs(DELVER_LOOT) do
        DAEMON.items.spawn(template, into)
    end

    rot_later(corpse.id, mob.room_id)

    -- An area-wide event on the boss's death: the thing `signals.md` uses as
    -- its worked example, made real.
    if DAEMON.event then
        pcall(DAEMON.event.emit, "area.collapsed_mine.delver_slain", {
            room_id = mob.room_id,
            killer_char_id = mob._killed_by and mob._killed_by.char_id,
        })
    end
end

return {
    rooms = {
        ["collapsed_mine.first_level"] = {
            actions = {
                mine = { func = mine_ore, hint = "mine the seam" },
            },
        },
        ["collapsed_mine.second_level"] = {
            description = second_level_description,
            exits = {
                west = {
                    target = "collapsed_mine.deep_workings",
                    check  = through_the_grille,
                },
            },
            actions = {
                open = { func = try_door, hint = "open the grille" },
            },
        },
        ["collapsed_mine.pump_house"] = {
            description = pump_house_description,
            sound = pump_house_sound,
            actions = {
                pull = { func = pull_lever, hint = "pull <left|middle|right>" },
            },
        },
        ["collapsed_mine.deep_workings"] = {
            description = deep_workings_description,
            exits = {
                down = {
                    target = "collapsed_mine.the_sump",
                    check  = down_the_shaft,
                },
            },
        },
    },

    mobs = {
        ["the_delver"] = {
            on_combat = delver_on_combat,
            on_death  = delver_on_death,
        },
    },
}
