-- mudlib/lib/prototype.lua — Authoring by inheritance.
--
-- Six creatures in two areas differ by four numbers and repeat the other twelve
-- keys each. There was no way to say "this is another one of those", so every
-- area file restated a skeleton, and changing what a crawler *is* meant finding
-- every crawler. A prototype is that skeleton, named, and a template says which
-- one it is and what differs.
--
--     schema defaults  ←  prototype chain  ←  the area's data file  ←  custom.lua
--
-- Each layer is more specific and more hand-written than the last, and the merge
-- order is exactly that order. Everything here follows from that one line.
--
-- ─── Resolved at load, not at spawn ──────────────────────────────────────────
--
-- Flattening happens on the flat authoring data, in `areaload`, before
-- `patch.apply` and before anything is registered. So a registered template is
-- exactly what it has always been and `mob_d`, `item_d`, `combat_d` and every
-- `spawn` path cannot tell a prototyped one from a hand-written one — which is
-- why none of them needed changing.
--
-- The cost is that an edit takes effect on area reload rather than instantly.
-- `areaload` flushes this module's caches on every load, so that is structural
-- rather than something each reload path has to remember.
--
-- ─── Not a daemon ────────────────────────────────────────────────────────────
--
-- `lib/schema.lua` and `lib/areaload.lua` both need this and both load before
-- the daemon block. `schema/` and `components/` are the two closest precedents
-- in the tree and neither is a daemon either.
--
-- > **`lib/prototype.lua` must not `require('lib.schema')` at module top level.**
-- > `lib.schema` → `schema` (init) → `schema.mob` → `lib.prototype` → back, and
-- > `package.loaded["schema.mob"]` is not set when the cycle closes. Every
-- > require in here is inside a function, deliberately.
--
-- Exposes:
--   proto.NONE / proto.MAX_DEPTH
--   proto.chain(kind, id, from)      -> array of { id, data } root-first | nil, err
--   proto.flatten(kind, data, chain) -> table            the pure merge algebra
--   proto.resolve(kind, data)        -> table, err       one datum, not mutated
--   proto.resolve_list(kind, list)   -> list, report     mutates in place
--   proto.resolved_copy(kind, list)  -> list             originals untouched
--   proto.origin(kind, data, field)  -> origin, source
--   proto.discovery_seed(kind, data) -> table            which components apply
--   proto.thin(kind, data)           -> array of removed key names
--   proto.flush_cache()
--
-- See docs/src/lua-api/prototypes.md.

local M = {}

--- The delete sentinel: "this child has one field fewer than its prototype".
---
--- A string rather than a table identity, because codegen has to be able to
--- *emit* a struck field — `olc strike patrol` then `olc save` writes it to the
--- file, and a table identity does not survive that round trip.
---
--- `@` cannot begin a valid map key (`schema.valid_map_key` requires
--- `[%a_][%w_]*`), a tag, a trait id or a room id, so it cannot collide with
--- real content. The price, stated plainly: a `description` of literally
--- "@none" is unwritable.
M.NONE = "@none"

--- Not because eight is meaningful, but so a pathological chain produces a
--- named error rather than something the C stack has to survive.
M.MAX_DEPTH = 8

local _seed_memo = {}

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

local function log_warn(message)
    log("warn", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.warn, message) end
end

-- ─── Copying ─────────────────────────────────────────────────────────────────

--- A deep copy that passes functions through by reference.
---
--- A prototype's tables are shared by every template that inherits it, so they
--- can never be handed out live: one mob growing a tag would give it to every
--- creature in the game. Functions are the point of a prototype holding code
--- and are copied by reference, which is the only thing a function copy can mean.
local function copy_value(value, depth)
    if type(value) ~= "table" then return value end
    if (depth or 0) > 16 then return value end
    local out = {}
    for k, v in pairs(value) do out[k] = copy_value(v, (depth or 0) + 1) end
    return out
end

M._copy_value = copy_value

-- ─── The chain ───────────────────────────────────────────────────────────────

--- What a missing parent should say, which depends on why it is missing.
local function describe_missing(kind, id)
    local ok, protos = pcall(require, 'prototypes')
    if ok and protos and protos.find_kind then
        local other = protos.find_kind(id, kind)
        if other then
            return "prototype '" .. id .. "' is a " .. other .. " prototype; this is a " .. kind
        end
    end
    return "prototype '" .. id .. "' does not exist"
end

--- Walk a prototype id to its root.
---
--- Root-first, so `flatten` folds left to right and the nearest ancestor is the
--- last to speak before the datum itself.
--- @param kind string
--- @param id string
--- @param from string|nil  the template's own id, for the error message only
--- @return table|nil chain  array of { id, data }, or nil on failure
--- @return string|nil err
function M.chain(kind, id, from)
    if type(id) ~= "string" or id == "" then return {} end

    local ok, protos = pcall(require, 'prototypes')
    if not ok or type(protos) ~= "table" then
        return nil, "the prototype index is unavailable"
    end

    local out, seen, path = {}, {}, {}
    if from then path[#path + 1] = tostring(from) end

    local cur = id
    while cur ~= nil do
        if type(cur) ~= "string" or cur == "" then
            return nil, "a prototype reference must be a string"
        end
        path[#path + 1] = cur

        -- The full path, not "a cycle exists". The path is the fix.
        if seen[cur] then
            return nil, "prototype cycle: " .. table.concat(path, " -> ")
        end
        seen[cur] = true

        if #out >= M.MAX_DEPTH then
            return nil, "prototype chain deeper than " .. M.MAX_DEPTH
                .. ": " .. table.concat(path, " -> ")
        end

        local data = protos.get(kind, cur)
        if type(data) ~= "table" then return nil, describe_missing(kind, cur) end

        table.insert(out, 1, { id = cur, data = data })
        cur = data.prototype
    end

    return out
end

-- ─── Which components apply ──────────────────────────────────────────────────

--- The datum `schema.fields_for` should ask its component question about.
---
--- `fields_for("item", data)` reads `data.components`. If the prototype names
--- `weapon` and the child does not, then before resolution `olc set damage`
--- fails with "no field 'damage'", `verify` skips the weapon block entirely,
--- `orderer` misplaces it and `unknown` reports `damage` as undeclared. Four
--- things wrong, one place to fix them.
---
--- **Shallow on purpose.** The only question being answered is which components
--- apply, which depends on `components` (a `string_array`, so nearest writer
--- wins) and on the presence of an implicit component's fields. A shallow fill
--- answers both, memoises on the prototype id, and never mutates `data`.
---
--- Invariant, asserted in `tests/prototypes.rs`:
---   `discovery_seed(k, d).components == resolve(k, d).components`
--- @param kind string
--- @param data table|nil
--- @return table
function M.discovery_seed(kind, data)
    if type(data) ~= "table" then return data end
    if data.prototype == nil then return data end

    local key = tostring(kind) .. "|" .. tostring(data.prototype)
    local ancestors = _seed_memo[key]
    if ancestors == nil then
        ancestors = {}
        local chain = M.chain(kind, data.prototype)
        for _, layer in ipairs(chain or {}) do
            for k, v in pairs(layer.data) do ancestors[k] = v end
        end
        _seed_memo[key] = ancestors
    end

    local out = {}
    for k, v in pairs(ancestors) do out[k] = v end
    for k, v in pairs(data) do out[k] = v end
    -- A struck field is absent, which is the whole point of striking it.
    for k, v in pairs(out) do if v == M.NONE then out[k] = nil end end
    return out
end

-- ─── The merge ───────────────────────────────────────────────────────────────

--- Fold a chain and a datum into one flat table.
---
--- The chain is an argument rather than something this looks up, so the merge
--- algebra is testable with no registry, no world and no VM — which is where
--- every per-type rule below is actually pinned down.
---
--- Per-type rules, and they are the schema's, not a guess about shape:
---   scalars, enum, id, lfun   child replaces
---   range                     replaced **wholesale** — {min,max} is one value
---   map                       merged key-by-key; an `of_record` value
---                             (`exits.north`) is replaced whole, never deeper
---   string_array, id_array    child replaces
---   record_array              child replaces
---   no descriptor             kept, and replaced
---
--- Arrays replace rather than union because union has no removal, because order
--- is content (the union of two patrol routes is not a route), and because an
--- append silently doubles a loot entry that reads as correct in both files.
--- It also means `tags = {}` is the delete mechanism for a list, with no
--- sentinel and no magic.
--- @param kind string
--- @param data table                 the child's own authoring data
--- @param chain table|nil            array of { id, data }, root-first
--- @return table  a new table; neither `data` nor any chain entry is mutated
function M.flatten(kind, data, chain)
    local patch = require('lib.patch')

    chain = chain or {}
    if #chain == 0 then return copy_value(data) end

    -- One seed for every step. `prototype` is stripped from it so
    -- `schema.fields_for` does not walk the chain again for each layer: the
    -- seed already carries the ancestors' `components`.
    local seed = {}
    for _, layer in ipairs(chain) do
        for k, v in pairs(layer.data) do seed[k] = v end
    end
    for k, v in pairs(data) do seed[k] = v end
    for k, v in pairs(seed) do if v == M.NONE then seed[k] = nil end end
    seed.prototype = nil

    local opts = { seed = seed, none = M.NONE }

    local acc = {}
    for _, layer in ipairs(chain) do
        patch.merge_one(kind, acc, copy_value(layer.data), opts)
    end
    patch.merge_one(kind, acc, copy_value(data), opts)

    -- The child's own reference, kept: it is a declared field, codegen emits it,
    -- and `objdump` prints it so a flattened template does not look like it came
    -- from nowhere. A chain layer's own `prototype` was overwritten above.
    acc.prototype = data.prototype

    return acc
end

--- One datum, resolved. Nothing is mutated.
--- @param kind string
--- @param data table
--- @return table merged, string|nil err
function M.resolve(kind, data)
    if type(data) ~= "table" then return data end
    if data.prototype == nil then return data end

    local chain, err = M.chain(kind, data.prototype, data.id)
    if not chain then return copy_value(data), err end
    return M.flatten(kind, data, chain)
end

--- Resolve every datum in an area's list, in place.
---
--- In place, so callers holding a reference to a record — and `areaload` does —
--- see the resolved form. Matches `patch.apply`'s contract.
---
--- A failure logs, leaves that record **with its own data unresolved**, and
--- carries on. Never raises, never drops: a broken prototype should cost you one
--- creature's stat block, not the area.
--- @param kind string
--- @param list table  array of flat authoring data
--- @return table list, table report  { resolved = n, failed = { { id, why } } }
function M.resolve_list(kind, list)
    local report = { resolved = 0, failed = {} }
    if type(list) ~= "table" then return list, report end

    for _, data in ipairs(list) do
        if type(data) == "table" and data.prototype ~= nil then
            local merged, err = M.resolve(kind, data)
            if err then
                report.failed[#report.failed + 1] = { id = tostring(data.id), why = err }
                log_error("PROTOTYPE: " .. kind .. " '" .. tostring(data.id)
                    .. "': " .. err .. " -- loaded without it")
            else
                for k in pairs(data) do data[k] = nil end
                for k, v in pairs(merged) do data[k] = v end
                report.resolved = report.resolved + 1
            end
        end
    end
    return list, report
end

--- The same, as copies. For `verify`, which must be able to ask both "what does
--- this file hold" and "what will the next reload do" of the same area.
--- @param kind string
--- @param list table|nil
--- @return table  a new array of new tables
function M.resolved_copy(kind, list)
    local out = {}
    for i, data in ipairs(list or {}) do
        if type(data) ~= "table" then
            out[i] = data
        elseif data.prototype == nil then
            out[i] = copy_value(data)
        else
            out[i] = (M.resolve(kind, data))
        end
    end
    return out
end

-- ─── Where a value came from ─────────────────────────────────────────────────

--- Where a field's effective value came from.
---
--- A peer of `schema.is_default`, not an extension of it: `is_default` is a
--- question about a *descriptor* and this is a question about a *chain*. Putting
--- it in `lib/schema.lua` would make that module require the prototype registry
--- at top level, which is the cycle this file's header warns about.
--- @param kind string
--- @param data table
--- @param field string
--- @return string origin  "self" | "struck" | "inherited" | "default" | "unset"
--- @return string|nil source  the prototype id, when it is not the datum's own
function M.origin(kind, data, field)
    if type(data) ~= "table" or type(field) ~= "string" then return "unset" end

    if data[field] == M.NONE then return "struck" end
    if data[field] ~= nil then return "self" end

    local chain = M.chain(kind, data.prototype, data.id)
    if chain then
        -- Nearest ancestor first: the one that would actually have supplied it.
        for i = #chain, 1, -1 do
            local v = chain[i].data[field]
            if v == M.NONE then return "struck", chain[i].id end
            if v ~= nil then return "inherited", chain[i].id end
        end
    end

    local schema = require('lib.schema')
    local descriptor = schema.field(kind, field, M.discovery_seed(kind, data))
    if descriptor and descriptor.default ~= nil then return "default" end
    return "unset"
end

-- ─── Thinning ────────────────────────────────────────────────────────────────

local function deep_equal(a, b)
    if a == b then return true end
    if type(a) ~= "table" or type(b) ~= "table" then return false end
    for k, v in pairs(a) do if not deep_equal(v, b[k]) then return false end end
    for k in pairs(b) do if a[k] == nil then return false end end
    return true
end

M._deep_equal = deep_equal

--- Drop every key that only restates what the prototype already says.
---
--- The *safe* form of subtraction: a human asked for it, in a session, and sees
--- what went. Codegen must never do this on its own — a builder who deliberately
--- sets a value equal to the inherited one is saying "this is mine now, and it
--- must not move if the prototype moves", and subtracting deletes that intent
--- with nothing in the diff to show for it.
--- @param kind string
--- @param data table  mutated
--- @return table  array of the key names removed, sorted
function M.thin(kind, data)
    local removed = {}
    if type(data) ~= "table" or data.prototype == nil then return removed end

    local chain = M.chain(kind, data.prototype, data.id)
    if not chain then return removed end

    -- What the chain alone would produce, with nothing of the child's in it.
    local inherited = M.flatten(kind, { prototype = data.prototype }, chain)

    for key, value in pairs(data) do
        if key ~= "id" and key ~= "prototype" and value ~= M.NONE
            and inherited[key] ~= nil and deep_equal(value, inherited[key]) then
            removed[#removed + 1] = tostring(key)
        end
    end
    table.sort(removed)
    for _, key in ipairs(removed) do data[key] = nil end
    return removed
end

-- ─── Cache ───────────────────────────────────────────────────────────────────

--- Called from `prototypes.flush_cache`, which `areaload` calls on every load.
function M.flush_cache()
    _seed_memo = {}
end

M._log_warn = log_warn

return M
