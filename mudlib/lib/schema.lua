-- mudlib/lib/schema.lua — What an authorable thing is.
--
-- Nothing ever wrote this down, and every OLC defect traced back to that:
-- `codegen_d.generate_room` hardcoded five fields and re-emitted from that list,
-- so `light`, `smell`, `sound`, `tags` and `actions` were destroyed on the next
-- `dig`. `olc set` could not exist because there was nothing to enumerate. And
-- `adopt` could not report what it would lose because nothing knew what "lose"
-- meant.
--
-- So this is not one deliverable among several — it is the thing the others are
-- expressed in terms of:
--
--   codegen     emits fields in schema order, and refuses what the schema says
--               cannot be written
--   olc set     coerces and validates through `M.set`, which is the ONLY
--               string-to-value converter in the system
--   verify      reports `M.validate`, `M.lossy` and `M.unknown`
--   objdump -s  marks each field editable / hand-code / not-in-the-schema
--
-- One converter matters more than it looks. If OLC parses "3" one way and the
-- file loader another, the disagreement does not surface as an error — it
-- surfaces as a field that round-trips wrong, six months later, in one area.
--
-- ─── Descriptors ─────────────────────────────────────────────────────────────
--
--   { name, type, default, editable, required,
--     min, max, max_len, values, of, target, key_values, key_source, record,
--     validate = function(v, data) -> ok, err,
--     help }
--
-- `type` is one of:
--
--   string        one line
--   text          prose; prefers a long bracket when it holds a newline
--   number        any finite number
--   integer       a whole one
--   boolean       exactly true or false, never Lua truthiness
--   enum          a member of `values`
--   id            a reference to something with an id; `target` says what kind
--   string_array  an array of strings
--   id_array      an array of ids
--   map           key -> value; `of` is the value type
--   range         { min, max }
--   record_array  an array of tables; `record` is their field list
--   lfun          a string OR a function — the one type OLC may not set
--
-- The array is ordered, and that order *is* the emit order. Deterministic key
-- ordering falls out of it for free.

local M = {}

local schemas    = require('schema')
local components = require('components')
local serialize  = require('lib.serialize')

--- Words the OLC grammar reserves. A field named `on` would make
--- `olc set on <target> <path> <value>` ambiguous, and the alternative —
--- deciding by whether the next word happens to resolve as a field — is DWIM on
--- a command that writes files.
M.RESERVED = { on = true, all = true, from = true, force = true }

-- ─── Kinds ───────────────────────────────────────────────────────────────────

--- Every authorable kind, in declared order.
--- @return table  array of kind names
function M.kinds()
    local out = {}
    for _, mod in ipairs(schemas.all()) do out[#out + 1] = mod.kind end
    return out
end

--- One kind's schema module, or nil.
--- @param kind string
--- @return table|nil
function M.of(kind)
    return schemas.get(kind)
end

--- Which component blocks apply to this datum, in component order.
local function component_blocks(kind, data)
    local mod = schemas.get(kind)
    if not mod or not mod.components then return {} end

    -- Which components apply is a question about the *resolved* datum. A
    -- prototype that names `weapon` makes `damage` a real field on every child,
    -- and a child that cannot see its own fields cannot be set, linted, ordered
    -- or serialized — four wrong answers from one place. `discovery_seed`
    -- returns `data` unchanged when there is no prototype, which is every
    -- existing record, so this costs a nil check in the common case.
    --
    -- Required inside the function: `lib.prototype` is reached from `schema.mob`
    -- through this module, and a top-level require closes that loop.
    local seed = data
    if type(data) == "table" and data.prototype ~= nil then
        local ok, proto = pcall(require, 'lib.prototype')
        if ok and proto and proto.discovery_seed then
            seed = proto.discovery_seed(kind, data)
        end
    end

    local defs = components.schemas()
    local out = {}
    for _, c in ipairs(components.claimed(seed or {})) do
        local def = defs[c.component]
        if def then
            out[#out + 1] = { component = c.component, fields = def.fields }
        end
    end
    return out
end

--- Every descriptor that applies to this datum, in emit order.
---
--- The kind's own fields first, then one block per component the data claims.
--- Component descriptors carry `component`, so a caller can group them — that is
--- what puts a `-- weapon` comment above the weapon block in a generated file.
--- @param kind string
--- @param data table|nil
--- @return table  array of descriptors
function M.fields_for(kind, data)
    local mod = schemas.get(kind)
    if not mod then return {} end

    local out = {}
    for _, f in ipairs(mod.fields) do out[#out + 1] = f end
    for _, block in ipairs(component_blocks(kind, data)) do
        for _, f in ipairs(block.fields) do
            local copy = {}
            for k, v in pairs(f) do copy[k] = v end
            copy.component = block.component
            out[#out + 1] = copy
        end
    end
    return out
end

--- One descriptor by dotted path, and the table it lives in.
---
--- `short`, `weapon.speed`, `exits.north`, `resist.fire`. A path into a `map`
--- resolves to a synthetic descriptor for its *value* type, so `olc set
--- items.well "A stone well."` validates the value rather than the map.
--- @param kind string
--- @param path string
--- @param data table|nil
--- @return table|nil descriptor, string|nil container  the map field's name
function M.field(kind, path, data)
    if type(path) ~= "string" then return nil end

    local all = M.fields_for(kind, data)
    for _, f in ipairs(all) do
        if f.name == path then return f end
    end

    local head, key = path:match("^([%w_]+)%.(.+)$")
    if not head then return nil end

    for _, f in ipairs(all) do
        if f.name == head then
            if f.type == "map" then
                return {
                    name       = path,
                    type       = f.of or "string",
                    target     = f.target,
                    values     = f.values,
                    key_source = f.key_source,
                    editable   = f.editable,
                    component  = f.component,
                    help       = f.help,
                }, head
            end
            -- A component's own name is not a path prefix: its fields are flat.
            return nil
        end
    end
    return nil
end

--- One descriptor, searched across every component regardless of which a datum
--- claims.
---
--- For callers that have a field name and no record to go with it — the orderer,
--- which is handed a nested table and never sees its parent.
--- @param name string
--- @return table|nil
function M.component_field(name)
    for _, def in pairs(components.schemas()) do
        for _, f in ipairs(def.fields) do
            if f.name == name then return f end
        end
    end
    return nil
end

--- The defaults a fresh datum of this kind starts from.
--- @param kind string
--- @param base string|nil  a component to include, for `olc new item x from weapon`
--- @return table
function M.defaults(kind, base)
    local seed = {}
    if base and base ~= kind then seed.components = { base } end

    local out = {}
    for _, f in ipairs(M.fields_for(kind, seed)) do
        if f.default ~= nil then
            if type(f.default) == "table" then
                local copy = {}
                for k, v in pairs(f.default) do copy[k] = v end
                out[f.name] = copy
            else
                out[f.name] = f.default
            end
        end
    end
    if seed.components then out.components = { base } end
    return out
end

--- Is this value the descriptor's default? For deciding what to leave out.
--- @param descriptor table
--- @param value any
--- @return boolean
function M.is_default(descriptor, value)
    local d = descriptor.default
    if d == nil then return value == nil end
    if type(d) ~= "table" or type(value) ~= "table" then return d == value end
    for k, v in pairs(d) do if value[k] ~= v then return false end end
    for k in pairs(value) do if d[k] == nil then return false end end
    return true
end

-- ─── Coercion ────────────────────────────────────────────────────────────────

--- Exactly these spellings, and nothing else, is a boolean.
---
--- Not Lua truthiness. `olc set aggressive maybe` under truthiness sets it
--- *true*, which is the opposite of what somebody typing "maybe" meant, and
--- there would be no error to notice.
local BOOLEANS = {
    ["true"] = true, ["yes"] = true, ["on"] = true, ["1"] = true,
    ["false"] = false, ["no"] = false, ["off"] = false, ["0"] = false,
}

local function allowed_values(descriptor)
    local v = descriptor.values
    if type(v) == "function" then
        local ok, list = pcall(v)
        return (ok and type(list) == "table") and list or {}
    end
    return type(v) == "table" and v or {}
end

--- Turn a builder's typed text into a value of the descriptor's type.
---
--- An empty string clears the field — `olc set smell` with nothing after it is
--- how you take a smell off a room, and there needs to be some way.
--- @param descriptor table
--- @param text string
--- @return any value, string|nil err
function M.coerce(descriptor, text)
    local t = descriptor.type or "string"
    text = tostring(text or "")

    if text == "" then return nil end

    if t == "string" or t == "text" or t == "id" then
        if text:find("%z") then return nil, "a NUL byte cannot be stored" end
        if descriptor.max_len and #text > descriptor.max_len then
            return nil, "longer than " .. descriptor.max_len .. " characters"
        end
        return text
    end

    if t == "lfun" then
        return nil, "'" .. descriptor.name .. "' is an lfun field — a string OR a "
            .. "function — so OLC does not own it. Put it in the area's custom.lua."
    end

    if t == "number" or t == "integer" then
        local n = tonumber(text)
        if not n then return nil, "'" .. text .. "' is not a number" end
        if n ~= n or n == math.huge or n == -math.huge then
            return nil, tostring(n) .. " cannot be stored"
        end
        if t == "integer" and n % 1 ~= 0 then
            return nil, "'" .. text .. "' is not a whole number"
        end
        if descriptor.min and n < descriptor.min then
            return nil, "below the minimum of " .. descriptor.min
        end
        if descriptor.max and n > descriptor.max then
            return nil, "above the maximum of " .. descriptor.max
        end
        return n
    end

    if t == "boolean" then
        local b = BOOLEANS[text:lower()]
        if b == nil then
            return nil, "'" .. text .. "' is not a boolean. It accepts: "
                .. "true false yes no on off 1 0"
        end
        return b
    end

    if t == "enum" then
        local values = allowed_values(descriptor)
        for _, v in ipairs(values) do
            if v == text then return v end
        end
        return nil, "'" .. text .. "' is not one of: " .. table.concat(values, " ")
    end

    if t == "range" then
        local lo, hi = text:match("^(%-?%d+%.?%d*)%s*%-%s*(%-?%d+%.?%d*)$")
        if not lo then
            local single = tonumber(text)
            if single then return { min = single, max = single } end
            return nil, "'" .. text .. "' is not a range. Try 3-7, or a single number."
        end
        lo, hi = tonumber(lo), tonumber(hi)
        if hi < lo then return nil, "the maximum is below the minimum" end
        return { min = lo, max = hi }
    end

    if t == "string_array" or t == "id_array" then
        -- One value per `add`/`remove`. Splitting on a comma here is how a
        -- description containing a comma becomes two tags.
        return { text }
    end

    if t == "map" or t == "record_array" then
        return nil, "'" .. descriptor.name .. "' holds several values. "
            .. "Set one at a time: " .. descriptor.name .. ".<key> <value>"
    end

    return nil, "no rule for a '" .. t .. "' field"
end

--- Render a value the way `olc show` and `objdump` should print it.
--- @param descriptor table
--- @param value any
--- @return string
function M.render(descriptor, value)
    if value == nil then return "(unset)" end
    local t = descriptor.type or "string"

    if t == "range" and type(value) == "table" then
        return tostring(value.min) .. "-" .. tostring(value.max)
    end
    if (t == "string_array" or t == "id_array") and type(value) == "table" then
        if #value == 0 then return "(none)" end
        return table.concat(value, ", ")
    end
    if t == "map" and type(value) == "table" then
        local keys = {}
        for k in pairs(value) do keys[#keys + 1] = tostring(k) end
        if #keys == 0 then return "(none)" end
        table.sort(keys)
        return #keys .. " entr" .. (#keys == 1 and "y" or "ies")
            .. ": " .. table.concat(keys, ", ")
    end
    if type(value) == "function" then return "<function>" end
    if type(value) == "table" then return "<table>" end
    return tostring(value)
end

-- ─── Validation ──────────────────────────────────────────────────────────────

--- A map key has to be writable as a bare table key.
---
--- Codegen *could* emit `["my key"]`, and the reason it does not is upstream of
--- formatting: the map types are room scenery (`examine <keyword>`) and mob
--- dialogue (`talk <topic>`), which are keyword namespaces where a space is a
--- bug rather than a style.
--- @param key string
--- @return boolean ok, string|nil err
function M.valid_map_key(key)
    if type(key) ~= "string" or not key:match("^[%a_][%w_]*$") then
        return false, "map key '" .. tostring(key) .. "' is not a keyword. "
            .. "Keys must match [a-zA-Z_][a-zA-Z0-9_]*"
    end
    return true
end

--- The prototype delete sentinel, as a literal.
---
--- Spelled out rather than required, for the same cycle reason `component_blocks`
--- requires lazily. `tests/schema.rs` asserts it still matches `proto.NONE`.
local PROTOTYPE_NONE = "@none"

local function validate_one(descriptor, value, data)
    if value == nil then
        if descriptor.required then return false, "is required" end
        return true
    end

    -- A struck field: "this child has one field fewer than its prototype". It is
    -- legal wherever a value is, and it is gone by the time anything is
    -- registered — the resolver consumes it. Without this clause
    -- `tags = "@none"` lints as "is not a list", which is true and useless.
    if value == PROTOTYPE_NONE then return true end

    -- `lfun = true` is the property `Object.resolve` implements: this field is a
    -- string **or** a function returning one. It is a flag rather than a type
    -- because it is orthogonal to what the field *means* — a room's description
    -- is prose whether it is written out or computed, and OLC can author the
    -- first while the second is perfectly legal content.
    --
    -- Not the same as `type = "lfun"`, which is for a field OLC may never set at
    -- all (a weapon's hit message). Here a function value is merely `lossy`: it
    -- moves to `custom.lua` on adoption and is otherwise left alone.
    if descriptor.lfun and type(value) == "function" then return true end

    local t = descriptor.type or "string"

    if t == "boolean" then
        if type(value) ~= "boolean" then return false, "is not a boolean" end
    elseif t == "number" or t == "integer" then
        if type(value) ~= "number" then return false, "is not a number" end
        if t == "integer" and value % 1 ~= 0 then return false, "is not a whole number" end
        if descriptor.min and value < descriptor.min then
            return false, "is below the minimum of " .. descriptor.min
        end
        if descriptor.max and value > descriptor.max then
            return false, "is above the maximum of " .. descriptor.max
        end
    elseif t == "enum" then
        local values = allowed_values(descriptor)
        local hit = false
        for _, v in ipairs(values) do if v == value then hit = true break end end
        if not hit then
            return false, "is not one of: " .. table.concat(values, " ")
        end
    elseif t == "range" then
        if type(value) ~= "table" or type(value.min) ~= "number" or type(value.max) ~= "number" then
            return false, "is not a range"
        end
        if value.max < value.min then return false, "has its maximum below its minimum" end
    elseif t == "string_array" or t == "id_array" then
        if type(value) ~= "table" then return false, "is not a list" end
        for i, v in ipairs(value) do
            if type(v) == "function" and descriptor.lfun then
                -- an lfun element, which is legal content
            elseif type(v) ~= "string" then
                return false, "entry " .. i .. " is not a string"
            end
        end
    elseif t == "map" then
        if type(value) ~= "table" then return false, "is not a table" end
        for k, v in pairs(value) do
            local ok, err = M.valid_map_key(k)
            if not ok then return false, err end

            if type(v) == "table" then
                -- Some maps take a *record* as the value as well as a scalar.
                -- A room's exit is the case: `north = "crypt.hall"` and
                -- `north = { target = "crypt.hall", check = fn }` are both legal
                -- and `room_d.from_data` unwraps either. `of_record` is what
                -- says so; without it a table is a mistake.
                if not descriptor.of_record then
                    return false, "'" .. tostring(k) .. "' holds a table, and this "
                        .. "field takes one value per key"
                end
                for _, f in ipairs(descriptor.of_record) do
                    local fok, ferr = validate_one(f, v[f.name], v)
                    if not fok then
                        return false, "'" .. tostring(k) .. "'." .. f.name .. " " .. ferr
                    end
                end
            elseif type(v) == "function" and not descriptor.lfun then
                return false, "'" .. tostring(k) .. "' is a function, and this field is not an lfun"
            end
        end
    elseif t == "record_array" then
        if type(value) ~= "table" then return false, "is not a list" end
        -- Each entry is a table of the declared record fields, or — when the
        -- descriptor names a `bare` field — a plain value standing for it.
        -- `echoes` takes both: a string is an echo of weight one, and the table
        -- form is the same echo with a weight on it.
        for i, entry in ipairs(value) do
            if type(entry) == "table" then
                for _, f in ipairs(descriptor.record or {}) do
                    local ok, err = validate_one(f, entry[f.name], entry)
                    if not ok then
                        return false, "entry " .. i .. "'s " .. f.name .. " " .. err
                    end
                end
            elseif descriptor.bare then
                local field
                for _, f in ipairs(descriptor.record or {}) do
                    if f.name == descriptor.bare then field = f end
                end
                local ok, err = validate_one(field or {}, entry, value)
                if not ok then return false, "entry " .. i .. " " .. err end
            else
                return false, "entry " .. i .. " is not a table"
            end
        end
    elseif t == "lfun" then
        if type(value) ~= "string" and type(value) ~= "function" then
            return false, "is neither a string nor a function"
        end
    else
        if type(value) ~= "string" then return false, "is not a string" end
        if descriptor.max_len and #value > descriptor.max_len then
            return false, "is longer than " .. descriptor.max_len .. " characters"
        end
    end

    if type(descriptor.validate) == "function" then
        local ok, err = descriptor.validate(value, data)
        if not ok then return false, tostring(err or "is not valid") end
    end
    return true
end

--- Every problem with this datum. An empty list means it is writable.
--- @param kind string
--- @param data table
--- @return boolean ok, table errors  { { path, message } }
function M.validate(kind, data)
    local errors = {}
    if type(data) ~= "table" then
        return false, { { path = "", message = "is not a table" } }
    end

    for _, f in ipairs(M.fields_for(kind, data)) do
        local ok, err = validate_one(f, data[f.name], data)
        if not ok then
            errors[#errors + 1] = { path = f.name, message = err }
        end
    end

    table.sort(errors, function(a, b) return a.path < b.path end)
    return #errors == 0, errors
end

--- Why a field cannot be set, in a form somebody can act on.
---
--- "Not editable" is true and useless. There are only two reasons a field is
--- closed, they have different answers, and a builder who is told neither goes
--- and edits the generated file by hand — which is the one thing this whole
--- design is trying to make unnecessary.
--- @param descriptor table
--- @return string
function M.why_not_editable(descriptor)
    if descriptor.type == "lfun" then
        return "'" .. descriptor.name .. "' is an lfun field — a string OR a function — "
            .. "so OLC does not own it. Messages that read the fight have to be code. "
            .. "Put it in the area's custom.lua, which is applied after the generated "
            .. "files and is never regenerated."
    end
    if descriptor.name == "id" then
        return "'id' cannot be changed. Every exit, loot table and spawn_room that "
            .. "names it would still name the old one."
    end
    return "'" .. descriptor.name .. "' is not editable"
end

--- Write one field, by path, with coercion and validation.
---
--- The single entry point `olc set` uses, which is what stops `set` and `verify`
--- disagreeing about what a valid value is.
--- @param kind string
--- @param data table
--- @param path string
--- @param text string
--- @return boolean ok, string|nil err
function M.set(kind, data, path, text)
    local descriptor, container = M.field(kind, path, data)
    if not descriptor then
        return false, "no field '" .. tostring(path) .. "' on a " .. kind
    end
    if descriptor.editable == false then
        return false, M.why_not_editable(descriptor)
    end

    local value, err = M.coerce(descriptor, text)
    if err then return false, err end

    if container then
        local key = path:match("%.([^%.]+)$")
        local ok, why = M.valid_map_key(key)
        if not ok then return false, why end
        data[container] = data[container] or {}
        data[container][key] = value
        return true
    end

    if value ~= nil then
        local ok, why = validate_one(descriptor, value, data)
        if not ok then return false, path .. " " .. why end
    end
    data[descriptor.name] = value
    return true
end

-- ─── What will not survive a write ───────────────────────────────────────────

--- Everything in this datum that a generated file cannot express.
---
--- Two sources: a value that is a function, and a field the kind or one of its
--- components lists as `hand_written`. Both go to `custom.lua`; naming them here
--- is what lets `adopt` say "moves to custom.lua" rather than "unknown field".
--- @param kind string
--- @param data table
--- @return table  { { path, why } }   why = "function" | "hand-written"
function M.lossy(kind, data)
    local out = {}
    if type(data) ~= "table" then return out end

    local mod = schemas.get(kind)
    local named = {}
    for _, name in ipairs((mod and mod.hand_written) or {}) do named[name] = true end
    for _, block in ipairs(component_blocks(kind, data)) do
        local defs = components.schemas()[block.component]
        for _, name in ipairs((defs and defs.hand_written) or {}) do named[name] = true end
    end

    for key, value in pairs(data) do
        if named[key] then
            out[#out + 1] = { path = tostring(key), why = "hand-written" }
        elseif not serialize.check(value) then
            out[#out + 1] = { path = tostring(key), why = type(value) == "table"
                and "unserializable" or type(value) }
        end
    end

    table.sort(out, function(a, b) return a.path < b.path end)
    return out
end

--- Fields present that no schema names, and that *can* be written.
---
--- Kept verbatim on regeneration and reported — never dropped. Silently losing
--- a field nobody had got round to declaring is the exact bug this whole design
--- exists to end, and it is indistinguishable from a typo unless it is reported.
--- @param kind string
--- @param data table
--- @return table  array of paths
function M.unknown(kind, data)
    local out = {}
    if type(data) ~= "table" then return out end

    local known = { components = true }
    for _, f in ipairs(M.fields_for(kind, data)) do known[f.name] = true end

    local mod = schemas.get(kind)
    for _, name in ipairs((mod and mod.hand_written) or {}) do known[name] = true end
    for _, block in ipairs(component_blocks(kind, data)) do
        local defs = components.schemas()[block.component]
        for _, name in ipairs((defs and defs.hand_written) or {}) do known[name] = true end
    end

    for key, value in pairs(data) do
        if not known[key] and serialize.check(value) then
            out[#out + 1] = tostring(key)
        end
    end
    table.sort(out)
    return out
end

-- ─── Emit order ──────────────────────────────────────────────────────────────

--- The key order codegen passes straight to `serialize` as `opts.order`.
---
--- Schema fields in schema order, then anything left over, sorted. Nested tables
--- use their descriptor's `key_values` when it has one — so a room's exits come
--- out north-south-east-west rather than alphabetically — and sorted otherwise.
--- @param kind string
--- @return function  f(tbl, path) -> array of keys
function M.orderer(kind)
    return function(tbl, path)
        local present = {}
        for k in pairs(tbl) do
            if type(k) ~= "number" then present[k] = true end
        end

        local out = {}
        local function take(k)
            if present[k] then
                out[#out + 1] = k
                present[k] = nil
            end
        end

        -- A generated file holds an *array* of records, so a record's own
        -- breadcrumb is `1`, not `""` — `serialize`'s `join` spells a top-level
        -- index bare, the way `jsonsafe` does. Both are "the top level of a
        -- datum", and missing that is how every generated file came out in
        -- alphabetical order with the schema silently doing nothing.
        local at_top = path == "" or path:match("^%d+$") ~= nil

        if at_top then
            take("id")
            take("components")
            for _, f in ipairs(M.fields_for(kind, tbl)) do take(f.name) end
        else
            -- Inside a field. Strip the record index, so `1.exits` looks up the
            -- same descriptor `exits` does.
            local field_path = path:gsub("^%d+%.", "")
            -- Across every component, not only the ones this record claims: the
            -- orderer is handed the *nested* table and never sees its parent, so
            -- it cannot know which components are in play. Ordering does not
            -- depend on that anyway — `damage` is min-then-max wherever it is.
            local descriptor = M.field(kind, field_path)
                or M.component_field(field_path)
            if descriptor then
                for _, k in ipairs(descriptor.key_values or {}) do take(k) end
                -- A range is always `min` then `max`. Written here rather than
                -- as `key_values` on every range descriptor, because it is a
                -- property of the type and not of any one field.
                if descriptor.type == "range" then
                    take("min")
                    take("max")
                end
            end
        end

        local rest = {}
        for k in pairs(present) do rest[#rest + 1] = k end
        table.sort(rest, function(a, b) return tostring(a) < tostring(b) end)
        for _, k in ipairs(rest) do out[#out + 1] = k end
        return out
    end
end

--- The comment lines codegen puts above each component block.
--- @param kind string
--- @param data table
--- @return table  { [field_name] = { "weapon" } }
function M.comments(kind, data)
    local out = {}
    local seen = {}
    for _, f in ipairs(M.fields_for(kind, data)) do
        if f.component and not seen[f.component] and data[f.name] ~= nil then
            seen[f.component] = true
            out[f.name] = { f.component }
        end
    end
    return out
end

--- One line of help for a field.
--- @param kind string
--- @param path string
--- @return string|nil
function M.help(kind, path)
    local descriptor = M.field(kind, path)
    return descriptor and descriptor.help
end

return M
