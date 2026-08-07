-- mudlib/lib/render.lua — One line, authored once, read per viewer.
--
--     "$Actor $actor.v(swing) $weapon.of(actor) at $target."
--
--     the attacker sees   You swing your longsword at the pale wisp.
--     the defender sees   Wren swings her longsword at you.
--     an onlooker sees    Wren swings her longsword at the pale wisp.
--
-- Before this every caller wrote two or three strings by hand and kept them in
-- step by remembering to. `combat_d` had a private `display_name`, `ability_d`
-- had a private `$token` substitution, and neither could say "you".
--
-- ─── Why the sigil is `$` ────────────────────────────────────────────────────
--
-- Because `ability_d.messages` already ships `$name $target $dealt`. Choosing
-- `$` means this **subsumes** that substitution rather than competing with it:
-- no second syntax, no deprecation, no dual parse. Every message already
-- authored is already valid input and simply gains meaning.
--
-- Not `{actor}`: `lib/color.lua` matches `{(.-)}` across the whole string, so
-- `{actor}` and `{red}` would be indistinguishable in the source. Nothing in
-- the text would say which consumer owns a tag, and it would break silently the
-- first time somebody added a colour alias named `target`. That is the same
-- shape as the `cmd.olc`-versus-`olc` collision CLAUDE.md already records.
--
-- Not `%actor%`: `%` is both Lua's pattern escape and `gsub`'s replacement
-- escape, so it puts a doubling hazard inside the parser.
--
-- ─── The grammar ─────────────────────────────────────────────────────────────
--
--   $dealt              a scalar out of ctx                 -> "7"
--   $actor              a reference        "you" | "Wren" | "the pale wisp"
--   $Actor              the same, capitalised
--   $actor.name         always the display name, even to the actor
--   $actor.they         subjective        "you" | "she" | "they" | "it"
--   $actor.them  .their  .theirs  .themself
--   $actor.v(swing)     the verb agreeing with `actor`
--   $weapon.of(actor)   "your longsword" | "her longsword"
--   $$                  a literal dollar
--
-- The verb **names its subject**. Three extra characters buy two things: a
-- second clause is writable (`"$Target $target.v(be) dead."`), and the fact
-- that agreement is a property of the *subject's* pronoun set rather than of
-- the verb is visible in the source, which is the one thing about this that
-- surprises people.
--
-- Capitalisation is an explicit flag rather than sentence detection, because
-- `"{red}$actor strikes{/}"` has its token at byte 6 and colour tags are
-- everywhere in this codebase. Guessing is worse than a shift key.
--
-- ─── An unknown token survives verbatim ──────────────────────────────────────
--
-- `$victim` stays `$victim`; `$actor.blah` stays `$actor.blah`; a table with no
-- name of any kind stays as its token rather than printing an address. The rule
-- `lib/abilities.lua` already had, generalised: *"You strike $victim" is a typo
-- somebody can see and fix; "You strike " is a bug they will stare at.* Emitting
-- an empty string for a form that could not apply is the same bug wearing a hat.
--
-- Exposes:
--   render.parse(template)                     -> compiled  (memoised)
--   render.render(template, ctx, viewer)       -> string
--   render.render_for(template, ctx, viewers)  -> { [viewer] = string }
--   render.display_name(entity)                -> string
--   render.is_same(a, b)                       -> boolean
--   render.flush_cache()
--
-- See docs/src/lua-api/messages.md.

local Grammar = require('lib.grammar')
local Object  = require('lib.object')

local M = {}

--- Past this many distinct templates the cache is wiped and restarted.
---
--- A plain table rather than a weak one: `__mode = "v"` would recompile a hot
--- template after every collection, and authored templates are a fixed finite
--- set. The cap only bounds a caller generating templates at runtime, which
--- nothing does today.
local MAX_CACHED = 512

local _cache, _cached = {}, 0

--- Which field names something in a sentence. Read once and remembered, because
--- `display_name` is called for every participant of every swing of every fight
--- and `config` crosses into Rust.
local _prefers = nil

function M.flush_cache()
    _cache, _cached = {}, 0
    _prefers = nil
end

--- `"name"` or `"short"`, from `game.display_name_prefers`.
---
--- A game decision, not an engine one, which is why it is configuration and not
--- a rule. A creature carries both — `greywater_wisp` is `name = "wisp"` and
--- `short = "a pale wisp"` — because they answer different questions: `name` is
--- what you type to attack it, `short` is what it reads as in prose.
---
--- Which one a *message* should use depends entirely on the game. "You hit the
--- pale wisp for 9" is right for a roleplay MUD; "You hit wisp for 9" is right
--- for a hack-and-slash where the same noun you typed comes back at you. Neither
--- is more correct, so neither is hardcoded.
---
--- Defaults to `"name"`, which is what `combat_d` did before this existed.
local function prefers()
    if _prefers then return _prefers end
    local ok, v = pcall(config, "game.display_name_prefers")
    _prefers = (ok and v == "short") and "short" or "name"
    return _prefers
end

-- ─── Identity ────────────────────────────────────────────────────────────────

--- Is this viewer this role?
---
--- Identity first, then ids, because `DAEMON.character.get` hands back the same
--- live table but a rehydrated player may not be the one a caller captured.
--- @return boolean
function M.is_same(a, b)
    if a == nil or b == nil then return false end
    if a == b then return true end
    if type(a) ~= "table" or type(b) ~= "table" then return false end
    if a.char_id ~= nil and a.char_id == b.char_id then return true end
    if a.id ~= nil and a.id == b.id then return true end
    return false
end

--- What something is called, in one place.
---
--- Falls back to the other field when the preferred one is absent, so a thing
--- with only a `short` and a thing with only a `name` both work under either
--- setting. `short` resolves through `Object.resolve`, because a description
--- that changes with the weather is still a description.
--- @param entity table|nil
--- @return string
function M.display_name(entity)
    if type(entity) ~= "table" then return tostring(entity) end

    local name = type(entity.name) == "string" and entity.name ~= "" and entity.name or nil
    local short = Object.resolve(entity.short, entity)
    if type(short) ~= "string" or short == "" then short = nil end

    if prefers() == "short" then return short or name or "something" end
    return name or short or "something"
end

--- Can this value stand in for somebody or something in a sentence?
---
--- The guard that stops `$ability` printing `table: 0x7f…`. A table nothing can
--- name is not renderable, and an unrenderable token survives verbatim.
local function nameable(v)
    return type(v) == "table"
        and (v.name ~= nil or v.short ~= nil or v.id ~= nil or v.char_id ~= nil)
end

-- ─── Parsing ─────────────────────────────────────────────────────────────────

--- Split a template into literals and token records.
---
--- A token is `$` then an identifier, optionally `.form`, optionally `(arg)`.
--- The identifier runs to the end of the word, so `$target_name` is one token
--- rather than `$target` followed by `_name` — which is what the old
--- substitution did, and it was wrong.
--- @param template string
--- @return table  array of strings and { role, form, arg, caps }
function M.parse(template)
    if type(template) ~= "string" then return {} end

    local hit = _cache[template]
    if hit then return hit end

    local out, at = {}, 1
    while true do
        local s = template:find("%$", at)
        if not s then break end

        if s > at then out[#out + 1] = template:sub(at, s - 1) end

        -- `$$` is a literal dollar and consumes both.
        if template:sub(s + 1, s + 1) == "$" then
            out[#out + 1] = "$"
            at = s + 2
        else
            local raw = template:match("^%$([%a_][%w_]*)", s)
            if not raw then
                out[#out + 1] = "$"
                at = s + 1
            else
                local pos = s + 1 + #raw
                local form, arg

                local f, after = template:match("^%.([%a_][%w_]*)()", pos)
                if f then
                    -- Lowercased, so `$Actor.They` and `$Actor.they` are the
                    -- same token. Capitalisation comes from the *role's* initial
                    -- and from nowhere else, which is one rule rather than two
                    -- that could disagree.
                    form = f:lower()
                    pos = after
                    local a, after_arg = template:match("^%(([^%)]*)%)()", pos)
                    if a then
                        arg = a
                        pos = after_arg
                    end
                end

                local first = raw:sub(1, 1)
                out[#out + 1] = {
                    role = raw,
                    lower = first:lower() .. raw:sub(2),
                    form = form,
                    arg  = arg,
                    caps = first:match("%u") ~= nil,
                    -- The exact source text, so an unknown token can be handed
                    -- back byte for byte rather than reconstructed.
                    raw  = template:sub(s, pos - 1),
                }
                at = pos
            end
        end
    end

    if at <= #template then out[#out + 1] = template:sub(at) end

    if _cached >= MAX_CACHED then M.flush_cache() end
    _cache[template] = out
    _cached = _cached + 1
    return out
end

-- ─── Rendering ───────────────────────────────────────────────────────────────

--- Look a role up, accepting the token as typed and as lowercased.
local function lookup(ctx, token)
    if type(ctx) ~= "table" then return nil end
    local v = ctx[token.lower]
    if v ~= nil then return v end
    return ctx[token.role]
end

--- One token, for one viewer. Returns nil to mean "leave it verbatim".
local function one(token, ctx, viewer)
    local value = lookup(ctx, token)
    if value == nil then return nil end

    if type(value) == "function" then
        local ok, produced = pcall(value, ctx, viewer)
        if not ok or produced == nil then return nil end
        value = produced
    end

    local form = token.form

    -- A table nothing can name is not renderable at all, so the token survives.
    -- Without this `$ability` — an ordinary key in `ability_d`'s ctx — prints an
    -- address into a player's face, which is the general form of the `$target`
    -- bug this layer was written to fix.
    if type(value) == "table" and not nameable(value) then return nil end

    -- A plain scalar. Only the bare form and `.name` mean anything for one.
    if type(value) ~= "table" then
        if form == nil or form == "name" then return tostring(value) end
        return nil
    end

    local is_viewer = M.is_same(value, viewer)
    local set = is_viewer and Grammar.VIEWER or Grammar.set_for(value)

    if form == nil then
        if is_viewer then return "you" end
        return M.display_name(value)
    end

    if form == "name" then return M.display_name(value) end

    if set[form] ~= nil and form ~= "plural" then return set[form] end

    -- `$actor.v(swing)` agrees with how `$actor` itself renders — "you swing",
    -- "Ash swings", "the rats swarm". A name is third-person singular whatever
    -- its owner's pronouns are, which is why this reads `collective` and not
    -- `plural`: otherwise a they/them person's *name* would take plural
    -- agreement and the game would say "Ash swing".
    if form == "v" then
        if type(token.arg) ~= "string" or token.arg == "" then return nil end
        return Grammar.conjugate(token.arg, is_viewer or set.collective)
    end

    -- `$actor.vthey(be)` agrees with the **pronoun**, for the rarer construction
    -- where the pronoun is the subject: `"$Actor.They $actor.vthey(be) bleeding."`
    -- -> "You are bleeding" / "They are bleeding" / "She is bleeding".
    if form == "vthey" then
        if type(token.arg) ~= "string" or token.arg == "" then return nil end
        return Grammar.conjugate(token.arg, set.plural)
    end

    if form == "of" then
        local owner = token.arg and ctx and (ctx[token.arg])
        local name = Grammar.strip_article(M.display_name(value))
        if not nameable(owner) then return M.display_name(value) end
        local owner_set = M.is_same(owner, viewer) and Grammar.VIEWER or Grammar.set_for(owner)
        return owner_set.their .. " " .. name
    end

    return nil
end

--- One template, one reader.
--- @param template string
--- @param ctx table|nil    role -> entity | item | scalar | function
--- @param viewer table|nil nil means nobody is "you"
--- @return string
function M.render(template, ctx, viewer)
    if type(template) ~= "string" then return "" end

    local parts = M.parse(template)
    local out = {}
    for i = 1, #parts do
        local part = parts[i]
        if type(part) == "string" then
            out[#out + 1] = part
        else
            local text = one(part, ctx, viewer)
            if text == nil then
                out[#out + 1] = part.raw
            else
                out[#out + 1] = part.caps and Grammar.capitalise(text) or text
            end
        end
    end
    return table.concat(out)
end

--- The same line for a group, rendered once per **distinct role set**.
---
--- A forty-person room watching one attacker hit one target does three renders,
--- not forty: everybody who is neither participant reads the same sentence. The
--- key is built from the (typically two or three) entity-valued roles, so this
--- is cheaper than the character lookup a caller has to do anyway.
--- @param viewers table  array of entities
--- @return table  { [viewer] = string }
function M.render_for(template, ctx, viewers)
    local parts = M.parse(template)

    -- Which roles could make a viewer say "you" at all.
    local roles = {}
    for _, part in ipairs(parts) do
        if type(part) == "table" then
            local v = lookup(ctx, part)
            if nameable(v) then roles[#roles + 1] = { key = part.lower, value = v } end
        end
    end
    table.sort(roles, function(a, b) return a.key < b.key end)

    local by_key, out = {}, {}
    for _, viewer in ipairs(viewers or {}) do
        local sig = {}
        for _, r in ipairs(roles) do
            if M.is_same(r.value, viewer) then sig[#sig + 1] = r.key end
        end
        local key = table.concat(sig, ",")

        local text = by_key[key]
        if text == nil then
            text = M.render(template, ctx, viewer)
            by_key[key] = text
        end
        out[viewer] = text
    end
    return out
end

return M
