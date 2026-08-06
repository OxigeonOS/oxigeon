-- mudlib/cmds/building/olc.lua — Online creation.
--
-- One verb, no sub-shell. Every subcommand is `olc <sub>` and nothing is
-- swallowed, so `look`, `who` and a tell all still work while you build — see
-- `daemons/olc_d.lua` for why a modal shell would be a trap.
--
-- The work is in `lib/olc.lua`; this is the grammar. Structured like
-- `cmds/admin/role.lua`: a usage array, one `verb` switch, and a fall-through
-- that prints the usage rather than a bare "unknown option".

local olc    = require('lib.olc')
local schema = require('lib.schema')
local proto  = require('lib.prototype')

local M = {}

M.name       = "olc"
M.aliases    = {}
M.category   = "building"
M.summary    = "Build rooms, items and creatures without leaving the game."
M.permission = "cmd.olc"

M.usage = {
    "{cyan}Session{/}",
    "  olc <area>                enter an area (refuses one OLC does not manage)",
    "  olc new area <name> [title...]",
    "  olc adopt <area>          report what adopting an existing area would change",
    "  olc done                  leave",
    "{cyan}Cursor{/}",
    "  olc edit <target>         what `set` acts on. `olc edit` = the room you are in",
    "  olc where                 cursor, versus where you are standing",
    "{cyan}Create{/}",
    "  olc new room|item|mob <id> [from <base>]",
    "  olc new mob <id> from proto:<prototype>    inherit; the record is 2 keys",
    "  olc bases [item|mob|room]",
    "{cyan}Inspect{/}",
    "  olc show [<target>]       current values (`olc show proto:<id>` for a prototype)",
    "  olc protos [kind]         prototypes, with parent and how many use them",
    "  olc fields [<kind>]       what could be set, with types",
    "  olc help <field>          what one field is for",
    "  olc list rooms|items|mobs",
    "  olc diff                  unsaved changes",
    "{cyan}Change{/}",
    "  olc set <field> <value>   ... or `olc set <field>` to open the editor",
    "  olc set on <target> <field> <value>",
    "  olc unset <field>         clear it here — an inherited value comes back",
    "  olc strike <field>        remove an inherited field entirely",
    "  olc thin                  drop what only restates the prototype",
    "  olc add|remove <field> <value>      for lists",
    "  olc tag|untag <tag>...",
    "  olc comp add|remove|list <component>",
    "{cyan}Persist{/}",
    "  olc save                  verify, then write",
    "  olc revert [<target>]",
}

-- ─── Helpers ─────────────────────────────────────────────────────────────────

local function fail(player, message)
    player:send("{red}" .. message .. "{/}")
end

local function ok(player, message)
    player:send("{green}[OLC]{/} " .. message)
end

--- The session's area, or nil with the reason said.
local function area_of(player, session_id)
    if not (DAEMON.olc and DAEMON.olc.is_active(session_id)) then
        fail(player, "You are not building. `olc <area>` to start.")
        return nil
    end
    return DAEMON.olc.get_state(session_id).area_name
end

--- Which kind an id names, guessing from its shape and then from what exists.
---
--- A dotted id is a room — that is the room-id convention and nothing else uses
--- it. Otherwise ask the registries, because an item and a creature can share a
--- word and the answer should be the one that exists rather than the one that
--- sorts first.
local function classify(id)
    if id:find("%.") then return "room" end
    if DAEMON.items and DAEMON.items.get(id) then return "item" end
    if DAEMON.mobs and DAEMON.mobs.get(id) then return "mob" end
    return nil
end

--- Parse `kind:id` or a bare id.
local function target_of(spec)
    local kind, id = spec:match("^(%a+):(.+)$")
    if kind and schema.of(kind) then return kind, id end
    return classify(spec), spec
end

--- `proto:<id>` — a prototype rather than a record. Nil if the spec is not one.
---
--- Which kind is asked of the cursor first, so `olc show proto:beast` while
--- editing a creature means the mob prototype even if an item shares the name.
--- @return string|nil kind, string|nil id, table|nil data
local function proto_target(session_id, spec)
    local id = type(spec) == "string" and spec:match("^proto:(.+)$")
    if not id then return nil end

    local protos = require('prototypes')
    local cursor = DAEMON.olc.cursor(session_id)
    if cursor and protos.get(cursor.kind, id) then
        return cursor.kind, id, protos.get(cursor.kind, id)
    end
    for _, kind in ipairs(schema.kinds()) do
        local data = protos.get(kind, id)
        if data then return kind, id, data end
    end
    return nil, id
end

--- The thing `set` acts on: an explicit target, or the cursor.
local function current(player, session_id)
    local cursor = DAEMON.olc.cursor(session_id)
    if not cursor then
        fail(player, "Nothing is selected. `olc edit <target>`, or `olc edit` for "
            .. "the room you are standing in.")
        return nil
    end
    local draft, err = olc.draft(session_id, cursor.kind, cursor.id)
    if not draft then
        fail(player, tostring(err))
        return nil
    end
    return cursor.kind, cursor.id, draft
end

--- Commit a changed draft to the live world and mark it unsaved.
local function commit(player, session_id, kind, id, draft)
    DAEMON.olc.touch(session_id, kind, id)
    local applied, err = olc.apply_live(kind, draft)
    if not applied then
        fail(player, "The change is held but the live " .. kind
            .. " could not be rebuilt: " .. tostring(err))
    end
end

-- ─── Subcommands ─────────────────────────────────────────────────────────────

local subs = {}

function subs.done(player, session_id)
    if not DAEMON.olc.is_active(session_id) then
        return fail(player, "You are not building.")
    end
    if DAEMON.olc.is_dirty(session_id) then
        fail(player, "You have unsaved changes. `olc save` to write them, or "
            .. "`olc revert` to throw them away.")
        return
    end
    DAEMON.olc.stop(session_id)
    ok(player, "Left build mode.")
end
subs.quit = subs.done

function subs.where(player, session_id)
    local area = area_of(player, session_id)
    if not area then return end

    local cursor = DAEMON.olc.cursor(session_id)
    local standing = DAEMON.world and DAEMON.world.get_character_room(player.char_id)

    local lines = { "{cyan}Area{/}:     " .. area }
    lines[#lines + 1] = "{cyan}Cursor{/}:   "
        .. (cursor and (cursor.kind .. " " .. cursor.id) or "{dim}(none){/}")
    lines[#lines + 1] = "{cyan}Standing{/}: " .. tostring(standing or "nowhere")
    if cursor and cursor.kind == "room" and standing and cursor.id ~= standing then
        -- Worth saying out loud. The cursor deliberately does not follow you,
        -- so this is the one state where `set` writes somewhere you are not.
        lines[#lines + 1] = "{yellow}The cursor is not where you are standing. "
            .. "`olc edit` to bring it here.{/}"
    end
    player:send_lines(lines)
end

function subs.edit(player, session_id, args_str)
    local area = area_of(player, session_id)
    if not area then return end

    local spec = args_str
    if spec == "" then
        spec = DAEMON.world and DAEMON.world.get_character_room(player.char_id)
        if not spec then return fail(player, "You are nowhere.") end
    end

    local kind, id = target_of(spec)
    if not kind then
        return fail(player, "Nothing called '" .. spec .. "'. Try `kind:id`, "
            .. "e.g. `olc edit item:bone_saw`.")
    end

    local draft, err = olc.draft(session_id, kind, id)
    if not draft then return fail(player, tostring(err)) end

    DAEMON.olc.set_cursor(session_id, kind, id)
    ok(player, "Cursor: " .. kind .. " " .. id)
end
subs.here = function(player, session_id) return subs.edit(player, session_id, "") end

function subs.show(player, session_id, args_str)
    if not area_of(player, session_id) then return end

    local kind, id, draft
    if args_str:match("^proto:") then
        -- A prototype is a record minus its id, so `olc.show` renders it with no
        -- adaptation at all. This is the answer to "what am I inheriting".
        local pkind, pid, data = proto_target(session_id, args_str)
        if not pkind then
            return fail(player, "No prototype '" .. tostring(pid) .. "'. `olc protos` lists them.")
        end
        local lines = { "{cyan}" .. pkind .. " prototype{/} " .. pid
            .. "  {dim}(hand-written; OLC never writes prototypes){/}" }
        for _, line in ipairs(olc.show(pkind, data)) do lines[#lines + 1] = line end
        return player:send_paged(table.concat(lines, "\r\n"))
    end

    if args_str ~= "" then
        kind, id = target_of(args_str)
        if not kind then return fail(player, "Nothing called '" .. args_str .. "'.") end
        draft = olc.draft(session_id, kind, id)
        if not draft then return fail(player, "no " .. kind .. " '" .. id .. "'") end
    else
        kind, id, draft = current(player, session_id)
        if not kind then return end
    end

    local lines = { "{cyan}" .. kind .. "{/} " .. id }
    for _, line in ipairs(olc.show(kind, draft)) do lines[#lines + 1] = line end
    player:send_paged(table.concat(lines, "\r\n"))
end

function subs.fields(player, session_id, args_str)
    local kind, base = args_str:match("^(%a+)%s*(%a*)$")
    if not kind or not schema.of(kind) then
        local cursor = DAEMON.olc.cursor(session_id)
        kind = cursor and cursor.kind
        base = nil
    end
    if not kind then
        return fail(player, "Which kind? " .. table.concat(schema.kinds(), ", "))
    end

    local lines = { "{cyan}" .. kind .. "{/} fields" }
    for _, line in ipairs(olc.fields(kind, base ~= "" and base or nil)) do
        lines[#lines + 1] = line
    end
    player:send_paged(table.concat(lines, "\r\n"))
end

function subs.bases(player, session_id, args_str)
    local kind = args_str ~= "" and args_str or "item"
    if not schema.of(kind) then kind = "item" end

    local lines = {}
    if kind == "item" then
        local components = require('components')
        lines[#lines + 1] = "{cyan}Item components{/}  {dim}(`from <name>` or `from comp:<name>`){/}"
        lines[#lines + 1] = "  item          the plain thing — no components"
        for _, name in ipairs(components.names()) do
            lines[#lines + 1] = string.format("  %-13s + %s", name, name)
        end
    end

    -- Two different things share one keyword, so both are listed under it. A
    -- component says what an item *is*; a prototype says what a record starts
    -- from. Only one of them survives into the file as a live link.
    local protos = olc.protos(kind)
    if #protos > 0 then
        lines[#lines + 1] = "{cyan}" .. kind .. " prototypes{/}  {dim}(`from proto:<id>`){/}"
        for _, p in ipairs(protos) do
            lines[#lines + 1] = string.format("  %-26s %s", p.id,
                p.parent and ("{dim}<- " .. p.parent .. "{/}") or "")
        end
    elseif kind ~= "item" then
        return ok(player, "A " .. kind .. " has no components and no prototypes yet. "
            .. "`olc new " .. kind .. " <id>`.")
    end

    player:send_lines(lines)
end

function subs.help(player, session_id, args_str)
    local cursor = DAEMON.olc.cursor(session_id)
    local kind = cursor and cursor.kind
    if args_str == "" or not kind then
        return fail(player, "Which field? `olc help short`, while something is selected.")
    end

    local descriptor = schema.field(kind, args_str)
    if not descriptor then
        return fail(player, "No field '" .. args_str .. "' on a " .. kind .. ".")
    end

    local lines = {
        "{cyan}" .. kind .. "." .. descriptor.name .. "{/}  {yellow}"
            .. (descriptor.type or "string") .. "{/}",
    }
    if descriptor.help then lines[#lines + 1] = "  " .. descriptor.help end
    if descriptor.editable == false then
        lines[#lines + 1] = "  {red}" .. schema.why_not_editable(descriptor) .. "{/}"
    end
    if descriptor.values then
        local values = type(descriptor.values) == "function"
            and descriptor.values() or descriptor.values
        lines[#lines + 1] = "  Accepts: " .. table.concat(values, " ")
    end
    if descriptor.default ~= nil and type(descriptor.default) ~= "table" then
        lines[#lines + 1] = "  Default: " .. tostring(descriptor.default)
    end
    player:send_lines(lines)
end

function subs.set(player, session_id, args_str)
    if not area_of(player, session_id) then return end

    -- `on <target> <field> <value>` is the one-shot form: it writes somewhere
    -- else without moving the cursor. `on` is a reserved word — `schema.RESERVED`
    -- — and a test asserts no field is called that, because the alternative is
    -- deciding by whether the next word happens to resolve as a field, which is
    -- DWIM on a command that writes files.
    local kind, id, draft
    local one_shot, rest = args_str:match("^on%s+(%S+)%s+(.*)$")
    if one_shot then
        kind, id = target_of(one_shot)
        if not kind then return fail(player, "Nothing called '" .. one_shot .. "'.") end
        draft = olc.draft(session_id, kind, id)
        if not draft then return fail(player, "no " .. kind .. " '" .. id .. "'") end
        args_str = rest
    else
        kind, id, draft = current(player, session_id)
        if not kind then return end
    end

    local field, value = args_str:match("^(%S+)%s*(.*)$")
    if not field then return fail(player, "Usage: olc set <field> <value>") end

    local descriptor = schema.field(kind, field, draft)
    if not descriptor then
        return fail(player, "No field '" .. field .. "' on a " .. kind
            .. ". `olc fields` lists them.")
    end

    -- No value, and the field is prose: open the editor rather than clearing it.
    -- Clearing is `olc unset`, said out loud, because a description is an hour's
    -- work and "I pressed enter too early" should not be able to delete it.
    if value == "" and descriptor.type == "text" and DAEMON.editor then
        return DAEMON.editor.open(session_id, {
            title   = id .. "." .. field,
            initial = draft[descriptor.name],
            on_save = function(text)
                local set_ok, err = schema.set(kind, draft, field, text)
                if not set_ok then return fail(player, tostring(err)) end
                commit(player, session_id, kind, id, draft)
                ok(player, id .. "." .. field .. " updated.")
            end,
        })
    end

    -- Setting the prototype itself is checked before it lands, because the
    -- failure modes are ones a builder can neither see nor undo from the record
    -- in front of them: a typo silently inherits nothing, and naming something
    -- that names you back is a chain that never terminates.
    if descriptor.name == "prototype" and value ~= "" then
        local chain, why = proto.chain(kind, value, id)
        if not chain then return fail(player, tostring(why)) end
        for _, layer in ipairs(chain) do
            if layer.id == id then
                return fail(player, "'" .. value .. "' inherits from '" .. id
                    .. "', so this would be a cycle.")
            end
        end
    end

    local before = schema.render(descriptor, draft[descriptor.name])
    local set_ok, err = schema.set(kind, draft, field, value)
    if not set_ok then return fail(player, tostring(err)) end

    commit(player, session_id, kind, id, draft)

    if descriptor.name == "prototype" then
        local copy = {}
        for k, v in pairs(draft) do copy[k] = v end
        local redundant = #proto.thin(kind, copy)
        ok(player, id .. " now inherits from " .. tostring(draft.prototype) .. ".")
        if redundant > 0 then
            player:send("  {yellow}" .. redundant .. " of its current values only restate "
                .. "that prototype. `olc thin` drops them.{/}")
        end
        return
    end
    ok(player, id .. "." .. field .. " = "
        .. schema.render(descriptor, draft[descriptor.name])
        .. "  {dim}(was " .. before .. "){/}")
end

function subs.unset(player, session_id, args_str)
    local kind, id, draft = current(player, session_id)
    if not kind then return end

    local descriptor = schema.field(kind, args_str, draft)
    if not descriptor then return fail(player, "No field '" .. args_str .. "'.") end
    if descriptor.editable == false then
        return fail(player, schema.why_not_editable(descriptor))
    end

    draft[descriptor.name] = nil
    commit(player, session_id, kind, id, draft)

    -- Under a prototype, "unset" means "revert to inherited" rather than
    -- "clear". Saying "cleared" and then showing the old value back is how a
    -- builder concludes the command is broken.
    local origin, source = proto.origin(kind, draft, descriptor.name)
    if origin == "inherited" then
        local merged = olc.effective(kind, draft)
        return ok(player, id .. "." .. descriptor.name .. " is back to what "
            .. tostring(source) .. " says: "
            .. schema.render(descriptor, merged[descriptor.name])
            .. "  {dim}`olc strike " .. descriptor.name .. "` removes it entirely.{/}")
    end
    ok(player, id .. "." .. descriptor.name .. " cleared.")
end

--- Remove an inherited field entirely, rather than reverting to it.
---
--- `custom.lua` deliberately has no delete sentinel, and its reason is good: the
--- generated file is the whole truth there, so "take it out in OLC" is always
--- available. A prototyped record is incomplete by construction — the value is
--- in the *parent's* file — so that argument does not carry across, and without
--- this a child needing one field fewer has to stop inheriting or make the
--- prototype worse.
function subs.strike(player, session_id, args_str)
    local kind, id, draft = current(player, session_id)
    if not kind then return end

    local descriptor = schema.field(kind, args_str, draft)
    if not descriptor then return fail(player, "No field '" .. tostring(args_str) .. "'.") end
    if descriptor.editable == false then
        return fail(player, schema.why_not_editable(descriptor))
    end
    if draft.prototype == nil then
        return fail(player, "'" .. id .. "' has no prototype, so nothing is inherited. "
            .. "`olc unset " .. descriptor.name .. "` clears it.")
    end

    local without = {}
    for k, v in pairs(draft) do without[k] = v end
    without[descriptor.name] = nil
    local origin = proto.origin(kind, without, descriptor.name)
    if origin ~= "inherited" then
        return fail(player, "Nothing inherits '" .. descriptor.name .. "'. "
            .. "`olc unset " .. descriptor.name .. "` clears it.")
    end

    draft[descriptor.name] = proto.NONE
    commit(player, session_id, kind, id, draft)
    ok(player, id .. "." .. descriptor.name .. " struck — removed here, not inherited. "
        .. "{dim}`olc unset " .. descriptor.name .. "` puts it back.{/}")
end

function subs.thin(player, session_id)
    local kind, id = current(player, session_id)
    if not kind then return end

    local removed, err = olc.thin(session_id, kind, id)
    if err then return fail(player, err) end
    if #removed == 0 then
        return ok(player, id .. " restates nothing its prototype already says.")
    end
    ok(player, "Dropped " .. #removed .. " redundant field(s) from " .. id .. ": "
        .. table.concat(removed, ", "))
    player:send("  {dim}They are back to inherited. Anything you set deliberately to "
        .. "the same value went too — `olc set` it again to pin it.{/}")
end

function subs.protos(player, session_id, args_str)
    local kind = (args_str:gsub("s$", "")):lower()
    if not schema.of(kind) then
        local cursor = DAEMON.olc.cursor(session_id)
        kind = cursor and cursor.kind or "mob"
    end

    local list = olc.protos(kind)
    if #list == 0 then
        return ok(player, "No " .. kind .. " prototypes. They are hand-written, in "
            .. "`game/prototypes/*.lua`.")
    end

    local lines = { "{cyan}" .. kind .. " prototypes{/} — " .. #list }
    for _, p in ipairs(list) do
        lines[#lines + 1] = string.format("  %-26s %-26s %s",
            p.id,
            p.parent and ("{dim}<- " .. p.parent .. "{/}") or "",
            p.uses > 0 and ("{dim}" .. p.uses .. " use"
                .. (p.uses == 1 and "" or "s") .. "{/}") or "{dim}unused{/}")
    end
    lines[#lines + 1] = "  {dim}`olc show proto:<id>` for one. `olc new " .. kind
        .. " <id> from proto:<id>` to use one.{/}"
    player:send_paged(table.concat(lines, "\r\n"))
end

--- `add` and `remove` for list fields.
---
--- Arrays are never indexed, and a value is never comma-split: splitting is how
--- a description containing a comma becomes two tags.
local function list_op(player, session_id, args_str, adding)
    local kind, id, draft = current(player, session_id)
    if not kind then return end

    local field, value = args_str:match("^(%S+)%s+(.+)$")
    if not field then
        return fail(player, "Usage: olc " .. (adding and "add" or "remove")
            .. " <field> <value>")
    end

    local descriptor = schema.field(kind, field, draft)
    if not descriptor then return fail(player, "No field '" .. field .. "'.") end
    if descriptor.type ~= "string_array" and descriptor.type ~= "id_array" then
        return fail(player, "'" .. field .. "' is not a list. Use `olc set`.")
    end
    if descriptor.editable == false then
        return fail(player, schema.why_not_editable(descriptor))
    end

    -- The *effective* list, copied. Arrays replace on a prototype merge rather
    -- than union, and this is where that is paid for: a builder adding one tag
    -- to an inherited list gets the whole resulting list written as an override,
    -- rather than having to retype what the prototype already said.
    local list = draft[descriptor.name]
    if list == nil then
        local merged = olc.effective(kind, draft)[descriptor.name]
        list = {}
        for _, v in ipairs(type(merged) == "table" and merged or {}) do list[#list + 1] = v end
    end
    draft[descriptor.name] = list

    local at = nil
    for i, v in ipairs(list) do if v == value then at = i end end

    if adding then
        if at then return ok(player, id .. " already has '" .. value .. "'. No change.") end
        list[#list + 1] = value
    else
        if not at then return ok(player, id .. " does not have '" .. value .. "'.") end
        table.remove(list, at)
    end

    commit(player, session_id, kind, id, draft)
    ok(player, id .. "." .. descriptor.name .. " " .. (adding and "+= " or "-= ")
        .. value .. "  (" .. #list .. " entr" .. (#list == 1 and "y" or "ies") .. ")")
end

function subs.add(player, session_id, args_str)
    return list_op(player, session_id, args_str, true)
end

function subs.remove(player, session_id, args_str)
    return list_op(player, session_id, args_str, false)
end

--- `tag`/`untag` are sugar over the tags list, plus the reverse index.
local function tag_op(player, session_id, args_str, adding)
    local kind, id, draft = current(player, session_id)
    if not kind then return end
    if args_str == "" then
        return fail(player, "Usage: olc " .. (adding and "tag" or "untag") .. " <tag>...")
    end

    for word in args_str:gmatch("%S+") do
        list_op(player, session_id, "tags " .. word, adding)
    end

    -- `tag_d.index` REPLACES rather than appends, which is exactly the semantics
    -- an edit wants — the alternative is a stale tag nothing can remove.
    if DAEMON.tag then
        pcall(DAEMON.tag.index, kind, id, draft.tags or {})
    end
end

function subs.tag(player, session_id, args_str)
    return tag_op(player, session_id, args_str, true)
end

function subs.untag(player, session_id, args_str)
    return tag_op(player, session_id, args_str, false)
end

function subs.comp(player, session_id, args_str)
    local kind, id, draft = current(player, session_id)
    if not kind then return end
    if kind ~= "item" then return fail(player, "Only an item carries components.") end

    local components = require('components')
    local op, name = args_str:match("^(%a+)%s*(%a*)$")
    op = (op or "list"):lower()

    if op == "list" then
        local have = draft.components or {}
        local lines = {
            id .. " carries: " .. (#have > 0 and table.concat(have, ", ") or "{dim}(none){/}"),
            "Available: " .. table.concat(components.names(), "  "),
        }
        return player:send_lines(lines)
    end

    -- `armor.lua` declares `M.component = "armour"`, so both spellings reach the
    -- same component and the one that gets written is always the declared one.
    local resolved = nil
    for _, known in ipairs(components.names()) do
        if known == name or known:gsub("our", "or") == name then resolved = known end
    end
    if not resolved then
        return fail(player, "No component '" .. tostring(name) .. "'. Available: "
            .. table.concat(components.names(), " "))
    end

    draft.components = draft.components or {}
    local at = nil
    for i, c in ipairs(draft.components) do if c == resolved then at = i end end

    if op == "add" then
        if at then return ok(player, id .. " already carries " .. resolved .. ".") end
        draft.components[#draft.components + 1] = resolved
        -- The component's own defaults, so its required fields are not simply
        -- absent the moment it is added.
        for _, f in ipairs(schema.fields_for("item", draft)) do
            if f.component == resolved and f.default ~= nil and draft[f.name] == nil then
                draft[f.name] = f.default
            end
        end
    elseif op == "remove" then
        if not at then return ok(player, id .. " does not carry " .. resolved .. ".") end
        table.remove(draft.components, at)
    else
        return fail(player, "Usage: olc comp add|remove|list <component>")
    end

    commit(player, session_id, kind, id, draft)
    ok(player, id .. " components: "
        .. (#draft.components > 0 and table.concat(draft.components, ", ") or "(none)"))
end

function subs.list(player, session_id, args_str)
    local area = area_of(player, session_id)
    if not area then return end

    local kind = (args_str:gsub("s$", "")):lower()
    if not schema.of(kind) then
        return fail(player, "Usage: olc list rooms|items|mobs")
    end

    local list = olc.merged(session_id, area, kind)
    local lines = { "{cyan}" .. area .. "{/} — " .. #list .. " " .. kind
        .. (#list == 1 and "" or "s") }
    for _, data in ipairs(list) do
        lines[#lines + 1] = string.format("  %-28s %s",
            tostring(data.id), tostring(data.short or ""))
    end
    player:send_paged(table.concat(lines, "\r\n"))
end

function subs.diff(player, session_id)
    if not area_of(player, session_id) then return end

    local changed = DAEMON.olc.changed(session_id)
    if #changed == 0 then return ok(player, "Nothing unsaved.") end

    local lines = { "{cyan}Unsaved{/} — " .. #changed .. " object(s)" }
    for _, c in ipairs(changed) do
        lines[#lines + 1] = string.format("  %-6s %s", c.kind, c.id)
    end
    player:send_lines(lines)
end

function subs.revert(player, session_id, args_str)
    if not area_of(player, session_id) then return end

    if args_str == "" then
        DAEMON.olc.revert(session_id)
        return ok(player, "Every unsaved change discarded. `areas reset` to reload "
            .. "the live world from disk.")
    end

    local kind, id = target_of(args_str)
    if not kind then return fail(player, "Nothing called '" .. args_str .. "'.") end
    DAEMON.olc.revert(session_id, kind, id)
    ok(player, kind .. " " .. id .. " reverted.")
end

function subs.save(player, session_id)
    local area = area_of(player, session_id)
    if not area then return end

    local managed, why = DAEMON.codegen.is_managed(area)
    if not managed then return fail(player, tostring(why)) end

    -- Verify before writing, which is the whole reason drafts are buffered: you
    -- cannot gate a write on a check that runs after the write, and the old OLC
    -- wrote on every `dig`.
    local report = DAEMON.verify and DAEMON.verify.area(area, {
        rooms = olc.merged(session_id, area, "room"),
        items = olc.merged(session_id, area, "item"),
        mobs  = olc.merged(session_id, area, "mob"),
    })
    if report then
        player:send_paged(table.concat(DAEMON.verify.render(report), "\r\n"))
        if report.counts.error > 0 then
            return fail(player, "Not written. Fix the errors, or `olc save force`.")
        end
    end

    local results = olc.save(session_id, area)
    local lines, failed = {}, false
    for _, r in ipairs(results) do
        if r.ok then
            lines[#lines + 1] = string.format("  %-10s %d record%s",
                r.file .. ".lua", r.count, r.count == 1 and "" or "s")
        else
            failed = true
            lines[#lines + 1] = "  {red}" .. r.file .. ".lua — " .. tostring(r.err) .. "{/}"
        end
    end

    if failed then
        fail(player, "Some files were not written:")
        player:send_lines(lines)
        return
    end

    DAEMON.olc.mark_saved(session_id)
    ok(player, "Wrote " .. #results .. " file(s):")
    player:send_lines(lines)
    player:send("  {dim}custom.lua untouched.{/}")
end

function subs.new(player, session_id, args_str)
    local what, rest = args_str:match("^(%a+)%s*(.*)$")
    what = (what or ""):lower()

    if what == "area" then
        return M._new_area(player, session_id, rest)
    end

    if not area_of(player, session_id) then return end
    if not schema.of(what) then
        return fail(player, "Usage: olc new area|room|item|mob <id> [from <base>]")
    end

    local id, base = rest:match("^(%S+)%s+from%s+(%S+)$")
    if not id then id = rest:match("^(%S+)$") end
    if not id then
        return fail(player, "Usage: olc new " .. what .. " <id> [from <base>]")
    end

    -- A room id is dotted and belongs to this area. Accepting a bare name and
    -- prefixing it is what `dig` does, and the two should agree.
    local area = DAEMON.olc.get_state(session_id).area_name
    if what == "room" and not id:find("%.") then id = area .. "." .. id end

    local draft, err = olc.create(session_id, what, id, base)
    if not draft then return fail(player, tostring(err)) end

    DAEMON.olc.set_cursor(session_id, what, id)

    local missing = {}
    for _, f in ipairs(schema.fields_for(what, draft)) do
        if f.required and draft[f.name] == nil then missing[#missing + 1] = f.name end
    end

    ok(player, "Created " .. what .. " '" .. id .. "'"
        .. (base and (" from base '" .. base .. "'") or "") .. ".")
    if #missing > 0 then
        player:send("  {yellow}Unset and required: " .. table.concat(missing, ", ") .. "{/}")
    end
    player:send("  {dim}Cursor moved here. `olc fields` for what you can set.{/}")
end

--- `olc new area` — the skeleton, and the `managed` flag that gates every write.
function M._new_area(player, session_id, args_str)
    local name, title = args_str:match("^(%S+)%s*(.*)$")
    if not name then return fail(player, "Usage: olc new area <name> [title...]") end
    name = name:lower()

    if not name:match("^[%a_][%w_]*$") then
        return fail(player, "An area name has to be a plain identifier: letters, "
            .. "digits and underscores. It becomes a directory and a room-id prefix.")
    end
    if not has_permission(session_id, "cmd.olc.areas") then
        return fail(player, "Creating an area needs 'cmd.olc.areas'.")
    end

    if DAEMON.codegen.read(name, "_meta") then
        return fail(player, "'" .. name .. "' already exists. `olc " .. name .. "` to build in it.")
    end

    local meta_ok, meta_err = DAEMON.codegen.write_meta(name, {
        title  = title ~= "" and title or name:gsub("_", " "):gsub("^%l", string.upper),
        author = player.name or "Unknown",
        status = "building",
    })
    if not meta_ok then return fail(player, "Could not write _meta.lua: " .. tostring(meta_err)) end

    DAEMON.olc.start(session_id, name)

    local entrance = name .. ".entrance"
    local draft = olc.create(session_id, "room", entrance)
    if draft then
        draft.short = "The Entrance"
        olc.apply_live("room", draft)
    end
    DAEMON.olc.set_cursor(session_id, "room", entrance)

    if DAEMON.world then
        pcall(DAEMON.world.move_character, player.char_id, entrance)
    end

    ok(player, "Created area '" .. name .. "'. You are in " .. entrance .. ".")
    player:send("  {dim}`olc save` writes it. `custom.lua` is yours and is never "
        .. "regenerated.{/}")
end

function subs.adopt(player, session_id, args_str)
    if args_str == "" then return fail(player, "Usage: olc adopt <area> [--confirm]") end
    local name = args_str:match("^(%S+)")
    local confirm = args_str:find("--confirm", 1, true) ~= nil

    if not DAEMON.adopt then
        return fail(player, "Adoption is unavailable (adopt_d is not loaded).")
    end
    local lines = DAEMON.adopt.run(player, name, confirm)
    player:send_paged(table.concat(lines, "\r\n"))
end

function subs.enter(player, session_id, args_str)
    return M._enter(player, session_id, args_str)
end

--- Enter an area, refusing one OLC does not manage.
function M._enter(player, session_id, area_name)
    area_name = area_name:lower()

    local managed, why = DAEMON.codegen.is_managed(area_name)
    if not managed then return fail(player, tostring(why)) end

    if DAEMON.olc.is_active(session_id) then
        if DAEMON.olc.is_dirty(session_id) then
            return fail(player, "You have unsaved changes in '"
                .. DAEMON.olc.get_state(session_id).area_name
                .. "'. `olc save` or `olc revert` first.")
        end
        DAEMON.olc.stop(session_id)
    end

    DAEMON.olc.start(session_id, area_name)
    ok(player, "Building '" .. area_name .. "'. `olc edit` to select the room you "
        .. "are standing in.")
end

-- ─── Dispatch ────────────────────────────────────────────────────────────────

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not (DAEMON.olc and DAEMON.codegen) then
        return fail(player, "OLC is unavailable (olc_d or codegen_d is not loaded).")
    end

    args_str = (args_str or ""):gsub("^%s+", ""):gsub("%s+$", "")

    -- No argument: status if building, usage if not.
    if args_str == "" then
        if DAEMON.olc.is_active(session_id) then
            return subs.where(player, session_id, "")
        end
        return player:send_paged(table.concat(M.usage, "\r\n"))
    end

    local verb, rest = args_str:match("^(%S+)%s*(.*)$")
    verb = verb:lower()

    local handler = subs[verb]
    if handler then
        return handler(player, session_id, rest)
    end

    -- Not a subcommand: it is an area name. Last, so a future subcommand can
    -- never be shadowed by somebody's area — the reverse would mean adding a
    -- verb silently broke whoever had named an area after it.
    if rest == "" then
        return M._enter(player, session_id, verb)
    end

    fail(player, "Unknown option '" .. verb .. "'.")
    player:send_paged(table.concat(M.usage, "\r\n"))
end

return M
