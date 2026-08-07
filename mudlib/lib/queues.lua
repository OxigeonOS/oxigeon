-- mudlib/lib/queues.lua — The arithmetic and the shape of an action queue.
--
-- The `lib/abilities.lua` half of `queue_d`: everything a track needs that is a
-- pure function of its arguments. No `DAEMON`, no clock, no world.
--
-- ─── The three rules the whole thing rests on ────────────────────────────────
--
-- 1. **Roundtime is recovery, not occupation.** It says "this track may not act
--    again for N seconds". It does not say "you are busy" — `ability_d`'s
--    `cast_time` and `channel` already own that, and because these are
--    different things they need no arbitration between them.
--
-- 2. **Roundtime lives on a *track*, and only actions on that track consult
--    it.** Nothing in command dispatch reads a track, so `look`, `say` and `who`
--    work in roundtime — not by an exemption list, but because they never enter
--    the code path at all.
--
-- 3. **The queue holds intent; something else holds the thing in flight.** A
--    cast is not the head of the queue. It is what happens after the head is
--    dequeued and turns out to take time.
--
-- ─── Tracks, and why they are named rather than hardcoded ────────────────────
--
-- Combat is the first track. Crafting and gathering are meant to feel like the
-- same mini-game later, with their own pace and their own idea of a round — so
-- a track carries its own round trait, its own bound and its own empty-queue
-- policy, and registering one is all a game has to do. The test of whether that
-- is really generic is whether a second track needs a mudlib edit. It does not.
--
-- Exposes:
--   Queues.rt_key(track)                        -> "rt.<track>"
--   Queues.rounds_to_seconds(rounds, length)    -> integer seconds
--   Queues.normalise_track(spec)                -> spec, err
--   Queues.normalise_entry(entry)               -> entry, err
--   Queues.push(queue, entry, opts)             -> ok, why
--   Queues.pop(queue)                           -> entry | nil
--   Queues.remember(history, entry, keep)
--   Queues.is_stale(entry, now, stale)          -> boolean
--   Queues.POLICIES
--
-- See docs/src/lua-api/queues.md.

local M = {}

--- What an empty queue does. Resolved per entity, then per track.
M.POLICIES = { auto = true, idle = true, repeat_ = true }

--- The cooldown key a track's roundtime lives under.
---
--- Spelled here and nowhere else, mirroring `Abilities.cooldown_key`. Roundtime
--- is an ordinary `cooldown_d` entry: always under a minute, so its threshold
--- rule already puts it in memory and forgets it on restart, which is exactly
--- right and costs nothing. It also means `cooldown list` answers "why can't I
--- swing" with no new code.
--- @param track string
--- @return string
function M.rt_key(track)
    return "rt." .. tostring(track or "combat")
end

-- ─── Rounds ──────────────────────────────────────────────────────────────────

--- Turn a number of rounds into whole seconds.
---
--- **Ceiled, and that is the clock's decision rather than a taste one.**
--- `os_time()` returns integer seconds deliberately — nothing in the mudlib
--- wants sub-second game time. So an expiry of `now + 2.25` is observed at
--- one-second granularity: the gate opens somewhere in [2s, 3s] depending on
--- where inside the second it happened to be marked, which is invisible and
--- unreproducible. A stated 3 is better than an unstated 2-or-3.
---
--- Floored at 1, because a roundtime of zero is not a roundtime and an ability
--- that wants none should not declare one.
--- @param rounds number
--- @param round_length number  seconds in one of this track's rounds
--- @return number  whole seconds
function M.rounds_to_seconds(rounds, round_length)
    local r = tonumber(rounds) or 0
    local len = tonumber(round_length) or 0
    if r <= 0 or len <= 0 then return 0 end
    return math.max(1, math.ceil(r * len))
end

-- ─── Tracks ──────────────────────────────────────────────────────────────────

--- Fill a track spec out, or say why it cannot be one.
--- @param spec table
--- @return table|nil spec, string|nil err
function M.normalise_track(spec)
    if type(spec) ~= "table" then return nil, "a track spec must be a table" end
    if type(spec.id) ~= "string" or spec.id == "" then
        return nil, "a track needs a string id"
    end

    -- Which derived trait is one round for this track. A game names
    -- `round_length` for combat and `craft_round_length` for crafting, and
    -- neither is hardcoded anywhere.
    spec.round_trait   = spec.round_trait or "round_length"
    spec.round_seconds = tonumber(spec.round_seconds) or 3
    spec.max           = tonumber(spec.max) or 3
    spec.history       = tonumber(spec.history) or 5
    spec.stale         = tonumber(spec.stale) or 30
    spec.empty         = spec.empty or "idle"

    if not M.POLICIES[spec.empty == "repeat" and "repeat_" or spec.empty] then
        return nil, "'" .. tostring(spec.empty) .. "' is not an empty-queue policy. "
            .. "One of: auto idle repeat"
    end
    if spec.max < 1 then return nil, "a track's `max` must be at least 1" end

    return spec
end

--- Fill an entry out, or say why it cannot be one.
---
--- An entry holds the **resolved target entity**, never a name. Same reasoning
--- as a cast record: re-resolving by name at dequeue would let somebody retarget
--- you by walking a creature with a matching prefix into the room.
--- @return table|nil entry, string|nil err
function M.normalise_entry(entry)
    if type(entry) ~= "table" then return nil, "a queue entry must be a table" end
    entry.kind = entry.kind or "ability"
    if entry.kind ~= "ability" and entry.kind ~= "attack" then
        return nil, "'" .. tostring(entry.kind) .. "' is not a queue entry kind"
    end
    if entry.kind == "ability" and (type(entry.id) ~= "string" or entry.id == "") then
        return nil, "an ability entry needs a string id"
    end
    return entry
end

-- ─── The queue itself ────────────────────────────────────────────────────────

--- Add an entry, honouring the bound.
---
--- **When it is full the newest is refused, not the oldest dropped.** Silently
--- discarding something the player already committed to is the worse failure:
--- a refusal tells them the queue is full, which is information they can act on.
--- @param queue table  array, mutated
--- @param opts table|nil { front = true, max = n }
--- @return boolean ok, string|nil why
function M.push(queue, entry, opts)
    opts = opts or {}
    local max = tonumber(opts.max) or 3
    if #queue >= max then
        return false, "You have too much planned already."
    end
    if opts.front then
        table.insert(queue, 1, entry)
    else
        queue[#queue + 1] = entry
    end
    return true
end

--- Take the head.
--- @return table|nil
function M.pop(queue)
    if #queue == 0 then return nil end
    return table.remove(queue, 1)
end

--- Has this entry been sitting long enough that acting on it would surprise?
---
--- A queue stuffed during a lag spike replaying a minute later is the single
--- most common way an action queue feels broken.
--- @return boolean
function M.is_stale(entry, now, stale)
    if type(entry) ~= "table" or type(entry.at) ~= "number" then return false end
    local limit = tonumber(stale) or 0
    if limit <= 0 then return false end
    return (now - entry.at) > limit
end

--- Record what completed, newest first.
---
--- **Ids and numbers only, never an entity reference** — so a corpse is not
--- retained by the history of the fight that killed it, and so this could move
--- to a written tier later without meeting `lua_to_json`.
--- @param history table  array, mutated
--- @param keep number
function M.remember(history, entry, keep)
    if type(entry) ~= "table" then return end
    table.insert(history, 1, {
        kind = entry.kind,
        id   = entry.id,
        at   = entry.at,
    })
    local limit = tonumber(keep) or 5
    while #history > limit do table.remove(history) end
end

return M
