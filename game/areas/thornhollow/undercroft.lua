-- game/areas/thornhollow/undercroft.lua — Under the chapel.
--
-- The third file of the one area. Also where the bank vault lives, which is the
-- container showcase that is not a corpse and not a backpack: a fixed container
-- in a room, that stays where it is and holds what you left in it.

local function pry_flagstone(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    local ROOM = "thornhollow.crypt"
    if get_object_state(ROOM, "flagstone_lifted") then
        player:send("The flagstone is already up. The hole under it is empty.")
        return
    end

    -- A requirement met by a trait, not by an item: this is the one place in
    -- the town that cares how strong you are.
    if player:trait("strength") < 13 then
        player:send("You get your fingers under the edge and heave. It does not "
            .. "move, and your fingers say enough about that.")
        return
    end

    set_object_state(ROOM, "flagstone_lifted", true)
    player:send_lines(
        "The flagstone comes up with a sound like a held breath let go.",
        "Underneath, in a hollow the size of a hat, is a small brass key.")
    player:message_room(player.name .. " levers up a flagstone.")

    if DAEMON and DAEMON.items then
        DAEMON.items.spawn("brass_key", DAEMON.items.location("room", ROOM))
    end
end

return {
    {
        id    = "thornhollow.undercroft_stair",
        short = "The Undercroft Stair",
        light = 1,
        tags  = { "indoor", "town", "underground" },
        smell = "Cold stone and candle smoke.",
        sound = "Your own feet, louder than you expected.",

        description = [[
Twelve steps down, cut rather than laid, with a rope handrail that has been
replaced more recently than anything else here. The light from the square
reaches four of the steps. A lantern bracket at the bottom holds no lantern.]],

        exits = {
            up   = "thornhollow.square",
            down = "thornhollow.undercroft",
        },

        items = {
            bracket = "An iron lantern bracket, empty, with a ring of old soot "
                   .. "on the stone above it.",
            rope    = "A rope handrail, newer than the steps by three centuries.",
        },
    },

    {
        id    = "thornhollow.undercroft",
        short = "The Undercroft",
        light = 1,
        tags  = { "indoor", "town", "underground", "safe" },
        smell = "Dry dust, which is a surprise this close to the marsh.",
        sound = "Nothing at all, and it takes a moment to notice.",

        description = [[
A vaulted room the footprint of the chapel above it, kept dry by something
nobody in the town can explain and nobody wants explained. The town's strongbox
sits against the north wall on a plinth, under the one bracket that does hold a
lantern. An arch east leads to the older part, where the crypt is.]],

        exits = {
            up   = "thornhollow.undercroft_stair",
            east = "thornhollow.crypt",
        },

        items = {
            plinth  = "A stone plinth, worn on top where the strongbox has been "
                   .. "dragged on and off it.",
            lantern = "A lantern in a bracket, lit. Somebody comes down to see "
                   .. "to it, and nobody has ever said who.",
        },
    },

    {
        id    = "thornhollow.crypt",
        short = "The Old Crypt",
        light = 0,
        tags  = { "indoor", "town", "underground", "dark" },
        smell = "Stone, and the ghost of incense.",
        sound = "A drip somewhere behind the wall, not in the room.",

        description = [[
The older part, and it does not match: the vaulting here is round rather than
pointed, and the floor is flagstones rather than beaten chalk. Names are cut
into the walls in a script the chapel above does not use. One flagstone near the
east wall sits a finger higher than the rest.]],

        exits = { west = "thornhollow.undercroft" },

        items = {
            names     = "Names cut into the wall, in a script that reads left to "
                     .. "right and then right to left, alternating.",
            flagstone = "One flagstone sits a finger proud of the others. It has "
                     .. "been lifted before.",
        },

        actions = {
            pry = { func = pry_flagstone, hint = "pry the flagstone" },
        },
    },
}
