-- game/areas/thornhollow/market.lua — The arcade, and the two shops in it.
--
-- The rooms are here; the *shops* are in `shops.lua`, because a shop is a
-- registration against a room rather than a property of one. That split is what
-- lets a shop move without editing a room, and a room be rebuilt without
-- losing its shop.

return {
    {
        id    = "thornhollow.market",
        short = "The Market Arcade",
        light = 2,
        tags  = { "indoor", "town", "safe", "social" },
        smell = "Dried herbs, sawdust and wet wool.",
        sound = "Two dozen conversations under a low stone ceiling.",

        description = [[
The arcade is a stone tunnel with the shopfronts cut into its sides, roofed
because the town has more rain than money. Trestles run down the middle on
market days and are stacked against the wall on every other. The far end opens
onto the apothecary's steps; the near end gives back onto the square.]],

        exits = {
            west  = "thornhollow.square",
            north = "thornhollow.general_store",
            south = "thornhollow.apothecary",
        },

        items = {
            trestles = "Stacked trestle tables, worn pale where things have been "
                    .. "put down on them.",
            ceiling  = "Low stone vaulting, sooted black above each shopfront.",
        },
    },

    {
        id    = "thornhollow.general_store",
        short = "Hobb's Provisions",
        light = 2,
        tags  = { "indoor", "town", "shop", "safe" },
        smell = "Rope, tallow and dried fish.",
        sound = "Something small moving in the back, which Hobb ignores.",

        description = [[
Every wall is shelf and every shelf is full, in the manner of a shop that has
never once thrown anything away. Rope, lamp oil, sacking, biscuit, nails by the
scoop. A hand-lettered card by the door reads NO CREDIT, and under it, in a
different hand, NOT EVEN YOU.]],

        exits = { south = "thornhollow.market" },

        items = {
            card    = "NO CREDIT. And beneath, in a different and angrier hand, "
                   .. "NOT EVEN YOU.",
            shelves = "Shelves to the ceiling, arranged by no system anyone but "
                   .. "Hobb could reconstruct.",
        },
    },

    {
        id    = "thornhollow.apothecary",
        short = "The Apothecary's Steps",
        light = 1,
        tags  = { "indoor", "town", "shop", "safe" },
        smell = "Bitter green things, and under it something medicinal and old.",
        sound = "A slow drip into a copper basin.",

        description = [[
Six steps down into a room the width of two people. Bunches hang drying from
every beam, so low that anyone tall walks bent. The counter is a plank across
two barrels, and behind it the wall is entirely small drawers, each labelled in
a hand that got smaller as it ran out of room.]],

        exits = { north = "thornhollow.market" },

        items = {
            drawers = "Two hundred small drawers, labelled in a hand that got "
                   .. "steadily smaller as the labeller ran out of wall.",
            bunches = "Herbs hung to dry. Some of them are from the marsh, which "
                   .. "is not a thing most towns would allow indoors.",
        },
    },
}
