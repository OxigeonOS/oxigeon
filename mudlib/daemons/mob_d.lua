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
    mob.xp_award    = template.xp_award

    -- Traits fill in whatever the template did not say and clamp the gauges.
    if DAEMON and DAEMON.trait then pcall(DAEMON.trait.attach, mob) end

    mob.on_death = function(self)
        if DAEMON and DAEMON.event then
            DAEMON.event.emit("mob.died", {
                instance_id = self.id,
                template_id = self.template_id,
                room_id     = self.room_id,
            })
        end
    end

    r.instances[instance_id] = mob
    room_set(r, room_id)[instance_id] = true
    r.alive = r.alive + 1
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

--- Every live mob in a room, in a stable order.
function M.in_room(room_id)
    local r = root()
    local out = {}
    for instance_id in pairs(r.rooms[room_id] or {}) do
        local mob = r.instances[instance_id]
        if mob then out[#out + 1] = mob end
    end
    table.sort(out, function(a, b) return a.id < b.id end)
    return out
end

--- Find a mob in a room by name or keyword. Matches a prefix, so "attack rat"
--- finds "a grey rat".
function M.find_in_room(room_id, name)
    if type(name) ~= "string" or #name == 0 then return nil end
    local needle = name:lower()
    for _, mob in ipairs(M.in_room(room_id)) do
        local candidates = { mob.name, mob.short, mob.template_id }
        for _, field in ipairs(candidates) do
            if type(field) == "string" then
                local haystack = field:lower()
                if haystack == needle or haystack:find(needle, 1, true) then
                    return mob
                end
            end
        end
    end
    return nil
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
