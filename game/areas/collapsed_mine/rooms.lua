-- game/areas/collapsed_mine/rooms.lua — Three levels down, and dark.
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

return {
    _meta = {
        name   = "collapsed_mine",
        title  = "The Collapsed Mine",
        author = "Oxigeon",
        level  = "5-15",
        status = "live",
    },

    {
        id    = "collapsed_mine.adit",
        short = "The Mine Adit",
        light = 2,
        tags  = { "indoor", "mine", "underground" },
        smell = "Cold rock and old blasting powder.",
        sound = "Water, a long way down, doing something regular.",

        description = [[
The mouth of the mine, timbered and still square after two years of nobody
maintaining it. A rail runs in and stops at a tub with one wheel. Daylight gets
about thirty feet and then gives up entirely, which is where the first level
starts.]],

        exits = {
            -- `up`, not `north`: the smithy comes down to here, so this goes
            -- back up. A pair of exits that disagree about which way they are
            -- is a pair a player cannot retrace.
            up   = "thornhollow.smithy",
            down = "collapsed_mine.first_level",
        },

        items = {
            tub  = "An ore tub with one wheel and no other wheel. Somebody took "
                .. "the other wheel.",
            rail = "Iron rail, still bright on the top where the tub ran.",
        },
    },

    {
        id    = "collapsed_mine.first_level",
        short = "The First Level",
        -- Pitch dark. `Room.light_level` was a field nothing read.
        light = 0,
        tags  = { "indoor", "mine", "underground", "dark" },
        smell = "Rock dust, and something metallic under it.",
        sound = "Your own breathing, coming back off a wall you cannot see.",

        description = [[
A gallery following the seam, propped every eight feet with timber that has gone
grey. The floor is loose shale and it moves. Someone has painted arrows on the
props at knee height, all pointing back the way you came.]],

        exits = {
            up   = "collapsed_mine.adit",
            down = "collapsed_mine.second_level",
        },

        items = {
            arrows = "Arrows painted at knee height on every prop, all pointing "
                  .. "out. Painted by somebody who expected to be crawling.",
            seam   = "A band of darker rock running along the wall at chest "
                  .. "height. There is metal in it.",
        },

        actions = {
            mine = { func = mine_ore, hint = "mine the seam" },
        },
    },

    {
        id    = "collapsed_mine.second_level",
        short = "The Second Level",
        light = 0,
        tags  = { "indoor", "mine", "underground", "dark" },
        smell = "Wet rock. It is warmer here than it should be.",
        sound = "The pipes overhead, which are dry and should not be.",

        description = function(room)
            local base = [[
The gallery widens into a junction with a cast-iron grille across the west
passage, hung on runners and fitted with a lock somebody paid real money for.
Pipes run along the ceiling toward the pump house.]]
            if get_object_state("collapsed_mine.second_level", "door_open") then
                return base .. "\r\n\r\nThe grille is up. Cold air comes through it."
            end
            return base .. "\r\n\r\nThe grille is down and the lock is engaged."
        end,

        exits = {
            up   = "collapsed_mine.first_level",
            east = "collapsed_mine.pump_house",
            west = {
                target = "collapsed_mine.deep_workings",
                check = function(player)
                    if get_object_state("collapsed_mine.second_level", "door_open") then
                        return true
                    end
                    return false, "The grille is down."
                end,
            },
        },

        items = {
            grille = "Cast iron on runners, with a lock let into the frame. The "
                  .. "lock is newer than the mine.",
            pipes  = "Iron pipes along the ceiling, running east. Dry, which "
                  .. "means the pump is not running.",
        },

        actions = {
            open = { func = try_door, hint = "open the grille" },
        },
    },

    {
        id    = "collapsed_mine.pump_house",
        short = "The Pump House",
        light = 1,
        tags  = { "indoor", "mine", "underground" },
        smell = "Grease, and the particular smell of cold cast iron.",
        sound = function()
            if get_object_state("collapsed_mine.pump_house", "pump_running") then
                return "The pump, working. It is enormously loud."
            end
            return "Nothing. The pump is stopped and the silence has weight."
        end,

        description = function(room)
            local base = [[
A chamber cut square around a beam engine three times the height of a person.
Three levers stand in a rack by the wall, each as long as an arm, and each with
a plate above it that the damp has taken.]]
            local step = get_object_state("collapsed_mine.pump_house", "lever_step") or 0
            if get_object_state("collapsed_mine.pump_house", "pump_running") then
                return base .. "\r\n\r\nThe engine is working. The beam comes over "
                    .. "and goes back, and the floor moves with it."
            elseif step > 0 then
                return base .. "\r\n\r\n" .. step .. " of the levers are over and "
                    .. "holding. Something underneath is under tension."
            end
            return base .. "\r\n\r\nAll three levers stand upright."
        end,

        exits = { west = "collapsed_mine.second_level" },

        items = {
            levers = "Three levers in a rack: left, middle and right. The plates "
                  .. "above them are illegible.",
            engine = "A beam engine, cold. The bearings have been greased "
                  .. "recently, which nobody in town admits to.",
            plates = "Brass plates, green and unreadable. One of them has been "
                  .. "scratched with a tally: one line, two lines, three lines, "
                  .. "left to right.",
        },

        actions = {
            pull = { func = pull_lever, hint = "pull <left|middle|right>" },
        },
    },

    {
        id    = "collapsed_mine.deep_workings",
        short = "The Deep Workings",
        light = 0,
        tags  = { "indoor", "mine", "underground", "dark", "dangerous" },
        smell = "Water, and under the water something organic.",
        sound = "Dripping, and further in, something that is not dripping.",

        description = function(room)
            local base = [[
Past the grille the workings stop following the seam and start following
something else. The cut is rounder here and the tool marks are wrong — too
broad, and angled as if made by something working with its whole arm.]]
            if get_object_state("collapsed_mine.pump_house", "pump_running") then
                return base .. "\r\n\r\nThe water has gone down. There is a shaft in "
                    .. "the floor that was not visible before, going further down."
            end
            return base .. "\r\n\r\nThe floor is under two feet of black water."
        end,

        exits = {
            east = "collapsed_mine.second_level",
            down = {
                target = "collapsed_mine.the_sump",
                check = function(player)
                    if get_object_state("collapsed_mine.pump_house", "pump_running") then
                        return true
                    end
                    return false, "The shaft is under water."
                end,
            },
        },

        items = {
            marks = "Tool marks, too broad for a pick and angled wrongly. "
                 .. "Whatever made them worked from the elbow.",
            water = "Black standing water, two feet of it, with no bottom "
                 .. "visible.",
        },
    },

    {
        id    = "collapsed_mine.the_sump",
        short = "The Sump",
        light = 0,
        tags  = { "indoor", "mine", "underground", "dark", "dangerous", "boss" },
        smell = "The bottom of the world.",
        sound = "One sound, repeating, that is not water.",

        description = [[
The bottom of the shaft opens into a chamber that nobody cut. The walls are the
same rounded cut as the workings above and they go all the way round, and the
floor is a floor because things have been put on it: rail, timber, a tub, a
boot. It is a nest, and it has been a nest for longer than the mine has been
shut.]],

        exits = { up = "collapsed_mine.deep_workings" },

        items = {
            nest = "Rail, timber, an ore tub and one boot, arranged. Arranged is "
                .. "the word that stays with you.",
            boot = "A miner's boot, laced. It has been placed rather than dropped.",
        },
    },
}
