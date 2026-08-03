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
}
