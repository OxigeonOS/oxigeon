-- game/areas/thornhollow/square.lua — The town square and what leads off it.
--
-- One area split across three files, joined by `init.lua` with `ROOM_D.merge`.
-- The split is by *place*, not by size: a builder editing the market should not
-- be reading the undercroft, and a merge conflict in one should not touch the
-- other.

local function read_notices(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end
    player:send_lines(
        "The board is a slab of oak nailed to two posts, thick with layers of",
        "old parchment. Read it properly with the {cyan}board{/} command.")
end

local function drink_from_well(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    -- A cooldown rather than a room-state flag. Per-character state on a room
    -- is wiped by an area reset, which is how a "once a day" gate becomes
    -- "once every fifteen minutes" — see CLAUDE.md.
    if DAEMON and DAEMON.cooldown then
        if not DAEMON.cooldown.ready(player.char_id, "well_draught") then
            local left = DAEMON.cooldown.remaining(player.char_id, "well_draught")
            player:send("The water is cold and clear, but you have had your fill. ("
                .. math.ceil(left) .. "s)")
            return
        end
        DAEMON.cooldown.mark(player.char_id, "well_draught", 300)
    end

    player:send("You haul up the bucket and drink. The water tastes faintly of iron.")
    player:message_room(player.name .. " drinks from the well.")
    if DAEMON and DAEMON.trait then
        DAEMON.trait.adjust(player, "hp", 5)
    end
end

return {
    {
        id    = "thornhollow.square",
        short = "Thornhollow Square",
        light = 3,
        tags  = { "outdoor", "town", "safe" },
        smell = "Woodsmoke, wet stone and something sour off the marsh.",
        sound = "The creak of the well rope and two people arguing about a goat.",

        description = [[
The square is a lopsided rectangle of packed earth with a stone well off centre,
as if the town grew around the water rather than the other way about. Buildings
lean inward on three sides — the market arcade to the east, the smithy's
chimney smoking to the north, and the low black mouth of the undercroft stair
going down beside the chapel. West, past the last house, the road gives up and
becomes a track toward the marsh.]],

        exits = {
            north = "thornhollow.smithy",
            east  = "thornhollow.market",
            west  = "thornhollow.west_gate",
            south = "thornhollow.tavern",
            down  = "thornhollow.undercroft_stair",
        },

        items = {
            well   = "A stone well, its rim worn into a shallow saddle by three "
                  .. "hundred years of rope. The bucket is newer than the rope.",
            board  = "A notice board thick with parchment, most of it illegible. "
                  .. "The top layer is fresh.",
            chapel = "A small chapel with a slate roof and no bell. The door is "
                  .. "shut but not locked.",
        },

        actions = {
            -- A room action that shadows nothing, and one that shadows a system
            -- command. Dispatch order puts room actions first, so `drink` here
            -- is the well rather than a potion — which is the precedence rule
            -- made visible rather than only documented.
            notices = { func = read_notices, hint = "read the notices" },
            drink   = { func = drink_from_well, hint = "drink from the well" },
        },
    },

    {
        id    = "thornhollow.smithy",
        short = "Bellow & Son, Smiths",
        light = 2,
        tags  = { "indoor", "town", "shop", "safe" },
        smell = "Hot iron, quench-water and coal.",
        sound = "The uneven ring of a hammer that stops whenever you speak.",

        description = [[
Heat comes off the forge in slabs. The walls are hung with work in every stage
of finish — blanks, half-ground blades, a mail shirt with one sleeve. A long
bench runs under the window, and everything on it has been put down mid-job and
left there. There is no son.]],

        exits = {
            south = "thornhollow.square",
            -- The mine adit is behind the forge, which is why the smith is the
            -- one who knows about it.
            down  = "collapsed_mine.adit",
        },

        items = {
            forge = "A brick forge, banked rather than lit. The coal in it is "
                 .. "good coal, which says something about the year.",
            mail  = "A mail shirt hanging from a nail, finished except for one "
                 .. "sleeve. It has been finished except for one sleeve for a "
                 .. "long time.",
        },
    },

    {
        id    = "thornhollow.tavern",
        short = "The Drowned Bell",
        light = 1,
        tags  = { "indoor", "town", "safe", "social" },
        smell = "Spilled ale, tallow and old rushes.",
        sound = "Low talk that drops whenever the door opens.",

        description = [[
The common room of the Drowned Bell is darker than the hour warrants, on
purpose. A bell hangs behind the bar, green with corrosion, dredged out of the
reach by someone's grandfather; nobody agrees which one. The tables are set far
enough apart that two conversations do not have to be one.]],

        exits = { north = "thornhollow.square" },

        items = {
            bell  = "A ship's bell, corroded to the colour of pond weed. Struck, "
                 .. "it makes almost no sound at all.",
            hearth = "A wide hearth with a fire in it that somebody keeps small "
                  .. "on purpose.",
        },
    },

    {
        id    = "thornhollow.west_gate",
        short = "The West Gate",
        light = 3,
        tags  = { "outdoor", "town", "border" },
        smell = "Standing water, and the wind off it.",
        sound = "Reeds, and further off, water moving where it should not.",

        description = [[
Two posts and a rail that has not been a gate for some years. Past it the road
stops pretending and becomes a causeway of laid stone running west into
Greywater — grey water, grey reed, grey light. A board nailed to the left post
reads DO NOT LEAVE THE STONE in letters that have been repainted.]],

        exits = {
            east = "thornhollow.square",
            -- Out onto the causeway. The marsh linked back here from the
            -- beginning and this side was missing, which made the whole area
            -- unreachable on foot — visible only by walking it.
            west = "greywater_marsh.causeway_head",
        },

        items = {
            board = "DO NOT LEAVE THE STONE. Repainted, more than once, by "
                 .. "someone who did not think the first two attempts had worked.",
            posts = "Two oak posts with hinge marks and no hinges.",
        },
    },
}
