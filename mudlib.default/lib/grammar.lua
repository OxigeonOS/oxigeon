-- mudlib/lib/grammar.lua — The English in the messages.
--
-- One authored line has to read correctly for everybody who sees it:
--
--     "You swing your longsword at the pale wisp."
--     "Wren swings her longsword at you."
--     "Wren swings her longsword at the pale wisp."
--
-- That needs two things this file owns and nothing else does: which pronouns a
-- creature takes, and which form a verb takes for a given subject. Both are
-- facts about **English**, not about this game — so a game translating to
-- another language replaces exactly this file and keeps `lib/render.lua`.
--
-- ─── `plural` is not "how many" ──────────────────────────────────────────────
--
-- The one idea worth reading twice. A pronoun set's `plural` flag does not mean
-- "this is several things". It means **which verb form this subject takes**, and
-- English gives the same answer for the second person and for singular they:
--
--     you swing   they swing   |   she swings   it swings
--     you are     they are     |   she is       it is
--
-- So one boolean covers both, and the conjugator never has to learn about
-- person at all. It is also why `neutral` produces "they swing" and never "they
-- swings": `neutral.plural` is true for the same reason the viewer's is.
--
-- `neutral` and `plural` then differ in exactly one cell — **themself** against
-- **themselves**. One person whose pronouns are they/them, against a swarm of
-- rats. That is one table row, and it is the distinction people notice.
--
-- ─── What an entity with no gender gets ──────────────────────────────────────
--
-- A creature: `thing`. An ungendered creature is a wisp or a grey rat, and "it
-- bites you" is right where "they bite you" about one rat reads as a bug.
--
-- A **player**: `neutral`. An ungendered player is a person who has not said,
-- and "it swings" about a person is offensive where "they swing" is
-- unremarkable. Nothing in the mudlib sets `gender` at character creation, so
-- every player alive is in this bucket — it is the default path, not an edge.
--
-- Exposes:
--   Grammar.SETS / Grammar.VIEWER / Grammar.FORMS
--   Grammar.set_for(entity)          -> a pronoun set
--   Grammar.define_set(name, tbl)    -> boolean
--   Grammar.conjugate(verb, plural)  -> string
--   Grammar.capitalise(text)         -> string
--   Grammar.strip_article(text)      -> string
--
-- See docs/src/lua-api/messages.md.

local M = {}

--- The forms a role may be asked for, and the words are the neutral set spelled
--- out. `$actor.their sword` reads correctly at a glance, and nobody has to
--- remember whether `.poss` meant the determiner or the pronoun.
M.FORMS = { "they", "them", "their", "theirs", "themself" }

--- Two flags, because English asks the agreement question twice.
---
---   `plural`      how a verb agrees when **the pronoun** is the subject.
---                 "they swing", "she swings", "you swing".
---   `collective`  how a verb agrees when **the name** is the subject.
---                 "Ash swings", "the rats swarm".
---
--- One flag will not do it, and the case that proves it is a person whose
--- pronouns are they/them: *"they swing"* is right and *"Ash swing"* is not.
--- A name is third-person singular whatever its owner's pronouns are — the only
--- thing that takes plural agreement by name is something that genuinely is
--- many.
M.SETS = {
    male    = { they = "he",   them = "him",  their = "his",
                theirs = "his",    themself = "himself",
                plural = false, collective = false },
    female  = { they = "she",  them = "her",  their = "her",
                theirs = "hers",   themself = "herself",
                plural = false, collective = false },
    neutral = { they = "they", them = "them", their = "their",
                theirs = "theirs", themself = "themself",
                plural = true,  collective = false },
    thing   = { they = "it",   them = "it",   their = "its",
                theirs = "its",    themself = "itself",
                plural = false, collective = false },
    plural  = { they = "they", them = "them", their = "their",
                theirs = "theirs", themself = "themselves",
                plural = true,  collective = true  },
}

--- What a role renders as to the person reading the line.
---
--- `collective` is true because the reader is always "you", and "you" takes the
--- same form whether it is one person or a crowd.
M.VIEWER = { they = "you", them = "you", their = "your",
             theirs = "yours", themself = "yourself",
             plural = true, collective = true }

-- ─── Conjugation ─────────────────────────────────────────────────────────────

--- The ones no rule reaches, in the third person singular.
local IRREGULAR = {
    be = "is", have = "has", ["do"] = "does", go = "goes",
}

--- And the ones no rule reaches in the *plural* form either.
---
--- Almost the whole point of `plural` is that it takes the bare stem — "you
--- swing", "they have", "they go". `be` is the single verb in English where
--- that is false, and it is false for both halves at once: "you **are**",
--- "they **are**", against "she **is**". One entry, and it is the entry every
--- death message in the game goes through ("You are dead", "It is dead"), so
--- getting it from the same flag as everything else is worth the table.
local IRREGULAR_PLURAL = {
    be = "are",
}

--- Modals do not inflect. "she can", never "she cans".
local MODAL = {
    can = true, will = true, may = true, must = true, shall = true,
    might = true, could = true, would = true, should = true, ought = true,
}

local _memo = {}

--- The form of a verb for a subject.
--- @param verb string
--- @param plural boolean  the subject's `plural` flag, NOT a count
--- @return string
function M.conjugate(verb, plural)
    if type(verb) ~= "string" or verb == "" then return tostring(verb or "") end

    local key = verb .. (plural and "|p" or "|s")
    local hit = _memo[key]
    if hit then return hit end

    local out
    if plural then
        -- The bare stem, with one exception. "you swing", "they swing".
        out = IRREGULAR_PLURAL[verb] or verb
    elseif MODAL[verb] then
        out = verb
    elseif IRREGULAR[verb] then
        out = IRREGULAR[verb]
    elseif verb:match("[^aeiou]y$") then
        out = verb:sub(1, -2) .. "ies"            -- parry -> parries, fly -> flies
    elseif verb:match("s$") or verb:match("sh$") or verb:match("ch$")
        or verb:match("x$") or verb:match("z$") or verb:match("o$") then
        out = verb .. "es"                        -- kiss, push, catch, mix, buzz, echo
    else
        out = verb .. "s"                         -- swing -> swings, say -> says
    end

    _memo[key] = out
    return out
end

--- Drop the conjugation memo. For a hot reload, and for a test that redefines a
--- set. Nothing else should need it.
function M.flush_cache()
    _memo = {}
end

-- ─── Which set an entity takes ───────────────────────────────────────────────

--- Add a pronoun set from the game layer.
---
--- A registry rather than a fixed table, the same shape as
--- `Abilities.checks()` — and the reason is the same: the thing that rots is a
--- central list somewhere else saying which set applies to whom, and there is
--- none. Missing forms are filled from `neutral` so a partial set is usable
--- rather than a source of nils.
--- @return boolean
function M.define_set(name, tbl)
    if type(name) ~= "string" or name == "" or type(tbl) ~= "table" then
        return false
    end
    local set = {
        plural = tbl.plural and true or false,
        collective = tbl.collective and true or false,
    }
    for _, form in ipairs(M.FORMS) do
        set[form] = type(tbl[form]) == "string" and tbl[form] or M.SETS.neutral[form]
    end
    M.SETS[name] = set
    return true
end

--- Fill a partial pronoun table into a whole one.
local function complete(tbl)
    local set = { plural = tbl.plural, collective = tbl.collective }
    if set.plural == nil then set.plural = M.SETS.neutral.plural end
    if set.collective == nil then set.collective = M.SETS.neutral.collective end
    for _, form in ipairs(M.FORMS) do
        set[form] = type(tbl[form]) == "string" and tbl[form] or M.SETS.neutral[form]
    end
    return set
end

--- Which pronoun set this entity takes.
---
--- `pronouns` is read before `gender` so a game can move to a better-named field
--- at its own pace with no save-format change and no migration code. "Gender" is
--- the wrong word for a pronoun set, but it is in `Player.SAVE_FIELDS` and in
--- `schema/mob.lua`, and renaming a saved field buys nothing.
---
--- A table in `pronouns` is used verbatim, so a game gets neopronouns by writing
--- five strings and needs nothing at all from the mudlib.
--- @param entity table|nil
--- @return table  a complete pronoun set
function M.set_for(entity)
    if type(entity) ~= "table" then return M.SETS.thing end

    local p = entity.pronouns
    if type(p) == "table" then return complete(p) end
    if type(p) == "string" and M.SETS[p] then return M.SETS[p] end

    local g = entity.gender
    if type(g) == "string" and M.SETS[g] then return M.SETS[g] end

    -- The only thing that distinguishes a person from a rat here, and it is the
    -- same test `scope_of` uses in combat_d, effect_d and ability_d.
    if entity.char_id ~= nil then return M.SETS.neutral end
    return M.SETS.thing
end

-- ─── Text shaping ────────────────────────────────────────────────────────────

--- Capitalise, stepping over any leading colour tags.
---
--- `"{cyan}a wisp"` has to become `"{cyan}A wisp"`. Capitalising byte one would
--- put a capital on a brace and leave the sentence lowercase, which is exactly
--- the failure that makes people stop trusting the layer.
--- @param text string
--- @return string
function M.capitalise(text)
    if type(text) ~= "string" or text == "" then return text or "" end

    local at = 1
    while true do
        local _, stop = text:find("^{[^}]*}", at)
        if not stop then break end
        at = stop + 1
    end
    if at > #text then return text end

    return text:sub(1, at - 1) .. text:sub(at, at):upper() .. text:sub(at + 1)
end

--- Drop a leading article, so `"a longsword"` can take a possessive: "your
--- longsword", never "your a longsword".
--- @param text string
--- @return string
function M.strip_article(text)
    if type(text) ~= "string" then return text or "" end
    return (text:gsub("^([Aa]n?%s+)", ""):gsub("^([Tt]he%s+)", ""))
end

return M
