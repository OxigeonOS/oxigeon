-- mudlib/lib/serialize.lua — Lua values back into Lua source.
--
-- What OLC writes. Every generated area file goes through here, so the property
-- that matters is not "does it look right" but:
--
--     load(emit(v))() == v          for every v this accepts
--     emit(load(emit(v))()) == emit(v)      byte for byte
--
-- The second is the one that catches things. `codegen_d` built its output by
-- concatenating strings — `'short = "' .. data.short .. '",'` — so a room whose
-- title contained a quote produced a file that would not compile, and its
-- multi-line branch indented the closing `]]`, which put four spaces inside the
-- string and grew them on every read-write cycle for ever.
--
-- ─── Why not lib/jsonsafe.lua ────────────────────────────────────────────────
--
-- It answers a different question with a different rule set. `jsonsafe` refuses
-- a table that is simultaneously a list and a map because *JSON* has no such
-- type — Lua does, and a mob's `loot_table` beside its `count` is ordinary here.
-- It also has nothing to say about float round-tripping, which is most of the
-- hard part below. And its contract is pinned to `lua_to_json` by
-- `tests/state_cache.rs`, so widening it to serve a second consumer would break
-- a pact that exists for a reason.
--
-- The breadcrumb helpers are duplicated from it rather than shared, so the two
-- produce error messages that read alike without either owning the other. Six
-- lines is cheaper than a third module that exists to hold six lines.
--
-- See docs/src/lua-api/olc.md.

local M = {}

--- How deep a value may nest. A pathological structure should error rather than
--- eat the C stack — the same reasoning as `jsonsafe.MAX_DEPTH`.
M.MAX_DEPTH = 32

--- A scalar array shorter than this renders on one line.
M.INLINE_WIDTH = 78

local KEYWORDS = {
    ["and"] = true, ["break"] = true, ["do"] = true, ["else"] = true,
    ["elseif"] = true, ["end"] = true, ["false"] = true, ["for"] = true,
    ["function"] = true, ["goto"] = true, ["if"] = true, ["in"] = true,
    ["local"] = true, ["nil"] = true, ["not"] = true, ["or"] = true,
    ["repeat"] = true, ["return"] = true, ["then"] = true, ["true"] = true,
    ["until"] = true, ["while"] = true,
}

--- A breadcrumb, spelled the way `jsonsafe` spells it.
local function join(path, key)
    if path == "" then return tostring(key) end
    if type(key) == "number" then return path .. "[" .. tostring(key) .. "]" end
    return path .. "." .. tostring(key)
end

local function finite(n)
    return n == n and n ~= math.huge and n ~= -math.huge
end

-- ─── Scalars ─────────────────────────────────────────────────────────────────

--- Can this string be a bare table key?
---
--- Keywords are excluded as well as non-identifiers: `end = 1` is a syntax
--- error, and a room with an `items.end` scenery keyword would otherwise emit a
--- file that does not compile.
--- @param s any
--- @return boolean
function M.is_identifier(s)
    if type(s) ~= "string" then return false end
    if KEYWORDS[s] then return false end
    return s:match("^[%a_][%w_]*$") ~= nil
end

--- A string as a double-quoted Lua literal.
---
--- Control bytes become `\ddd` with **three digits always**. Two would be
--- shorter and wrong: `"\0" .. "1"` emitted as `\01` reads back as byte 1
--- followed by nothing, because Lua takes up to three digits and `01` is a
--- complete escape only if the next character is not a digit.
---
--- Bytes at or above 0x80 pass through untouched — that is what UTF-8 prose is,
--- and escaping it would turn every accented character in a description into
--- four unreadable digits.
--- @param s string
--- @return string
function M.quote(s)
    local out = s:gsub("[\\\"]", "\\%0")
    out = out:gsub("\n", "\\n"):gsub("\r", "\\r"):gsub("\t", "\\t")
    out = out:gsub("[%z\1-\8\11\12\14-\31\127]", function(c)
        return string.format("\\%03d", c:byte())
    end)
    return '"' .. out .. '"'
end

--- The smallest long-bracket level that can safely hold `s`, or nil.
---
--- Both `]==]` *and* `[==[` are scanned for. Only the closing form can truncate
--- the literal, but a `[==[` inside would nest and change where it ends, and the
--- cost of skipping a level is one character.
---
--- Returns nil when a long bracket cannot hold the string at all: a `\r` would
--- be normalized by the parser, and a trailing `]` would run into the closing
--- bracket. Those fall back to a quoted literal, which is always correct.
--- @param s string
--- @return string|nil open, string|nil close
function M.long_bracket(s)
    if s:find("\r", 1, true) then return nil end
    if s:sub(-1) == "]" then return nil end

    for level = 0, 8 do
        local eq = string.rep("=", level)
        if not s:find("]" .. eq .. "]", 1, true)
           and not s:find("[" .. eq .. "[", 1, true) then
            return "[" .. eq .. "[", "]" .. eq .. "]"
        end
    end
    return nil
end

--- A number as source that reads back as exactly this number.
---
--- Three traps, all of which produced wrong output before they were handled:
---
--- * **NaN and the infinities** have no literal. `0/0` is not a number token,
---   and emitting `inf` produces a nil global.
--- * **An integral float must keep its point.** On 5.3+ `math.type(2.0)` is
---   "float" and `math.type(2)` is "integer"; emitting `2` for `2.0` silently
---   changes the type, and `speed = 1.0` would come back as an integer.
--- * **`%.17g` is not the answer.** It round-trips every double, and renders
---   every authored `speed = 1.2` as `1.1999999999999999`. Trying increasing
---   precision and stopping at the first that reads back equal gives `1.2` for
---   1.2 and full precision only where it is needed.
--- @param n number
--- @return string|nil source, string|nil err
function M.number(n)
    if not finite(n) then
        return nil, tostring(n) .. " has no Lua literal (NaN and the infinities do not)"
    end

    -- 5.3+ has an integer subtype; LuaJIT and 5.1 do not.
    if math.type then
        if math.type(n) == "integer" then return string.format("%d", n) end
    elseif n % 1 == 0 and math.abs(n) < 2 ^ 53 then
        return string.format("%d", n)
    end

    -- A float. Integral ones need the point kept.
    if n % 1 == 0 and math.abs(n) < 2 ^ 53 then
        return string.format("%.1f", n)
    end

    for _, fmt in ipairs({ "%.14g", "%.15g", "%.16g", "%.17g" }) do
        local text = string.format(fmt, n)
        if tonumber(text) == n then return text end
    end
    return nil, "no representation round-trips for " .. tostring(n)
end

-- ─── Whether a value can be written at all ───────────────────────────────────

local function check(value, depth, path, ancestors)
    local t = type(value)

    if t == "nil" or t == "boolean" or t == "string" then return true end
    if t == "number" then
        local _, err = M.number(value)
        if err then
            return false, (path == "" and "the value" or path) .. ": " .. err
        end
        return true
    end
    if t ~= "table" then
        return false, (path == "" and "the value" or path)
            .. " is a " .. t .. ", which has no source form"
    end

    if depth > M.MAX_DEPTH then
        return false, "nesting deeper than " .. M.MAX_DEPTH .. " at "
            .. (path == "" and "the top level" or path)
    end
    -- On the *ancestor* path, not globally: the same subtable appearing twice
    -- is legal and is written twice, while a table that is its own ancestor has
    -- no bottom.
    if ancestors[value] then
        return false, (path == "" and "the value" or path) .. " contains itself"
    end
    ancestors[value] = true

    for k, v in pairs(value) do
        local kt = type(k)
        if kt ~= "string" and kt ~= "number" then
            return false, join(path, tostring(k)) .. ": a " .. kt
                .. " cannot be a table key in source"
        end
        if kt == "number" and not finite(k) then
            return false, (path == "" and "the value" or path)
                .. " has " .. tostring(k) .. " as a key"
        end
        local ok, err = check(v, depth + 1, join(path, k), ancestors)
        if not ok then
            ancestors[value] = nil
            return false, err
        end
    end

    ancestors[value] = nil
    return true
end

--- Could this value be written as source?
---
--- Same shape as `jsonsafe.check`, and deliberately not the same rules — see the
--- note at the top of this file. Names the offending path, because "cannot
--- serialize" without a location is a bug report you cannot act on.
--- @param value any
--- @return boolean ok, string|nil reason
function M.check(value)
    return check(value, 0, "", {})
end

-- ─── Emitting ────────────────────────────────────────────────────────────────

--- Is this table a pure sequence, `1..n` with nothing else?
local function sequence_length(t)
    local n = 0
    for _ in ipairs(t) do n = n + 1 end
    local count = 0
    for _ in pairs(t) do count = count + 1 end
    return n, count == n
end

--- Default key order: the array part in order, then string keys sorted.
---
--- Sorted rather than `pairs` order, because `pairs` is not stable across runs
--- and a file that reorders itself between writes is a file whose diff is
--- noise. `codegen_d` sorted its exits and not its scenery items, so half its
--- output shuffled.
local function default_order(t)
    local n = sequence_length(t)
    local keys = {}
    for k in pairs(t) do
        if not (type(k) == "number" and k >= 1 and k <= n and k % 1 == 0) then
            keys[#keys + 1] = k
        end
    end
    table.sort(keys, function(a, b) return tostring(a) < tostring(b) end)
    return keys
end

local emit_value

--- One table key, as it appears before the `=`.
local function emit_key(k)
    if M.is_identifier(k) then return k end
    if type(k) == "number" then return "[" .. M.number(k) .. "]" end
    return "[" .. M.quote(k) .. "]"
end

--- A string: long bracket when it is prose with newlines, quoted otherwise.
local function emit_string(s)
    if not s:find("\n", 1, true) then return M.quote(s) end

    local open, close = M.long_bracket(s)
    if not open then return M.quote(s) end

    -- **An opening long bracket eats the newline that immediately follows it.**
    -- That is exactly what the newline written here is for: it is consumed by
    -- the parser, and the content begins at column 0 of the next line, byte for
    -- byte. So a string that itself starts with a newline needs nothing extra —
    -- it keeps its blank first line because the eaten newline is the separator
    -- rather than part of the value. (Adding a second one here "to compensate"
    -- is the obvious move and it silently prepends a blank line on every
    -- round trip.)
    --
    -- Nothing is added at the end either. The closing bracket goes flush against
    -- the last character, so a value not ending in a newline does not acquire
    -- one, and indentation is never inserted before the close — indenting it
    -- puts that indentation *inside* the string, which is how a description
    -- grows four spaces on every read-and-rewrite.
    return open .. "\n" .. s .. close
end

local function emit_table(t, indent, opts, depth, path)
    local n, pure = sequence_length(t)
    local keys = (opts.order and opts.order(t, path)) or default_order(t)

    if n == 0 and #keys == 0 then return "{}" end

    local inner = indent .. opts.indent

    -- A short table of scalars on one line.
    --
    -- `tags = { "indoor", "damp" }` reads as one fact; spread over four lines it
    -- reads as four. The same is true of a map — `damage = { min = 3, max = 7 }`
    -- is one number pair, and three lines make it look like structure.
    do
        local scalar = true
        for _, v in pairs(t) do
            if type(v) == "table" then scalar = false break end
        end
        if scalar then
            local parts = {}
            for i = 1, n do
                parts[#parts + 1] = emit_value(t[i], inner, opts, depth + 1, join(path, i))
            end
            for _, k in ipairs(keys) do
                parts[#parts + 1] = emit_key(k) .. " = "
                    .. emit_value(t[k], inner, opts, depth + 1, join(path, k))
            end
            local oneline = "{ " .. table.concat(parts, ", ") .. " }"
            -- Not when a comment belongs above one of these keys: the whole
            -- point of a section comment is the line break before it.
            local commented = false
            if opts.comment_for then
                for _, k in ipairs(keys) do
                    if opts.comment_for(t, path, k) then commented = true break end
                end
            end
            if not commented
               and #oneline + #indent <= opts.inline_width
               and not oneline:find("\n", 1, true) then
                return oneline
            end
        end
    end
    local _ = pure

    local lines = { "{" }

    for i = 1, n do
        lines[#lines + 1] = inner
            .. emit_value(t[i], inner, opts, depth + 1, join(path, i)) .. ","
    end

    -- `=` aligned within this literal. Deterministic given the value, so it
    -- costs nothing in idempotence; adding a long key reformats this one table,
    -- which is irrelevant when the file is regenerated wholesale anyway.
    local width = 0
    local rendered = {}
    for _, k in ipairs(keys) do
        local text = emit_key(k)
        rendered[#rendered + 1] = { key = k, text = text }
        if #text > width then width = #text end
    end

    for _, entry in ipairs(rendered) do
        local comments = opts.comment_for and opts.comment_for(t, path, entry.key)
        if comments then
            -- A blank line before a section, so the block reads as a block.
            if #lines > 1 then lines[#lines + 1] = "" end
            for _, c in ipairs(comments) do
                lines[#lines + 1] = inner .. "-- " .. c
            end
        end
        local pad = string.rep(" ", width - #entry.text)
        lines[#lines + 1] = inner .. entry.text .. pad .. " = "
            .. emit_value(t[entry.key], inner, opts, depth + 1, join(path, entry.key))
            .. ","
    end

    lines[#lines + 1] = indent .. "}"
    return table.concat(lines, "\n")
end

emit_value = function(value, indent, opts, depth, path)
    local t = type(value)
    if t == "nil" then return "nil" end
    if t == "boolean" then return tostring(value) end
    if t == "number" then return (M.number(value)) end
    if t == "string" then return emit_string(value) end
    return emit_table(value, indent, opts, depth, path)
end

--- Render a value as Lua source that loads back to an equal value.
---
--- @param value any
--- @param opts table|nil {
---     indent       = "    ",
---     order        = function(tbl, path) -> array of keys,
---     comments     = { [key] = { "weapon" } }
---                  | function(tbl, path, key) -> { "weapon" } | nil,
---     inline_width = 78,
--- }
---
--- `comments` takes a function as well as a table for the same reason `order`
--- does: a file holds an *array* of records, and which key deserves a section
--- comment depends on the record. A flat table would put the same comment above
--- the same key in every record, or in none.
--- @return string|nil source, string|nil err
function M.value(value, opts)
    local ok, why = M.check(value)
    if not ok then return nil, why end

    opts = opts or {}

    local comment_for
    if type(opts.comments) == "function" then
        comment_for = opts.comments
    elseif type(opts.comments) == "table" then
        local flat = opts.comments
        comment_for = function(_, _, key) return flat[key] end
    end

    return emit_value(value, "", {
        indent       = opts.indent or "    ",
        order        = opts.order,
        comment_for  = comment_for,
        inline_width = opts.inline_width or M.INLINE_WIDTH,
    }, 0, "")
end

--- A whole module file: header comments, then `return <value>`, newline
--- terminated. What codegen actually calls.
--- @param value any
--- @param opts table|nil  as `M.value`, plus `header` = array of comment lines
--- @return string|nil source, string|nil err
function M.module(value, opts)
    opts = opts or {}
    local body, err = M.value(value, opts)
    if not body then return nil, err end

    local lines = {}
    for _, line in ipairs(opts.header or {}) do
        lines[#lines + 1] = (line == "" and "--" or ("-- " .. line))
    end
    lines[#lines + 1] = "return " .. body
    lines[#lines + 1] = ""
    return table.concat(lines, "\n")
end

return M
