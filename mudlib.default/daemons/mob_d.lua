-- mudlib/daemons/mob_d.lua — Creatures: templates, instances, and where they are.
--
-- Mirrors ITEM_D: the game layer authors templates as plain data
-- (game/areas/*/mobs.lua), this daemon registers them, and `spawn` turns a
-- template into a live Mobile in a room. A template is shared and never
-- mutated; an instance has its own id, its own hit points and its own effects.
--
-- Instances are entirely in memory. A mob is not worth saving: if the server
-- restarts, the rat is a new rat. That is the durability rule applied — see
-- docs/src/lua-api/state-cache.md — and it is why nothing here touches the
-- database.
--
-- Exposes:
--   DAEMON.mobs.register(template) / register_all(list) / get(id) / all()
--   DAEMON.mobs.spawn(template_id, room_id) -> mob | nil
--   DAEMON.mobs.despawn(mob, opts)
--   DAEMON.mobs.in_room(room_id)            -> array of mobs
--   DAEMON.mobs.find_in_room(room_id, name) -> mob | nil
--   DAEMON.mobs.get_instance(instance_id)   -> mob | nil
--   DAEMON.mobs.populate()                  -> count
--   DAEMON.mobs.count()
--
-- See docs/src/lua-api/combat.md.

local Mobile  = require('lib.mobile')
local persist = require('lib.persist')

local matching = require('lib.matching')

local M = {}

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

local function log_warn(message)
    log("warn", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.warn, message) end
end

--- Templates come from the game layer and instances are live objects, so both
--- have to outlive a reload of this file — otherwise every mob in the world
--- would vanish the moment a builder reloaded a daemon. Cached in an upvalue,
--- since `get_persistent` crosses into Rust on every call.
local S = nil

local function root()
    if S then return S end
    S = persist.root("mob_d", 1, function()
        return { templates = {}, instances = {}, rooms = {}, seq = 0, alive = 0 }
    end)
    return S
end

-- ─── Templates ───────────────────────────────────────────────────────────────

--- Register one mob template.
--- @param template table  { id, short, name, description, stats, xp_award,
---                          damage = { min, max }, spawn_room, count,
---                          respawn_time, loot_table, aggressive }
--- @return boolean
function M.register(template)
    if type(template) ~= "table" or type(template.id) ~= "string" or #template.id == 0 then
        log_warn("MOB_D.register: a mob template needs a string id")
        return false
    end
    local r = root()
    r.templates[template.id] = template
    return true
end

function M.register_all(list)
    if type(list) ~= "table" then
        log_warn("MOB_D.register_all: expected an array of templates")
        return 0
    end
    local n = 0
    for _, template in ipairs(list) do
        if M.register(template) then n = n + 1 end
    end
    log("info", "MOB_D: registered " .. n .. " mob template(s)")
    return n
end

function M.get(id)  return root().templates[id] end

function M.all()
    local out = {}
    for id in pairs(root().templates) do out[#out + 1] = id end
    table.sort(out)
    return out
end

-- ─── Instances ───────────────────────────────────────────────────────────────

local function room_set(r, room_id)
    r.rooms[room_id] = r.rooms[room_id] or {}
    return r.rooms[room_id]
end

--- Create a live mob from a template and put it in a room.
--- @return table|nil  the Mobile
function M.spawn(template_id, room_id)
    local r = root()
    local template = r.templates[template_id]
    if not template then
        log_warn("MOB_D.spawn: no such mob template '" .. tostring(template_id) .. "'")
        return nil
    end
    if type(room_id) ~= "string" then
        log_warn("MOB_D.spawn('" .. template_id .. "'): needs a room id")
        return nil
    end

    r.seq = r.seq + 1
    local instance_id = "mob:" .. r.seq

    -- A shallow copy so the instance can take damage without wounding every
    -- other rat that shares the template.
    local data = {}
    for k, v in pairs(template) do data[k] = v end
    data.id = instance_id
    data.stats = {}
    for k, v in pairs(template.stats or {}) do data.stats[k] = v end

    local ok, mob = pcall(Mobile.new, Mobile, data)
    if not ok or not mob then
        log_error("MOB_D.spawn('" .. template_id .. "') failed: " .. tostring(mob))
        return nil
    end

    mob.template_id = template_id
    mob.room_id     = room_id
    mob.name        = template.name or template.short or template_id
    -- Mobile:new only knows about the fields it declares, and these two are
    -- combat's business rather than the class library's.
    mob.damage      = template.damage
    -- What its unarmed attacks count as. A wisp deals magic with nothing in
    -- its hands, and without this the only way to have a damage type is to be
    -- holding something.
    mob.damage_type = template.damage_type
    mob.xp_award    = template.xp_award
    -- How often it attacks with nothing in its hands. Combat's business, like
    -- `damage` above — and on this list for the same reason, which is that
    -- every class constructor in the mudlib copies a fixed set of fields and a
    -- new one is invisible until somebody adds it to the right list.
    mob.speed       = template.speed
    -- What it is made of, for hit locations. Nil is the ordinary case.
    mob.body        = template.body

    -- The character set fills in whatever the template did not say, then the
    -- gauges are clamped. A mob carries the same stat block a player does.
    if DAEMON and DAEMON.trait then pcall(DAEMON.trait.seed, mob, "character") end

    -- The template's own `on_death`, if it has one, then the event. It used to
    -- be *replaced* here, which meant a template could declare the hook and
    -- never see it called — a boss that drops a corpse would silently drop
    -- nothing. Wrapped rather than ordered the other way round, so the hook
    -- runs while the world still looks the way it did when the creature died.
    local template_on_death = mob.on_death

    mob.on_death = function(self)
        if type(template_on_death) == "function" then
            local ok, err = pcall(template_on_death, self)
            if not ok then
                log_error("MOB_D: on_death for '" .. tostring(self.template_id)
                    .. "' raised: " .. tostring(err))
            end
        end

        if DAEMON and DAEMON.event then
            DAEMON.event.emit("mob.died", {
                instance_id = self.id,
                template_id = self.template_id,
                room_id     = self.room_id,
                -- Who killed it, when there was a who. A quest counter, a
                -- faction's grudge and a loot rule all want this, and none of
                -- them should have to reach into combat to get it.
                killer_char_id = self._killed_by and self._killed_by.char_id,
                killer_id      = self._killed_by and self._killed_by.id,
            })
        end
    end

    r.instances[instance_id] = mob
    room_set(r, room_id)[instance_id] = true
    r.alive = r.alive + 1

    -- Into the tag index, so "every guard in this faction" is a lookup rather
    -- than a walk over every live mob in the world.
    if DAEMON and DAEMON.tag then
        pcall(DAEMON.tag.index, "mob", instance_id, mob.tags)
    end

    return mob
end

--- Remove a mob from the world.
--- @param opts table|nil  { respawn = true } to schedule its return
function M.despawn(mob, opts)
    if type(mob) ~= "table" or not mob.id then return false end
    local r = root()
    if not r.instances[mob.id] then return false end

    opts = opts or {}
    local template_id, room_id = mob.template_id, mob.room_id

    if DAEMON and DAEMON.effect then pcall(DAEMON.effect.detach, mob) end
    if DAEMON and DAEMON.trait then pcall(DAEMON.trait.detach, mob) end
    if DAEMON and DAEMON.combat then pcall(DAEMON.combat.disengage, mob) end
    -- Instance ids are `mob:<seq>` and never reused, so a scope left behind is
    -- retained for the life of the process. Nothing else evicts it: the
    -- ability namespaces are `owner = "none"` precisely because a creature does
    -- not disconnect.
    if DAEMON and DAEMON.ability then pcall(DAEMON.ability.detach, mob) end
    if DAEMON and DAEMON.queue then pcall(DAEMON.queue.detach, mob) end
    if DAEMON and DAEMON.cooldown then pcall(DAEMON.cooldown.clear_all, mob) end

    -- Object state is keyed by object id in a plain table in `_G`, and mob
    -- instance ids are `"mob:" .. seq` — monotonic and never reused. Everything
    -- else here was detached and this was not, so every mob that ever had state
    -- written left a permanently retained sub-table behind, and a respawn loop
    -- churned ids forever. The only pruning anywhere is `world_d`'s on area
    -- reset, which covers rooms in a registered area source: not mobs.
    --
    -- The cost is not the memory a few tables hold. It is that every mark phase
    -- has to walk them, forever, and nothing measured that until `mudstatus`
    -- grew a heap counter.
    if type(clear_object_state) == "function" then
        local ok, err = pcall(clear_object_state, mob.id)
        if not ok then
            log_error("MOB_D: could not clear object state for '" .. tostring(mob.id)
                .. "': " .. tostring(err))
        end
    end

    if DAEMON and DAEMON.tag then pcall(DAEMON.tag.forget, "mob", mob.id) end

    r.instances[mob.id] = nil
    if r.rooms[room_id] then r.rooms[room_id][mob.id] = nil end
    r.alive = math.max(0, r.alive - 1)

    local template = r.templates[template_id]
    if opts.respawn and template and template.respawn_time and DAEMON and DAEMON.ticker then
        local timer_id = "mob.respawn." .. template_id .. "." .. tostring(r.seq)
        DAEMON.ticker.after(template.respawn_time, timer_id, function()
            if DAEMON and DAEMON.mobs then
                local ok, err = pcall(DAEMON.mobs.spawn, template_id, room_id)
                if not ok then
                    log_error("MOB_D: respawn of '" .. tostring(template_id) .. "' failed: " .. tostring(err))
                end
            end
        end)
    end
    return true
end

function M.get_instance(instance_id)
    return root().instances[instance_id]
end

--- Every live mob in the world, in a stable order.
---
--- `in_room` answers "what is here", which is what the game needs. Admin
--- tooling needs "where is that thing" — `objdump rat` should find the rat
--- whether or not you are standing next to it — and there was no way to ask.
--- @return table  array of Mobiles, ordered by instance id
function M.instances()
    local out = {}
    for _, mob in pairs(root().instances) do out[#out + 1] = mob end
    table.sort(out, function(a, b) return tostring(a.id) < tostring(b.id) end)
    return out
end

--- Find a live mob anywhere by name, keyword, template or instance id.
--- Prefers an exact instance id, then the room the searcher is standing in,
--- so `objdump rat` means the rat in front of you when there is one.
--- @param name string
--- @param near_room_id string|nil  searched first, if given
--- @return table|nil
function M.find_anywhere(name, near_room_id)
    if type(name) ~= "string" or #name == 0 then return nil end

    local exact = root().instances[name]
    if exact then return exact end

    if near_room_id then
        local here = M.find_in_room(near_room_id, name)
        if here then return here end
    end

    local needle = name:lower()
    for _, mob in ipairs(M.instances()) do
        for _, field in ipairs({ mob.name, mob.short, mob.template_id }) do
            if type(field) == "string" and field:lower():find(needle, 1, true) then
                return mob
            end
        end
    end
    return nil
end

--- Every live mob in a room, in a stable order.
function M.in_room(room_id)
    local r = root()
    local out = {}
    for instance_id in pairs(r.rooms[room_id] or {}) do
        local mob = r.instances[instance_id]
        if mob then out[#out + 1] = mob end
    end
    -- **Numerically on the sequence**, not lexically on the whole id. The ids
    -- are `mob:<n>`, so a string sort puts `mob:10` before `mob:2` and the
    -- order stops being spawn order after nine of anything. That matters now
    -- that `1.rat` means "the oldest rat here" — an order nobody can predict
    -- makes the ordinal useless.
    local function seq_of(mob)
        return tonumber(tostring(mob.id):match("(%d+)$")) or 0
    end
    table.sort(out, function(a, b)
        local sa, sb = seq_of(a), seq_of(b)
        if sa ~= sb then return sa < sb end
        return tostring(a.id) < tostring(b.id)
    end)
    return out
end

--- Find a mob in a room by name or keyword. Matches a substring, so
--- `attack rat` finds "a grey rat", and understands `attack 2.rat`.
---
--- **Returns `nil, <listing>` when several match and no ordinal was given.**
--- The second return is the same shape a caller already handles for "no such
--- thing", so a call site cannot silently take the first of three rats — see
--- `lib/matching.lua` for why that is worth refusing.
--- @return table|nil mob, string|nil why
function M.find_in_room(room_id, name)
    if type(name) ~= "string" or #name == 0 then return nil, nil end
    return matching.choose(
        M.in_room(room_id), name,
        function(mob) return { mob.name, mob.short, mob.template_id } end,
        -- **The short, whatever `display_name_prefers` says.** That key is
        -- about prose voice — "you hit wisp" against "you hit a pale wisp" —
        -- and a disambiguation list is not prose. Its whole job is to tell
        -- three creatures apart, and under `prefers = "name"` it would print
        -- `rat` three times and distinguish nothing.
        function(mob)
            return mob.short or mob.name or "something"
        end)
end

--- Move a mob between rooms.
function M.move(mob, room_id)
    if type(mob) ~= "table" or not mob.id then return false end
    local r = root()
    if r.rooms[mob.room_id] then r.rooms[mob.room_id][mob.id] = nil end
    mob.room_id = room_id
    room_set(r, room_id)[mob.id] = true
    return true
end

--- Spawn every template that says where it lives, up to its count.
---
--- Idempotent: a template already at its population is left alone, so this can
--- be called on an area reset without the world filling up with rats.
--- @return number  how many were spawned
function M.populate()
    local r = root()
    local n = 0
    local ids = {}
    for id in pairs(r.templates) do ids[#ids + 1] = id end
    table.sort(ids)

    for _, id in ipairs(ids) do
        local template = r.templates[id]
        if template.spawn_room then
            local want = template.count or 1
            local have = 0
            for _, mob in ipairs(M.in_room(template.spawn_room)) do
                if mob.template_id == id then have = have + 1 end
            end
            for _ = have + 1, want do
                if M.spawn(id, template.spawn_room) then n = n + 1 end
            end
        end
    end
    if n > 0 then log("info", "MOB_D: spawned " .. n .. " mob(s)") end
    return n
end

function M.count() return root().alive end

--- Describe the mobs in a room, for `look`.
--- @return string|nil
function M.describe_room(room_id)
    local lines = {}
    for _, mob in ipairs(M.in_room(room_id)) do
        local Object = require('lib.object')
        local short = Object.resolve(mob.short, mob) or mob.name or "something"
        if mob:is_alive() then
            lines[#lines + 1] = short .. " is here."
        end
    end
    if #lines == 0 then return nil end
    return table.concat(lines, "\r\n")
end

log("info", "mob_d daemon loaded")

return M
