-- game/traits/broken_example.lua — Deliberately broken. Loaded only by a test.
--
-- `seal()` reports a cycle **as a path** and a missing dependency **by name**,
-- because "there is a cycle somewhere in your thirty traits" is not something
-- anybody can act on. This file is what proves those messages are produced, and
-- that a broken trait file does not take the server down — the same guarantee a
-- broken area file has.
--
-- It is **not** registered by `game/init.lua`. A game that shipped this would
-- have three broken traits for no reason; a test that has to hand-write them
-- would be testing its own fixture rather than the game's.

return {
    --- A dependency on something that does not exist. `seal` marks it failed,
    --- names what is missing, and the trait answers with its default.
    { id = "broken_dangling", label = "Dangling", kind = "derived",
      group = "broken", depends = { "no_such_trait" }, default = 3,
      formula = function(t) return t.no_such_trait * 2 end },

    --- A three-trait cycle. Reported as `a -> b -> c -> a`, which names every
    --- link so the one to break is obvious.
    { id = "broken_cycle_a", label = "Cycle A", kind = "derived",
      group = "broken", depends = { "broken_cycle_b" }, default = 1,
      formula = function(t) return t.broken_cycle_b + 1 end },

    { id = "broken_cycle_b", label = "Cycle B", kind = "derived",
      group = "broken", depends = { "broken_cycle_c" }, default = 1,
      formula = function(t) return t.broken_cycle_c + 1 end },

    { id = "broken_cycle_c", label = "Cycle C", kind = "derived",
      group = "broken", depends = { "broken_cycle_a" }, default = 1,
      formula = function(t) return t.broken_cycle_a + 1 end },

    --- Not broken, and that is the point: it depends on a real trait and must
    --- keep working while the three above are failed. One bad trait must not
    --- disable the other thirty.
    { id = "broken_bystander", label = "Bystander", kind = "derived",
      group = "broken", depends = { "wisdom" }, round = "floor",
      formula = function(t) return t.wisdom * 2 end },
}
