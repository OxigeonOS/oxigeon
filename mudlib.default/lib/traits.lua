-- mudlib/lib/traits.lua — The arithmetic behind TRAIT_D, with no daemons in it.
--
-- Everything here is a pure function of its arguments: no DAEMON, no efuns, no
-- clock of its own. That is deliberate. The dependency ordering and the
-- regeneration settle are the two places this system can be subtly,
-- silently wrong, and pure functions are the only kind you can test
-- exhaustively with a table of inputs and expected outputs.
--
-- Exposes:
--   traits.topo_sort(defs)      -> order[] | nil, cycle_path[]
--   traits.settle(...)          -> new_cur, new_anchor | nil, nil
--   traits.clamp(v, min, max)   -> number
--   traits.round(v, mode)       -> number

local M = {}

-- ─── Dependency ordering ─────────────────────────────────────────────────────

--- Order trait ids so every trait comes after everything it depends on.
---
--- Returns the order, or `nil` plus the cycle as a path — `{"willpower",
--- "wisdom", "insight", "willpower"}` — because "there is a cycle somewhere in
--- your 30 traits" is not a message anyone can act on.
---
--- Unknown dependencies are not an error here; `seal()` reports those
--- separately with better context. They are simply skipped, so one typo does
--- not also produce a misleading cycle.
--- @param defs table  id -> { depends = { id, ... } }
--- @return table|nil order   array of ids, dependencies first
--- @return table|nil cycle   the offending path, when order is nil
function M.topo_sort(defs)
    local order, state = {}, {}   -- state: nil = unvisited, 1 = visiting, 2 = done
    local path = {}               -- the current DFS stack, for the cycle report

    local function visit(id)
        if state[id] == 2 then return true end
        if state[id] == 1 then
            -- Found the back edge. Trim the stack to where this id first
            -- appears so the reported path is the cycle itself, not the whole
            -- route we took to reach it.
            local cycle, seen = {}, false
            for _, step in ipairs(path) do
                if step == id then seen = true end
                if seen then cycle[#cycle + 1] = step end
            end
            cycle[#cycle + 1] = id
            return false, cycle
        end

        state[id] = 1
        path[#path + 1] = id
        local def = defs[id]
        if def and def.depends then
            for _, dep in ipairs(def.depends) do
                if defs[dep] then
                    local ok, cycle = visit(dep)
                    if not ok then return false, cycle end
                end
            end
        end
        path[#path] = nil
        state[id] = 2
        order[#order + 1] = id
        return true
    end

    -- Sorted, so the order is stable across runs rather than depending on
    -- `pairs`. A test that asserts an order has to be able to rely on it.
    local ids = {}
    for id in pairs(defs) do ids[#ids + 1] = id end
    table.sort(ids)

    for _, id in ipairs(ids) do
        local ok, cycle = visit(id)
        if not ok then return nil, cycle end
    end
    return order
end

-- ─── Bounds and rounding ─────────────────────────────────────────────────────

--- Clamp into [min, max]. Either bound may be nil, meaning unbounded.
function M.clamp(v, min, max)
    if min and v < min then v = min end
    if max and v > max then v = max end
    return v
end

--- @param mode string|nil  "floor" (default) | "ceil" | "round" | "none"
function M.round(v, mode)
    if mode == "none" then return v end
    if mode == "ceil" then return math.ceil(v) end
    if mode == "round" then return math.floor(v + 0.5) end
    return math.floor(v)
end

-- ─── Regeneration ────────────────────────────────────────────────────────────

--- Work out what a regenerating gauge should hold now, given when it was last
--- settled.
---
--- The whole design rests on one idea: **the anchor is not advanced past the
--- seconds actually spent.** At 1 point per 3 seconds, 10 seconds of elapsed
--- time earns 3 points and consumes 9 — the tenth second stays in the anchor
--- and counts toward the next point. So the value never stores a fraction, and
--- no time is ever lost to rounding, however often this is called.
---
--- Two returns of `nil` mean *nothing changed*, and the caller must not write.
--- That matters more than it looks: the prompt settles every gauge on every
--- command, so a settle that always reported a change would dirty every online
--- player's state several times a second and defeat write-behind entirely.
---
--- @param cur number     the stored current value
--- @param anchor number  when it was last settled (unix seconds)
--- @param now number     unix seconds
--- @param rate number    units gained...
--- @param per number     ...per this many seconds
--- @param target number  the value regeneration moves toward
--- @param min number|nil
--- @param max number|nil
--- @return number|nil new_cur
--- @return number|nil new_anchor
function M.settle(cur, anchor, now, rate, per, target, min, max)
    if not rate or rate <= 0 or not per or per <= 0 then return nil, nil end
    if not anchor then return cur, now end          -- never settled: anchor it
    if now < anchor then return cur, now end        -- clock stepped back; re-anchor, no gain
    if cur == target then return nil, nil end       -- at rest, and *not* a write

    local elapsed = now - anchor
    local units   = math.floor(elapsed * rate / per)
    if units < 1 then return nil, nil end           -- nothing earned yet

    local sign     = (target > cur) and 1 or -1
    local consumed = math.floor(units * per / rate)
    local new_cur  = cur + sign * units

    -- Reaching the target re-anchors to now. Carrying the remainder past the
    -- target would let a player at full health bank credit while idle and dump
    -- it the instant they were hit.
    if (sign > 0 and new_cur >= target) or (sign < 0 and new_cur <= target) then
        return M.clamp(target, min, max), now
    end
    return M.clamp(new_cur, min, max), anchor + consumed
end

return M
