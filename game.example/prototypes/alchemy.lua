-- game/prototypes/alchemy.lua — What a reagent vial is.
--
-- The wizard workshop built its three reagent potions in a Lua `for` loop over a
-- colour table. That was the right instinct — they differ by two strings — and
-- it had one cost nobody had priced: a loop is not data, so `olc list items`
-- could not see them, `verify` could not check them and `olc set` could not
-- reach them. They existed only when the file ran.
--
-- Three declared records naming one prototype say the same thing and are
-- visible to all four.

return {
    items = {
        ["reagent_vial"] = {
            weight = 1,
            value  = 12,
            tags   = { "reagent" },
        },
    },
}
