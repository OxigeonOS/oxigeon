-- mudlib/lib/matching.lua — Which one did they mean?
--
-- Every "find the thing they named" call site goes through here, because the
-- alternative already has a name in this codebase: two converters that disagree
-- eventually. `attack rat`, `look rat` and `get sword` must resolve to the same
-- creature or object, or a player learns one rule and the game keeps another.
--
-- ─── The ordinal ─────────────────────────────────────────────────────────────
--
--     attack 2.rat        the second thing matching "rat"
--     look 1.black        the first thing matching "black"
--
-- Position in the match list, recomputed on every command. Not a number stored
-- on the thing: a stored id leaves *gaps* — `2.rat` no longer exists and `3.rat`
-- does — and a gap is more disorienting than a shift. It is also the form every
-- MUD player already has in their fingers.
--
-- The order is whatever order the caller hands its list in, and every caller
-- hands one that is sorted, so `1.rat` is the **oldest rat present**. That is a
-- rule that fits in a help file. The shift when one dies is explicable; an
-- arbitrary permutation would not be.
--
-- ─── Ambiguity is always reported ────────────────────────────────────────────
--
-- Never "take the first and hope". Guessing wrong on `attack` picks a fight
-- with the wrong creature, which can be fatal on its own, and the case where a
-- player has no time to disambiguate is already served without a name at all:
-- `attack` and every combat ability default to what you are already fighting.
--
-- So a bare keyword matching several things **fails, with the list**. Callers
-- get `nil, <message>` — the same shape as "no such thing" — so a call site that
-- already handles not-found handles this too and cannot silently pick.
--
-- Exposes:
--   matching.parse(text)                     -> ordinal|nil, needle
--   matching.candidates(list, needle, keys)  -> array of matching entries
--   matching.listing(needle, matches, label) -> the "matched 3" text
--   matching.choose(list, text, keys, label) -> entry|nil, why|nil

local M = {}

--- Normalise the way every existing matcher already did: lowercase, and
--- underscores to spaces so `get apprentice_dagger` and `get apprentice dagger`
--- are one spelling.
local function norm(s)
    return tostring(s):lower():gsub("_", " ")
end

--- Split `2.rat` into its ordinal and its keyword.
---
--- Only a leading run of digits followed by a dot, so a name that merely
--- contains one is left alone. `1.` with nothing after it is not an ordinal —
--- it is somebody who has not finished typing.
--- @param text string
--- @return number|nil ordinal, string needle
function M.parse(text)
    if type(text) ~= "string" then return nil, "" end
    local n, rest = text:match("^(%d+)%.(.+)$")
    if n and #rest > 0 then return tonumber(n), rest end
    return nil, text
end

--- Everything in `list` whose keys match `needle`.
---
--- `keys` is handed the entry and returns the strings it answers to — a
--- creature's name, short and template id; an item's short and template. It is
--- a function rather than a field list because a mob and an item instance keep
--- them in different places, and this file should not know either shape.
--- @param list table  array
--- @param needle string
--- @param keys function  entry -> array of strings
--- @return table  array of matching entries, in the order given
function M.candidates(list, needle, keys)
    local out = {}
    if type(list) ~= "table" or type(needle) ~= "string" or #needle == 0 then
        return out
    end
    local want = norm(needle)

    for _, entry in ipairs(list) do
        local ok, fields = pcall(keys, entry)
        if ok and type(fields) == "table" then
            for _, field in ipairs(fields) do
                if type(field) == "string" and norm(field):find(want, 1, true) then
                    out[#out + 1] = entry
                    break
                end
            end
        end
    end
    return out
end

--- The "which one?" text.
---
--- Deliberately short. A disambiguation prompt is read in the middle of a fight
--- and anything longer than the list itself is noise — which is also why the
--- ordinal is `2.rat` and not a handle a player has to copy.
--- @param needle string
--- @param matches table
--- @param label function  entry -> what to show beside the ordinal
--- @return string
function M.listing(needle, matches, label)
    local lines = { needle .. " matched " .. #matches .. ":" }
    for i, entry in ipairs(matches) do
        local ok, text = pcall(label, entry)
        lines[#lines + 1] = "  " .. i .. "." .. needle .. "  "
            .. ((ok and type(text) == "string") and text or "something")
    end
    return table.concat(lines, "\r\n")
end

--- The one they meant, or why not.
---
--- Three outcomes, and the second is the reason this file exists:
---
---   one match, or an ordinal that lands   the entry
---   several matches and no ordinal        nil, the listing
---   none, or an ordinal past the end      nil, nil — the caller's own wording
---
--- The third returns no message because "you do not see that here" reads better
--- in each command's own voice, and every call site already has one.
--- ─── Fungible things are never ambiguous ────────────────────────────────────
---
--- `fungible` returns a key for an entry that is interchangeable with others
--- carrying the same key, and nil for one that is not. When every match agrees
--- on a non-nil key there is nothing to ask: three stackable roots are the same
--- root, and `drop marshroot` refusing until you write `drop 1.marshroot` is
--- friction bought with nothing.
---
--- Read off a declared property — an item's `stackable` — rather than from a
--- list of commands that may skip the question. Creatures supply no `fungible`,
--- so two identical rats are always two rats.
--- @param list table
--- @param text string  may carry an ordinal
--- @param keys function
--- @param label function
--- @param fungible function|nil  entry -> key|nil
--- @return any entry, string|nil why
function M.choose(list, text, keys, label, fungible)
    local ordinal, needle = M.parse(text)
    local matches = M.candidates(list, needle, keys)

    if #matches == 0 then return nil, nil end

    if ordinal then
        -- An ordinal past the end is a miss rather than a wrap or a clamp: the
        -- player named a specific thing and it is not there.
        return matches[ordinal], nil
    end

    if #matches == 1 then return matches[1], nil end

    if type(fungible) == "function" then
        local key, same = nil, true
        for i, entry in ipairs(matches) do
            local ok, k = pcall(fungible, entry)
            if not ok or k == nil then same = false break end
            if i == 1 then key = k elseif k ~= key then same = false break end
        end
        if same then return matches[1], nil end
    end

    return nil, M.listing(needle, matches, label)
end

return M
