-- game/areas/thornhollow/shops.lua — Who sells what, and at what.
--
-- Separate from `market.lua` because a shop is a registration *against* a room
-- rather than a property of one: that split is what lets a shop move without
-- editing a room, and a room be rebuilt without losing its shop.
--
-- Three shops, deliberately different in the ways that matter:
--
--   smithy       sells weapons and armour, buys only those — the tag filter
--   provisions   sells tools, buys anything with a value — the `"*"` case
--   apothecary   a bad place to sell (rate 0.2) and a unique that never
--                restocks (`restock = 0`)

return {
    {
        id        = "thornhollow_smithy",
        name      = "Bellow & Son, Smiths",
        room      = "thornhollow.smithy",
        keeper    = "town_smith",
        greeting  = "Bellow does not look up. \"On the wall. Prices are the prices.\"",
        -- Sells at face value; pays a third. The gap is the gold sink.
        buy_rate  = 1.0,
        sell_rate = 0.33,
        -- Only what she has a use for. `sell rope` here gets a refusal that
        -- names the reason rather than a shrug.
        buys      = { "weapon", "armour" },

        stock = {
            { item = "apprentice_dagger", count = 4 },
            { item = "leather_jerkin",    count = 3 },
            { item = "oak_buckler",       count = 2 },
            -- One greatsword, and it does not come back. The `required_strength`
            -- refusal is the point of stocking it at all.
            { item = "iron_greatsword",   count = 1, restock = 0 },
        },
    },

    {
        id        = "thornhollow_provisions",
        name      = "Hobb's Provisions",
        room      = "thornhollow.general_store",
        keeper    = "town_hobb",
        greeting  = "\"Everything's on the shelves. Coin first.\"",
        buy_rate  = 1.0,
        -- Hobb buys anything, which is why he pays badly for it.
        sell_rate = 0.25,
        buys      = { "*" },

        stock = {
            { item = "hemp_rope",      count = 6 },
            { item = "hooded_lantern", count = 3 },
            { item = "hard_rations",   count = 12 },
            { item = "iron_lockpick",  count = 2, price = 30 },
            { item = "leather_backpack", count = 2 },
        },
    },

    {
        id        = "thornhollow_apothecary",
        name      = "The Apothecary",
        room      = "thornhollow.apothecary",
        keeper    = "town_apothecary",
        greeting  = "\"Mind your head. What's wrong with you?\"",
        -- Sells at a premium and pays badly: a specialist who knows you have
        -- nowhere else to go.
        buy_rate  = 1.2,
        sell_rate = 0.2,
        buys      = { "herb", "reagent", "potion" },

        stock = {
            { item = "healing_draught", count = 5 },
            { item = "marsh_antidote",  count = 4 },
            { item = "dried_marshroot", count = 20 },
        },
    },
}
