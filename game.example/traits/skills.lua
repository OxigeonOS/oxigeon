-- game/traits/skills.lua — What a character can learn.
--
-- Skills are traits, and deliberately not in any seed set: not having
-- swordsmanship until you learn it is the whole point of sparse traits. A
-- character starts with none of these, `set_base` is how one is learned, and
-- storage is what decides they are present from then on.
--
-- The three axes, on one line each:
--
--   kind      what is stored and how it is computed — counter, derived
--   category  what the number *is* in this game's vocabulary — "skill"
--   group     which heading it sorts under inside the `skills` command
--
-- `swordsmanship` is a counter and `sword_mastery` is a derived percentage over
-- it: different `kind`, same `category`, same `group`. No one field expresses
-- that, which is why there are three.
--
-- `category` never changes behaviour. It decides which command lists a trait
-- and nothing else — `score` names `stat`, `skills` names `skill`, `traits`
-- names everything. The moment a category is tempted to *mean* something
-- ("skills advance by use"), that belongs on the spec as its own declared
-- field, not implied by a string.

--- Diminishing: the last ten points of skill are worth less than the first ten,
--- which is the usual shape and needs no engine support.
local function sword_mastery_formula(t)
    return math.floor(t.swordsmanship * 0.6)
end

return {
    -- ─── Weapon skills ───────────────────────────────────────────────────────
    -- Counters: a number events change, clamped to its bounds. Practice raises
    -- it; nothing derives it.
    { id = "swordsmanship", label = "Swordsmanship", kind = "counter",
      category = "skill", group = "weapon", sets = false,
      default = 0, min = 0, max = 100 },

    { id = "archery", label = "Archery", kind = "counter",
      category = "skill", group = "weapon", sets = false,
      default = 0, min = 0, max = 100 },

    -- Derived over a counter, and present only when that counter is. A
    -- character who has never touched a sword has neither, and pays for
    -- neither on a recompute.
    { id = "sword_mastery", label = "Sword Mastery", kind = "derived",
      category = "skill", group = "weapon", sets = false, depends = { "swordsmanship" },
      round = "floor", min = 0,
      formula = sword_mastery_formula },

    -- ─── Craft skills ────────────────────────────────────────────────────────
    { id = "herbalism", label = "Herbalism", kind = "counter",
      category = "skill", group = "craft", sets = false,
      default = 0, min = 0, max = 100 },

    { id = "mining", label = "Mining", kind = "counter",
      category = "skill", group = "craft", sets = false,
      default = 0, min = 0, max = 100 },
}
