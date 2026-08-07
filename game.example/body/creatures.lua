-- game/body/creatures.lua — What the creatures in this game are made of.
--
-- The mudlib ships no layouts, deliberately: a humanoid is content, and a
-- mudlib that shipped one would be asserting that its creatures have hands.
-- These are discovered by `list_dir` across both jail roots, so this file needs
-- no entry anywhere.
--
-- ─── Optional by absence ─────────────────────────────────────────────────────
--
-- A creature with no layout behaves exactly as it did before layouts existed:
-- no location is chosen, **no roll is consumed choosing one**, `ev.hit_slot` is
-- nil and the per-slot armour guard is skipped. So adding this file does not
-- change any fight involving a creature that does not name one — which is why
-- the wisp, the crawler and the lurker below are attached one at a time rather
-- than by a default.
--
-- `size` is a hit weight and need not sum to anything; `lib/body.lua`
-- normalises. `height` is a percentage of *this creature's own* height, so one
-- layout serves a halfling and a giant. A part with no `slot` cannot be
-- armoured, which is the honest answer for a tail.

return {
    layouts = {

        -- ─── Humanoid ────────────────────────────────────────────────────────
        -- The townsfolk, the guards, the delver. Slots are drawn from
        -- `equipment.SLOTS`, so a helm protects a head and not a shin.
        humanoid = {
            features = { "hands", "feet", "eyes" },
            parts = {
                -- Small and high: hard to reach with a dagger, and a piercing
                -- weapon that does reach it hurts disproportionately.
                { id = "head",  size = 8,  height = 95, slot = "head",
                  vulnerable = { piercing = 0.25 } },
                { id = "neck",  size = 3,  height = 90, slot = "neck",
                  vulnerable = { slashing = 0.5, piercing = 0.5 } },
                { id = "chest", size = 30, height = 70, slot = "chest" },
                { id = "arms",  size = 16, height = 70, slot = "hands",
                  features = { "hands" } },
                { id = "waist", size = 12, height = 50, slot = "waist" },
                { id = "legs",  size = 22, height = 30, slot = "legs" },
                { id = "feet",  size = 9,  height = 5,  slot = "feet" },
            },
        },

        -- ─── Beast ───────────────────────────────────────────────────────────
        -- Low to the ground and with no hands, which is the point: an ability
        -- declaring `{ kind = "body_feature", feature = "hands" }` cannot be
        -- used by one of these, and says so rather than failing oddly.
        beast = {
            features = { "teeth" },
            parts = {
                { id = "head",  size = 12, height = 60, vulnerable = { piercing = 0.25 } },
                { id = "body",  size = 45, height = 45, slot = "chest" },
                { id = "legs",  size = 28, height = 20 },
                -- No `slot`: nothing armours a tail, and saying so is better
                -- than quietly letting a chest piece cover it.
                { id = "tail",  size = 15, height = 25 },
            },
        },

        -- ─── Insectile ───────────────────────────────────────────────────────
        -- The reed crawler. Many legs, so a leg hit is common and cheap.
        insectile = {
            features = { "legs", "mandibles" },
            parts = {
                { id = "head",   size = 10, height = 55, vulnerable = { crushing = 0.3 } },
                { id = "thorax", size = 30, height = 50, slot = "chest" },
                { id = "abdomen", size = 25, height = 40 },
                { id = "legs",   size = 35, height = 20, multiplier = 0.6 },
            },
        },

        -- ─── Amorphous ───────────────────────────────────────────────────────
        -- The wisp. One part, no slots, nothing to armour and nowhere
        -- meaningful to aim — which is a layout rather than an absence, because
        -- "there is nothing to aim at" is a statement worth making.
        amorphous = {
            parts = {
                { id = "mass", size = 1, height = 50 },
            },
        },
    },
}
