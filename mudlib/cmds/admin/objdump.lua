-- mudlib/cmds/objdump.lua — Everything known about one thing.
--
-- `stat` is the readable summary; this is the dump. It answers "what is
-- actually in that table", which is the question you have when something is
-- behaving as though a field you set is not there.
--
-- It knew about online players and rooms and nothing else, so `objdump rat`
-- said "Player or room not found" about a creature standing in front of you.
-- Creatures, item instances and both template registries are dumpable now, and
-- every dump ends with the raw fields — a curated view can only show what
-- somebody thought to list, and the reason you are running objdump is usually
-- that the interesting field is not one of them.

local M = {}
M.name = 'objdump'
M.aliases = {'@objdump'}
M.category = 'admin'
M.summary = 'Dump everything known about a player, room, creature or item.'
M.usage = {
    "objdump                    — the room you are standing in",
    "objdump <name>             — player, room, creature or item, in that order",
    "objdump player:<name>      — force one kind when a name is ambiguous",
    "objdump room:<area.room>",
    "objdump mob:<name|id>",
    "objdump item:<name|uuid>",
    "objdump template:<id>      — a mob or item template, unspawned",
    "",
    "  -d <n>  nest this deep (default 2, max 8)",
    "  -r      resolve lfun fields — show what they return, not <function>",
    "  -i      include fields inherited through the metatable",
    "  -s      annotate: · OLC-editable  # hand-code only  ! not in the schema",
    "  -a      all of the above",
}
M.help = [[
The dump `stat` is the summary of. Every dump ends with the raw fields, because
a curated view can only show what somebody thought to list and the reason you
are running objdump is usually that the interesting field is not one of them.

Defaults are unchanged by the flags: `objdump rat` prints exactly what it always
did, so a dump stays diffable against the last one.

{yellow}-s{/} is the one worth knowing about. It marks each field against the
schema, and `!` means **no schema names this field** — so `olc save` would drop
it. That is the only way to find out before the loss rather than after.

{yellow}-r{/} resolves lfuns, and only the fields the schema types as one: a dump
that called every stored function would be a dump with side effects.]]
M.permission = "cmd.objdump"

-- Sorted, not `pairs` order: a dump you cannot diff against the last one is
-- most of the way to useless.
local function format_dict(d)
    if not d or next(d) == nil then return "(none)" end
    local keys = {}
    for k in pairs(d) do keys[#keys + 1] = k end
    table.sort(keys, function(a, b) return tostring(a) < tostring(b) end)

    local parts = {}
    for _, k in ipairs(keys) do
        parts[#parts + 1] = tostring(k) .. " = " .. tostring(d[k])
    end
    return table.concat(parts, ", ")
end

local function format_array(arr)
    if not arr or #arr == 0 then return "(none)" end
    local parts = {}
    for i, v in ipairs(arr) do parts[i] = tostring(v) end
    return table.concat(parts, ", ")
end

--- Collapse an inventory into "id x2, other_id" in carry order.
---
--- Entries are `{ template = "id" }` tables (Mobile:add_item), with bare
--- strings still reaching this from older saves. Counting the *entry* used a
--- table as the key, so every count was 1 and the table itself went on to
--- `table.concat` — which raised for any player carrying anything at all.
--- Exposed for testing, as `_roll` and `_plan_flush` are.
--- @param inventory table|nil
--- @return string
function M._format_inventory(inventory)
    if type(inventory) ~= "table" then return "(empty)" end

    local counts, order = {}, {}
    for _, entry in ipairs(inventory) do
        local id
        if type(entry) == "string" then
            id = entry
        elseif type(entry) == "table" then
            id = entry.template
        end
        if type(id) == "string" then
            if not counts[id] then
                counts[id] = 0
                order[#order + 1] = id
            end
            counts[id] = counts[id] + 1
        end
    end

    if #order == 0 then return "(empty)" end

    local parts = {}
    for _, id in ipairs(order) do
        parts[#parts + 1] = counts[id] > 1 and (id .. " x" .. counts[id]) or id
    end
    return table.concat(parts, ", ")
end

-- ─── The generic dump ────────────────────────────────────────────────────────

--- How deep a dump nests by default, and how deep it may be asked to go.
---
--- Two now rather than one constant, because two was never enough for the thing
--- objdump is most often run on: a weapon shows at depth 1, `weapon.damage` at
--- 2, and a component's sub-sub-table as `<table>`. Raising the *default* would
--- change every existing invocation's output, and this file's stated purpose is
--- a dump you can diff against the last one — so the default stands and `-d`
--- asks for more.
local DEFAULT_DEPTH = 2
local MAX_DEPTH = 8

--- Per-dump options, set from the flags and read by `dump_fields`.
---
--- A module-level table rather than a parameter threaded through six call sites.
--- `objdump` is one dispatch on one thread and cannot re-enter itself, so the
--- alternative buys nothing and costs six signatures.
local opts = {}

local function reset_opts()
    opts = { depth = DEFAULT_DEPTH, resolve = false, inherit = false, schema = false }
end
reset_opts()

--- The schema descriptor for a field, when the caller asked for annotations.
---
--- Nil when `-s` is off, when the object's kind is unknown, or when no schema
--- names the field — and the *third* case is the interesting one: that is the
--- `!` marker, a field `olc save` would drop.
local function descriptor_for(key)
    if not (opts.schema and opts.kind) then return nil end
    local ok, schema = pcall(require, 'lib.schema')
    if not ok then return nil end
    local found = schema.field(opts.kind, tostring(key), opts.data)
    if found then return found end
    -- A hand-written field is known, just not editable.
    local mod = schema.of(opts.kind)
    for _, name in ipairs((mod and mod.hand_written) or {}) do
        if name == key then return { name = key, editable = false, hand_written = true } end
    end
    return nil
end

--- The marker column. Fixed width, so `diff` still lines up.
---
---   ·  OLC can edit this
---   #  hand-code only — an lfun, a hook, an id
---   !  NOT IN THE SCHEMA. `olc save` would drop it.
---
--- `!` is the point of the whole flag: it is the only thing in the system that
--- answers "what am I about to lose?" *before* the loss, and it is the same
--- question `verify`'s LOSSY section answers for a whole area.
local function marker(key, inherited)
    if not opts.schema then return "" end
    if inherited then return "^ " end
    local d = descriptor_for(key)
    if not d then return "! " end
    if d.editable == false then return "# " end
    return "· "
end

local function opaque(v)
    local t = type(v)
    if t == "function" then return "<function>" end
    if t == "userdata" then return "<userdata>" end
    if t == "thread"   then return "<thread>"   end
    return nil
end

--- What a function-valued field actually returns, under `-r`.
---
--- Only for fields the schema types `lfun` or `text`, and only when asked. A
--- dump that called every stored function would be a dump with side effects, and
--- an admin command that changes the world by looking at it is a trap. The
--- resolver is `Object.resolve` verbatim rather than a second copy: it already
--- `pcall`s and already answers `<invalid lfun return>` for a raiser.
local function resolved(key, value, owner)
    if not opts.resolve or type(value) ~= "function" then return nil end
    local d = descriptor_for(key)
    if not (d and (d.type == "lfun" or d.type == "text")) then return nil end

    local ok, Object = pcall(require, 'lib.object')
    if not ok then return nil end
    local rok, text = pcall(Object.resolve, value, owner)
    if not rok then return "<raised>" end
    return tostring(text)
end

local function is_array(t)
    local n = 0
    for k in pairs(t) do
        if type(k) ~= "number" then return false end
        n = n + 1
    end
    return n == #t
end

local function sorted_keys(t)
    local keys = {}
    for k in pairs(t or {}) do keys[#keys + 1] = tostring(k) end
    table.sort(keys)
    return keys
end

--- Append every field of `tbl` to `lines`, sorted, nesting expanded to `depth`.
---
--- Sorted for the same reason `format_dict` is: a dump you cannot diff against
--- the last one is most of the way to useless. Cycles are marked rather than
--- followed — an Object's metatable chain and a container's parent pointer both
--- close a loop, and a stack overflow inside an admin command takes the game
--- thread with it.
local function dump_fields(lines, tbl, indent, depth, seen, owner, inherited)
    seen = seen or {}
    if seen[tbl] then
        lines[#lines + 1] = indent .. "(cycle)"
        return
    end
    seen[tbl] = true
    owner = owner or tbl

    local keys = {}
    for k in pairs(tbl) do keys[#keys + 1] = k end
    table.sort(keys, function(a, b) return tostring(a) < tostring(b) end)

    for _, k in ipairs(keys) do
        local v = tbl[k]
        -- The marker is only meaningful at the top level of an object: a key
        -- inside `weapon` is not a field the schema names on its own.
        local label = indent .. ((depth == opts.depth) and marker(k, inherited) or "")
            .. tostring(k)
        local tag = opaque(v)
        if tag then
            local shown = resolved(k, v, owner)
            lines[#lines + 1] = label .. " = " .. tag
                .. (shown and (" -> " .. string.format("%q", shown)) or "")
        elseif type(v) ~= "table" then
            lines[#lines + 1] = label .. " = " .. tostring(v)
        elseif next(v) == nil then
            lines[#lines + 1] = label .. " = (empty)"
        elseif depth <= 0 then
            lines[#lines + 1] = label .. " = "
                .. (is_array(v) and ("<" .. #v .. " entries>") or "<table>")
        elseif is_array(v) then
            local parts = {}
            for i, item in ipairs(v) do
                parts[i] = opaque(item) or (type(item) == "table" and "<table>" or tostring(item))
            end
            lines[#lines + 1] = label .. " = [" .. table.concat(parts, ", ") .. "]"
        else
            lines[#lines + 1] = label .. ":"
            dump_fields(lines, v, indent .. "  ", depth - 1, seen, owner, inherited)
        end
    end

    seen[tbl] = nil
end

--- Fields and methods reached through the metatable chain, under `-i`.
---
--- `dump_fields` walks `pairs(tbl)`, which is the instance table only — so an
--- Item's `display_name`, `has_tag` and every `Object` default are invisible,
--- and "the field I am looking for is not in the dump" reads as "it does not
--- exist" rather than "it is inherited".
---
--- Data and methods are split, and that split is what makes the flag usable:
--- expanded identically, every room dump grows forty lines of `Room:` methods
--- and nobody turns it on twice.
local function dump_inherited(lines, tbl, indent)
    local seen_here = {}
    for k in pairs(tbl) do seen_here[k] = true end

    local mt = getmetatable(tbl)
    local level = 0
    while type(mt) == "table" and type(mt.__index) == "table" and level < MAX_DEPTH do
        local parent = mt.__index
        local data, methods = {}, {}
        for k, v in pairs(parent) do
            -- Metamethods are the *mechanism* of inheritance, not a field of
            -- the object. `__index` in particular points back at the class, so
            -- expanding it walks the chain a second time and reports a cycle —
            -- which is technically true and tells nobody anything.
            local meta = type(k) == "string" and k:sub(1, 2) == "__"
            if not seen_here[k] and not meta then
                seen_here[k] = true
                if type(v) == "function" then
                    methods[#methods + 1] = tostring(k)
                else
                    data[#data + 1] = k
                end
            end
        end

        if #data > 0 then
            table.sort(data, function(a, b) return tostring(a) < tostring(b) end)
            local slice = {}
            for _, k in ipairs(data) do slice[k] = parent[k] end
            dump_fields(lines, slice, indent, opts.depth, nil, tbl, true)
        end
        if #methods > 0 then
            table.sort(methods)
            lines[#lines + 1] = indent .. "^ methods: " .. table.concat(methods, ", ")
        end

        mt = getmetatable(parent)
        level = level + 1
    end
end

-- Exposed for testing, as `_format_inventory` is. The cycle guard is the part
-- worth a test: it is invisible until the day a dump takes the game thread
-- down with a stack overflow.
M._dump_fields = dump_fields

--- The raw view, appended to every curated one.
---
--- A curated dump can only show the fields somebody thought to list, and the
--- reason you are running objdump is usually that the field you care about is
--- not one of them.
local function dump_raw(lines, tbl, kind)
    if type(tbl) ~= "table" then return end

    -- Which schema to annotate against. Set here rather than passed, because
    -- every curated dump funnels into this one call.
    opts.kind = kind
    opts.data = tbl

    lines[#lines + 1] = "  {yellow}Raw fields:{/}"
    dump_fields(lines, tbl, "    ", opts.depth, nil, tbl, false)

    if opts.inherit then
        lines[#lines + 1] = "  {yellow}Inherited:{/}"
        dump_inherited(lines, tbl, "    ")
    end

    if opts.schema then
        lines[#lines + 1] = "  {dim}· OLC-editable   # hand-code only   "
            .. "^ inherited   ! not in the schema{/}"

        local dropped = {}
        for k in pairs(tbl) do
            if not descriptor_for(k) then dropped[#dropped + 1] = tostring(k) end
        end
        if #dropped > 0 and kind then
            table.sort(dropped)
            lines[#lines + 1] = "  {red}" .. #dropped .. " field"
                .. (#dropped == 1 and "" or "s")
                .. " no schema names, which `olc save` would drop: "
                .. table.concat(dropped, ", ") .. "{/}"
        end
    end
end

--- Present traits, read through the trait daemon rather than off `stats`.
---
--- `stats` is the *stored* table: a derived trait stores nothing at all and a
--- buffed one stores the unbuffed number, so dumping `stats` alone reports
--- neither. Base and effective are shown side by side when they differ, since
--- "why is this 12 when I set 10" is the question that brings you here.
local function dump_traits(lines, entity)
    if not (DAEMON and DAEMON.trait and DAEMON.trait.all) then return end
    local ok, traits = pcall(DAEMON.trait.all, entity)
    if not ok or type(traits) ~= "table" or #traits == 0 then return end

    lines[#lines + 1] = "  Traits:"
    for _, t in ipairs(traits) do
        local line = string.format("    %-20s %s", tostring(t.id), tostring(t.value))
        if t.max then line = line .. " / " .. tostring(t.max) end
        if t.base ~= nil and t.base ~= t.value then
            line = line .. "  (base " .. tostring(t.base) .. ")"
        end
        line = line .. string.format("  [%s/%s]", tostring(t.kind), tostring(t.category))
        if t.failed then line = line .. "  {red}FAILED: " .. tostring(t.failed) .. "{/}" end
        lines[#lines + 1] = line
    end
end

local function dump_effects(lines, entity)
    if not (DAEMON and DAEMON.effect and DAEMON.effect.active) then return end
    local ok, active = pcall(DAEMON.effect.active, entity)
    if not ok or type(active) ~= "table" or #active == 0 then return end

    lines[#lines + 1] = "  Effects:"
    for _, e in ipairs(active) do
        local inst = e.inst or {}
        lines[#lines + 1] = string.format("    %-20s source=%s  stacks=%s  expires=%s",
            tostring(inst.def), tostring(inst.source or "-"),
            tostring(inst.stacks or 1), tostring(inst.expires or "never"))
    end
end

-- ─── Resolution ──────────────────────────────────────────────────────────────

local KINDS = { player = true, room = true, mob = true, item = true, template = true }

local function find_online_player(name)
    local want = name:lower()
    local prefix_match
    for _, sid in ipairs(all_sessions()) do
        -- `get_session` raises on an id that has gone away rather than
        -- returning nil, and a session can close while we are iterating.
        local ok, s = pcall(get_session, sid)
        if ok and s and s.state == "playing" and s.character_id then
            local p = DAEMON.character and DAEMON.character.get(s.character_id)
            if p and type(p.name) == "string" then
                local n = p.name:lower()
                if n == want then return p end
                if not prefix_match and n:find(want, 1, true) == 1 then prefix_match = p end
            end
        end
    end
    return prefix_match
end

--- An item instance, wherever it is: by uuid, in your hands, or on the floor.
--- @return table|nil instance, table|nil resolved
local function find_item(player, name, room_id)
    local ID = DAEMON and DAEMON.items
    if not ID then return nil end

    if ID.get_instance then
        local inst = ID.get_instance(name)
        if inst then return inst, ID.resolve and ID.resolve(inst) or inst end
    end

    if ID.find_by_name and type(player.inventory) == "table" then
        local _, resolved, idx = ID.find_by_name(name, player.inventory)
        if resolved then return player.inventory[idx], resolved end
    end

    if room_id and ID.find_in_room then
        local inst, resolved = ID.find_in_room(room_id, name)
        if inst then return inst, resolved end
    end

    return nil
end

--- Work out what the admin meant. Returns a kind and the thing.
---
--- Order is player → room → creature → item → template, and it is the order it
--- is because that is roughly how specific the names are: a room id is unique,
--- a creature keyword is not, and a template id is the fallback that always
--- matches something.
local function resolve(player, spec, forced, room_id)
    local function want(kind) return forced == nil or forced == kind end

    if want("player") then
        local p = find_online_player(spec)
        if p then return "player", p end
    end

    if want("room") and DAEMON.world and DAEMON.world.get_room then
        local room = DAEMON.world.get_room(spec)
        if room then return "room", room end
    end

    if want("mob") and DAEMON.mobs then
        local finder = DAEMON.mobs.find_anywhere
        local ok, mob = pcall(finder or function() end, spec, room_id)
        if ok and mob then return "mob", mob end
    end

    if want("item") then
        local inst, resolved = find_item(player, spec, room_id)
        if inst then return "item", inst, resolved end
    end

    if want("template") then
        if DAEMON.mobs and DAEMON.mobs.get then
            local t = DAEMON.mobs.get(spec)
            if t then return "mob_template", t end
        end
        if DAEMON.items and DAEMON.items.get then
            local t = DAEMON.items.get(spec)
            if t then return "item_template", t end
        end
    end

    return nil
end

-- ─── Dumps ───────────────────────────────────────────────────────────────────

local function dump_mob(mob)
    local lines = {}
    table.insert(lines, string.format("─── {green}Creature{/}: %s ─────────────────────────────",
        tostring(mob.short or mob.name or mob.id)))
    table.insert(lines, string.format("  Instance: %s | Template: %s | Room: %s",
        tostring(mob.id), tostring(mob.template_id), tostring(mob.room_id)))
    table.insert(lines, string.format("  Name: %s | Level: %s | XP award: %s",
        tostring(mob.name), tostring(mob.level or "?"), tostring(mob.xp_award or 0)))

    local flags = {}
    for _, f in ipairs({ "aggressive", "stationary", "unique", "sentinel", "hidden" }) do
        if mob[f] then flags[#flags + 1] = f end
    end
    if mob.faction then flags[#flags + 1] = "faction=" .. tostring(mob.faction) end
    table.insert(lines, "  Flags: " .. (#flags > 0 and table.concat(flags, ", ") or "(none)"))

    if type(mob.patrol) == "table" then
        table.insert(lines, "  Patrol: " .. format_array(mob.patrol))
    end
    table.insert(lines, "  Dialogue: " .. (type(mob.dialogue) == "table"
        and format_array(sorted_keys(mob.dialogue)) or "(none)"))
    table.insert(lines, "  Inventory: " .. M._format_inventory(mob.inventory))

    dump_traits(lines, mob)
    dump_effects(lines, mob)
    dump_raw(lines, mob, "mob")
    return lines
end

local function dump_item(instance, resolved)
    local item = resolved or instance
    local lines = {}
    table.insert(lines, string.format("─── {green}Item{/}: %s ─────────────────────────────",
        tostring(item.short or item.id)))
    table.insert(lines, string.format("  Instance: %s | Template: %s | Location: %s",
        tostring(instance.id), tostring(instance.template or item.id),
        tostring(instance.location or "(nowhere)")))
    table.insert(lines, string.format("  Weight: %s | Value: %s | Slot: %s",
        tostring(item.weight or 0), tostring(item.value or 0), tostring(item.slot or "-")))

    if DAEMON.items and DAEMON.items.contents and instance.id then
        local ok, contents = pcall(DAEMON.items.contents, instance.id)
        if ok and type(contents) == "table" and #contents > 0 then
            local names = {}
            for i, c in ipairs(contents) do names[i] = tostring(c.id or c.template) end
            table.insert(lines, "  Contains: " .. table.concat(names, ", "))
        end
    end

    dump_traits(lines, item)
    dump_effects(lines, item)

    table.insert(lines, "  Object state:")
    local ok, state = pcall(get_all_object_state, instance.id)
    if ok and type(state) == "table" and next(state) then
        local keys = {}
        for k in pairs(state) do keys[#keys + 1] = k end
        table.sort(keys, function(a, b) return tostring(a) < tostring(b) end)
        for _, k in ipairs(keys) do
            table.insert(lines, string.format("    %s = %s", tostring(k), tostring(state[k])))
        end
    else
        table.insert(lines, "    (none)")
    end

    -- The instance, not the resolved overlay: the resolved view is the template
    -- with overrides merged in, and "what does this one actually override" is a
    -- different question from "what is it like".
    dump_raw(lines, instance, "item")
    return lines
end

local function dump_template(kind, t)
    local lines = {}
    table.insert(lines, string.format("─── {cyan}%s template{/}: %s ─────────────",
        kind, tostring(t.id)))
    table.insert(lines, "  Short: " .. tostring(t.short or "(none)"))
    table.insert(lines, "  {yellow}Not spawned — this is the shared template, not an instance.{/}")
    dump_raw(lines, t, kind == "Item" and "item" or "mob")
    return lines
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    local room_id = DAEMON.world and DAEMON.world.get_character_room
        and DAEMON.world.get_character_room(player.char_id)

    -- Flags first, then the spec. Defaults are unchanged by design: every
    -- existing invocation prints byte-identically, which is what makes a dump
    -- diffable against the last one — this file's stated purpose.
    reset_opts()
    local spec = (args_str or ""):gsub("^%s+", ""):gsub("%s+$", "")

    while true do
        local flag, rest = spec:match("^%-(%a)%s*(.*)$")
        if not flag then break end
        if flag == "d" then
            local n, tail = rest:match("^(%d+)%s*(.*)$")
            if not n then
                player:send("{red}-d needs a depth: objdump -d 4 <spec>{/}")
                return
            end
            opts.depth = math.max(1, math.min(MAX_DEPTH, tonumber(n)))
            spec = tail
        elseif flag == "r" then
            opts.resolve = true
            spec = rest
        elseif flag == "i" then
            opts.inherit = true
            spec = rest
        elseif flag == "s" then
            opts.schema = true
            spec = rest
        elseif flag == "a" then
            -- Everything, for when you do not yet know what you are looking for.
            opts.depth, opts.resolve, opts.inherit, opts.schema = 6, true, true, true
            spec = rest
        else
            player:send("{red}Unknown flag '-" .. flag .. "'. See: help objdump{/}")
            return
        end
    end
    spec = spec:gsub("^%s+", ""):gsub("%s+$", "")

    -- No argument dumps where you are standing. That is what you wanted nine
    -- times in ten, and reciting the usage line at somebody who typed the
    -- command correctly is not help.
    if spec == "" then
        if not room_id then
            local lines = { "Usage:" }
            for _, u in ipairs(M.usage) do lines[#lines + 1] = "  " .. u end
            player:send(table.concat(lines, "\r\n") .. "\r\n")
            return
        end
        spec = room_id
    end

    -- `kind:rest` forces one branch, for when a creature and a room share a
    -- word. Room ids are dotted and instance ids are uuids, so neither can be
    -- mistaken for a prefix.
    local forced = nil
    local head, rest = spec:match("^(%a+):(.+)$")
    if head and KINDS[head:lower()] then
        forced, spec = head:lower(), rest
    end

    local kind, thing, extra = resolve(player, spec, forced, room_id)

    if kind == "mob" then
        player:send(table.concat(dump_mob(thing), "\r\n") .. "\r\n")
        return
    elseif kind == "item" then
        player:send(table.concat(dump_item(thing, extra), "\r\n") .. "\r\n")
        return
    elseif kind == "mob_template" then
        player:send(table.concat(dump_template("Creature", thing), "\r\n") .. "\r\n")
        return
    elseif kind == "item_template" then
        player:send(table.concat(dump_template("Item", thing), "\r\n") .. "\r\n")
        return
    end

    if kind == "player" then
        local p = thing
        local lines = {}
        table.insert(lines, string.format("─── {green}Player{/}: %s ─────────────────────────────", p.name))
        table.insert(lines, string.format("  Char ID: %s | Account: %s | Session: %s", tostring(p.char_id), tostring(p.account_id), tostring(p.session_id)))
        
        local where = DAEMON.world and DAEMON.world.get_character_room(p.char_id) or "Unknown"
        table.insert(lines, string.format("  Room: %s", where))
        
        -- Through `:trait()` rather than `p.stats`. A derived trait stores
        -- nothing at all, so reading `stats.max_hp` reported 0 for every
        -- character; an effect-modified attribute reported the unbuffed number.
        -- `traits <name>` is where base and effective are shown side by side.
        local function tv(id) return tostring(p:trait(id)) end
        table.insert(lines, string.format("  HP: %s/%s | MP: %s/%s | Level: %s",
            tv("hp"), tv("max_hp"), tv("mp"), tv("max_mp"), tv("level")))
        table.insert(lines, string.format("  STR: %s | DEX: %s | INT: %s | CON: %s",
            tv("strength"), tv("dexterity"), tv("intelligence"), tv("constitution")))
        table.insert(lines, string.format("  XP: %s | Gold: %s", tostring(p.xp or 0), tostring(p.gold or 0)))
        table.insert(lines, string.format("  Title: %s | Race: %s | Gender: %s", p.title or "(none)", p.race or "(none)", p.gender or "(none)"))
        
        local eq_slots = {}
        if type(p.equipment) == "table" then
            for slot in pairs(p.equipment) do eq_slots[#eq_slots + 1] = slot end
            table.sort(eq_slots)
        end
        local eq_parts = {}
        for _, slot in ipairs(eq_slots) do
            eq_parts[#eq_parts + 1] = slot .. " -> " .. tostring(p.equipment[slot])
        end
        table.insert(lines, "  Equipment: " .. (#eq_parts > 0 and table.concat(eq_parts, ", ") or "(empty)"))

        table.insert(lines, "  Inventory: " .. M._format_inventory(p.inventory))
        
        table.insert(lines, "  Channels: " .. format_array(p.channels))
        table.insert(lines, "  Quest flags: " .. format_dict(p.quest_flags))
        -- Skills used to have a line of their own. They are traits now, so
        -- `traits <name>` shows them alongside everything else the character
        -- holds — one place to look rather than two that can disagree.
        table.insert(lines, "  Tags: " .. format_array(p.tags))
        table.insert(lines, "  Custom: " .. format_dict(p.custom))

        dump_traits(lines, p)
        dump_effects(lines, p)
        dump_raw(lines, p)

        player:send(table.concat(lines, "\r\n") .. "\r\n")
        return
    end

    if kind == "room" then
        local room = thing
        local lines = {}
        table.insert(lines, string.format("─── {cyan}Room{/}: %s ─────────────", room.id))
        table.insert(lines, string.format("  Short: %s", room.short or "(none)"))
        
        local area_name = room.id:match("^(.-)%.")
        local area_meta = area_name and DAEMON.world.all_area_meta and DAEMON.world.all_area_meta()[area_name]
        if area_meta then
            table.insert(lines, string.format("  Area: %s (Level %s, %s)", area_meta.name or area_name, tostring(area_meta.level or "?"), area_meta.status or "unknown"))
        else
            table.insert(lines, string.format("  Area: %s (Unknown)", area_name or "?"))
        end
        
        table.insert(lines, string.format("  Light: %s", tostring(room.light_level or 0)))
        
        local exit_parts = {}
        if room.exits then
            for dir, target in pairs(room.exits) do
                if type(target) == "table" then
                    table.insert(exit_parts, dir .. " → " .. tostring(target.target))
                else
                    table.insert(exit_parts, dir .. " → " .. tostring(target))
                end
            end
        end
        table.insert(lines, "  Exits: " .. (#exit_parts > 0 and table.concat(exit_parts, ", ") or "(none)"))
        
        local char_parts = {}
        if DAEMON.world and DAEMON.world._locations then
            for cid, rid in pairs(DAEMON.world._locations) do
                if rid == room.id then
                    local p = get_character(cid)
                    if p then
                        table.insert(char_parts, string.format("%s (char_id=%s)", p.name, tostring(cid)))
                    end
                end
            end
        end
        table.insert(lines, "  Characters: " .. (#char_parts > 0 and table.concat(char_parts, ", ") or "(none)"))
        
        local items_parts = {}
        if room.items then
            for name, _ in pairs(room.items) do
                table.insert(items_parts, name)
            end
        end
        table.insert(lines, "  Scenery items: " .. (#items_parts > 0 and table.concat(items_parts, ", ") or "(none)"))
        
        local action_parts = {}
        if room.actions then
            for action, _ in pairs(room.actions) do
                table.insert(action_parts, action)
            end
        end
        table.insert(lines, "  Actions: " .. (#action_parts > 0 and table.concat(action_parts, ", ") or "(none)"))
        
        -- Live creatures and loose items, which the room table does not hold:
        -- both live in their daemon's location index, and a room dump that
        -- omits them is why "there is nothing here" and `look` disagree.
        local mob_parts = {}
        if DAEMON.mobs and DAEMON.mobs.in_room then
            local ok, mobs = pcall(DAEMON.mobs.in_room, room.id)
            for _, mob in ipairs(ok and mobs or {}) do
                mob_parts[#mob_parts + 1] = string.format("%s (%s)",
                    tostring(mob.template_id), tostring(mob.id))
            end
        end
        table.insert(lines, "  Creatures: " .. (#mob_parts > 0 and table.concat(mob_parts, ", ") or "(none)"))

        local ground_parts = {}
        if DAEMON.items and DAEMON.items.in_room then
            local ok, items = pcall(DAEMON.items.in_room, room.id)
            for _, inst in ipairs(ok and items or {}) do
                ground_parts[#ground_parts + 1] = string.format("%s (%s)",
                    tostring(inst.template), tostring(inst.id))
            end
        end
        table.insert(lines, "  Ground items: " .. (#ground_parts > 0 and table.concat(ground_parts, ", ") or "(none)"))

        table.insert(lines, "  Object state:")
        local ok_state, state = pcall(get_all_object_state, room.id)
        if ok_state and type(state) == "table" and next(state) then
            local keys = {}
            for k in pairs(state) do keys[#keys + 1] = k end
            table.sort(keys, function(a, b) return tostring(a) < tostring(b) end)
            for _, k in ipairs(keys) do
                table.insert(lines, string.format("    %s = %s", tostring(k), tostring(state[k])))
            end
        else
            table.insert(lines, "    (none)")
        end

        dump_raw(lines, room, "room")

        player:send(table.concat(lines, "\r\n") .. "\r\n")
        return
    end

    -- Name what was searched. "Not found" on its own leaves you unable to tell
    -- a typo from a thing that is genuinely not loaded, which is the whole
    -- question an admin command exists to answer.
    player:send(string.format(
        "{red}Nothing called '%s' found.{/}\r\n" ..
        "Searched: online players, rooms, live creatures, items you can reach, " ..
        "and both template registries.\r\n" ..
        "Try a `kind:` prefix to force one — %s",
        spec, table.concat({ "player:", "room:", "mob:", "item:", "template:" }, " ")))
end

return M
