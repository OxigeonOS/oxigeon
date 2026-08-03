-- mudlib/daemons/cooldown_d.lua — "not yet" gates, per character.
--
-- Stores the moment a thing becomes available again, never the moment it was
-- last used. Storing expiry means the check does not have to know the duration,
-- so changing a cooldown from 24 hours to 12 does not retroactively rewrite
-- everyone's remaining time into something wrong.
--
-- Two homes, chosen by how long the gate lasts:
--
--   >= game.cooldown_durable_seconds (60)   written immediately; survives a restart
--   <  game.cooldown_durable_seconds        memory only; forgotten on restart
--
-- The threshold rather than a mandatory flag, because the two mistakes are not
-- equally loud. Forget the flag on a 2-second ability cooldown and you write to
-- disk on every use by every player — a slow leak that surfaces months later as
-- "the game feels sluggish" with nothing obvious to blame. Forget it on a daily
-- reward and the gate resets on the next restart, which a player reports the
-- same day. The default belongs on the side of the loud failure.
--
--   Under a minute it is a game mechanic. Over a minute it is a promise to
--   the player.
--
-- Pass `{ durable = true }` or `{ durable = false }` for the cases that rule
-- gets wrong — a 10-second gate on a rare reward, a 5-minute one nobody minds
-- losing.
--
-- Exposes:
--   DAEMON.cooldown.mark(char_id, what, seconds, opts) -> expires_at | false
--   DAEMON.cooldown.remaining(char_id, what)           -> seconds (0 = ready)
--   DAEMON.cooldown.ready(char_id, what)               -> boolean
--   DAEMON.cooldown.expires_at(char_id, what)          -> unix seconds | nil
--   DAEMON.cooldown.clear(char_id, what)               -> boolean
--   DAEMON.cooldown.clear_all(char_id)                 -> count
--   DAEMON.cooldown.list(char_id)                      -> array
--
-- See docs/src/lua-api/state-cache.md.

local M = {}

local DURABLE = "cooldowns"
local FAST    = "cooldowns_fast"

local function log_warn(message)
    log("warn", message)
    if DAEMON and DAEMON.journal then
        pcall(DAEMON.journal.warn, message)
    end
end

--- The stored value *is* the expiry, so the cache can prune dead gates on its
--- own without a reaper of any kind.
local function expiry_of(_, value)
    return type(value) == "number" and value or nil
end

do
    if DAEMON and DAEMON.cache then
        DAEMON.cache.define(DURABLE, {
            tier              = "write_through",
            scope_prefix      = "char:",
            owner             = "char",
            preload           = true,
            delete_when_empty = true,
            expiry_of         = expiry_of,
        })
        DAEMON.cache.define(FAST, {
            tier         = "memory",
            scope_prefix = "char:",
            owner        = "char",
            expiry_of    = expiry_of,
        })
    else
        log("error", "COOLDOWN_D: cache_d is not loaded — cooldowns will not work")
    end
end

local function threshold()
    local ok, v = pcall(config, "game.cooldown_durable_seconds")
    if ok and type(v) == "number" and v > 0 then return v end
    return 60
end

local function valid(char_id, what, who)
    if type(char_id) ~= "number" and type(char_id) ~= "string" then
        log_warn("COOLDOWN_D." .. who .. ": char_id must be a number or string, got " .. type(char_id))
        return false
    end
    if type(what) ~= "string" or #what == 0 then
        log_warn("COOLDOWN_D." .. who .. ": the cooldown name must be a non-empty string")
        return false
    end
    return true
end

--- Which tier is this gate in? Answered by looking, not by recomputing the
--- threshold — so changing the config, or an explicit `durable` override, can
--- never strand a gate in a namespace nobody reads any more.
local function read_both(char_id, what)
    if not (DAEMON and DAEMON.cache) then return nil end
    local durable = DAEMON.cache.get(DURABLE, char_id, what)
    local fast    = DAEMON.cache.get(FAST, char_id, what)
    if type(durable) ~= "number" then durable = nil end
    if type(fast) ~= "number" then fast = nil end
    if durable and fast then return math.max(durable, fast) end
    return durable or fast
end

--- Start (or extend) a cooldown.
--- @param seconds number  how long from now
--- @param opts table|nil  { durable = true|false } to override the threshold
--- @return number|false   the expiry timestamp
function M.mark(char_id, what, seconds, opts)
    if not valid(char_id, what, "mark") then return false end
    if type(seconds) ~= "number" or seconds <= 0 then
        log_warn("COOLDOWN_D.mark('" .. what .. "'): seconds must be a positive number, got "
            .. tostring(seconds))
        return false
    end
    if not (DAEMON and DAEMON.cache) then return false end

    local durable = opts and opts.durable
    if durable == nil then durable = seconds >= threshold() end

    local expires = os_time() + seconds
    local ns = durable and DURABLE or FAST

    -- Moving between tiers (an explicit override, or a duration that crossed
    -- the threshold) must not leave the old copy behind to be found by
    -- `remaining`.
    local other = durable and FAST or DURABLE
    if DAEMON.cache.get(other, char_id, what) ~= nil then
        DAEMON.cache.delete(other, char_id, what)
    end

    if not DAEMON.cache.set(ns, char_id, what, expires, { expires_at = expires }) then
        return false
    end
    return expires
end

--- Seconds until this is available again. 0 means ready now.
function M.remaining(char_id, what)
    if not valid(char_id, what, "remaining") then return 0 end
    local expires = read_both(char_id, what)
    if not expires then return 0 end
    local left = expires - os_time()
    if left <= 0 then return 0 end
    return left
end

function M.ready(char_id, what)
    return M.remaining(char_id, what) <= 0
end

--- The raw expiry timestamp, for a caller that wants to format a date rather
--- than a countdown.
function M.expires_at(char_id, what)
    if not valid(char_id, what, "expires_at") then return nil end
    return read_both(char_id, what)
end

function M.clear(char_id, what)
    if not valid(char_id, what, "clear") then return false end
    if not (DAEMON and DAEMON.cache) then return false end
    local a = DAEMON.cache.delete(DURABLE, char_id, what)
    local b = DAEMON.cache.delete(FAST, char_id, what)
    return a or b
end

function M.clear_all(char_id)
    if not (DAEMON and DAEMON.cache) then return 0 end
    local n = 0
    for _, ns in ipairs({ DURABLE, FAST }) do
        for _, key in ipairs(DAEMON.cache.keys(ns, char_id)) do
            if DAEMON.cache.delete(ns, char_id, key) then n = n + 1 end
        end
    end
    return n
end

--- Everything currently gating this character, ready ones already dropped.
--- @return table  array of { what, remaining, expires_at, durable }
function M.list(char_id)
    local out = {}
    if not (DAEMON and DAEMON.cache) then return out end
    local now = os_time()
    for _, entry in ipairs({ { DURABLE, true }, { FAST, false } }) do
        local ns, durable = entry[1], entry[2]
        for _, key in ipairs(DAEMON.cache.keys(ns, char_id)) do
            local expires = DAEMON.cache.get(ns, char_id, key)
            if type(expires) == "number" and expires > now then
                out[#out + 1] = {
                    what = key,
                    remaining = expires - now,
                    expires_at = expires,
                    durable = durable,
                }
            end
        end
    end
    table.sort(out, function(a, b) return a.remaining < b.remaining end)
    return out
end

log("info", "cooldown_d daemon loaded")

return M
