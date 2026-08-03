-- game/traits/core.lua — The attributes this game has.
--
-- Pure data plus a few formulas. Registered from game/init.lua; nothing here
-- knows about daemons, persistence or effects. The mudlib decides how a trait
-- behaves; this file decides which traits exist and what the numbers are.
--
-- Four kinds:
--   attribute   stores a base
--   counter     stores a current value that only events change
--   gauge       depletable, with a maximum and optional regeneration
--   derived     stores nothing at all — computed from other traits
--
-- Effects modify `attribute` and `derived` traits. They never modify a gauge
-- or a counter: a buff raises max_hp, it does not edit your current health.

return {
    -- ─── Attributes ──────────────────────────────────────────────────────────
    { id = "strength",     label = "Strength",     kind = "attribute",
      group = "attributes", default = 10, min = 1 },
    { id = "dexterity",    label = "Dexterity",    kind = "attribute",
      group = "attributes", default = 10, min = 1 },
    { id = "constitution", label = "Constitution", kind = "attribute",
      group = "attributes", default = 10, min = 1 },
    { id = "intelligence", label = "Intelligence", kind = "attribute",
      group = "attributes", default = 10, min = 1 },
    -- New in this pass. Before TRAIT_D existed, adding a stat meant editing a
    -- hardcoded list in mobile.lua — and any stat missing from that list was
    -- silently dropped on every load.
    { id = "wisdom",       label = "Wisdom",       kind = "attribute",
      group = "attributes", default = 10, min = 1 },

    -- ─── Counters ────────────────────────────────────────────────────────────
    { id = "level", label = "Level", kind = "counter",
      group = "vitals", default = 1, min = 1 },

    -- ─── Derived ─────────────────────────────────────────────────────────────
    -- A gauge's maximum is an ordinary trait, which is what makes "+10% max
    -- health" expressible without any special case anywhere.
    { id = "max_hp", label = "Max Health", kind = "derived", group = "derived",
      depends = { "constitution", "level" }, min = 1, round = "floor",
      formula = function(t)
          return 50 + t.constitution * 5 + (t.level - 1) * 10
      end },

    { id = "max_mp", label = "Max Mana", kind = "derived", group = "derived",
      depends = { "intelligence", "level" }, min = 0, round = "floor",
      formula = function(t)
          return 20 + t.intelligence * 3 + (t.level - 1) * 5
      end },

    -- The requirement, made concrete: a trait derived from another trait.
    -- Declaring `depends` is mandatory and enforced — reading a trait that is
    -- not listed raises, which is what keeps the dependency graph honest and
    -- the cycle detector truthful.
    { id = "willpower", label = "Willpower", kind = "derived", group = "derived",
      depends = { "wisdom", "level" }, round = "floor",
      formula = function(t)
          return math.floor((t.wisdom - 10) / 2) + math.floor(t.level / 2)
      end },

    -- ─── Gauges ──────────────────────────────────────────────────────────────
    -- Regeneration is computed from a timestamp when someone looks, not driven
    -- by a timer. A thousand idle players cost nothing.
    { id = "hp", label = "Health", kind = "gauge", group = "vitals",
      max = "max_hp", min = 0, round = "floor",
      regen = { rate = 1, per = 3, target = "max", offline = false } },

    { id = "mp", label = "Mana", kind = "gauge", group = "vitals",
      max = "max_mp", min = 0, round = "floor",
      regen = { rate = 1, per = 5, target = "max", offline = false } },

    -- ─── Breadth ─────────────────────────────────────────────────────────────
    -- The rest of this file exists to make each documented trait feature real
    -- rather than only described. Every one of them is used by something.

    { id = "perception", label = "Perception", kind = "attribute",
      group = "attributes", default = 10, min = 1 },
    { id = "charisma",   label = "Charisma",   kind = "attribute",
      group = "attributes", default = 10, min = 1 },

    --- A **second gauge with its own maximum**, so `max = "<trait id>"` is
    --- exercised twice and by two different shapes: `max_hp` is derived from
    --- two attributes, `max_stamina` from one and a *derived* trait.
    { id = "max_stamina", label = "Max Stamina", kind = "derived", group = "derived",
      depends = { "constitution", "carry_capacity" }, min = 1, round = "floor",
      formula = function(t)
          -- Derived-of-derived: `carry_capacity` is itself derived, so this is
          -- a two-level dependency chain and `seal` has to order all three.
          return 40 + t.constitution * 3 + math.floor(t.carry_capacity / 10)
      end },

    { id = "stamina", label = "Stamina", kind = "gauge", group = "vitals",
      max = "max_stamina", min = 0, round = "floor",
      -- `offline = true`, unlike hp and mp: stamina is the one thing it is
      -- reasonable to have got back while you were away, and having one of
      -- each proves the flag is read rather than assumed.
      regen = { rate = 1, per = 4, target = "max", offline = true } },

    --- What you can carry. Read by `lib/carry.lua`, which asks the *trait*
    --- rather than a constant — so a strength buff or a bag of holding changes
    --- it through the ordinary effect path and nothing in the item code has to
    --- know.
    { id = "carry_capacity", label = "Carry Capacity", kind = "derived",
      group = "derived", depends = { "strength" }, min = 10, round = "floor",
      formula = function(t) return 20 + t.strength * 8 end },

    --- Derived-of-derived again, and over a different pair: `willpower` is
    --- itself derived from wisdom and level.
    { id = "spell_power", label = "Spell Power", kind = "derived", group = "derived",
      depends = { "intelligence", "willpower" }, min = 0, round = "floor",
      formula = function(t)
          return math.floor(t.intelligence / 2) + t.willpower
      end },

    --- One trait per `round` mode, so all four are exercised. They differ only
    --- in rounding, which makes the difference between them visible in `score`
    --- rather than only in a unit test.
    { id = "reflex", label = "Reflex", kind = "derived", group = "derived",
      depends = { "dexterity", "perception" }, round = "floor",
      formula = function(t) return (t.dexterity + t.perception) / 3 end },

    { id = "resolve", label = "Resolve", kind = "derived", group = "derived",
      depends = { "wisdom", "level" }, round = "ceil",
      formula = function(t) return (t.wisdom + t.level) / 3 end },

    { id = "presence", label = "Presence", kind = "derived", group = "derived",
      depends = { "charisma", "level" }, round = "round",
      formula = function(t) return (t.charisma + t.level) / 3 end },

    { id = "attunement", label = "Attunement", kind = "derived", group = "derived",
      depends = { "wisdom", "intelligence" }, round = "none",
      formula = function(t) return (t.wisdom + t.intelligence) / 4 end },

    --- `hidden`: present, computed, and never shown by `score`. An internal
    --- number a game wants and a player has no business reading.
    { id = "luck_seed", label = "Luck", kind = "attribute", group = "derived",
      default = 7, min = 0, max = 99, hidden = true },
}
