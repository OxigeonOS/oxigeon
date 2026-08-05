-- mudlib/lib/effects.lua — The event pipeline's ordering and arithmetic.
--
-- An event runs through every handler an entity's effects contribute, and the
-- answer depends on the order they run in. "Take 15% less damage" and "negate
-- 5 damage" applied to a 30-point hit give 20 one way round and 21 the other,
-- and nobody should have to know which effect was registered first to predict
-- which they get.
--
-- So order is a property of the *phase* a handler declares, not of
-- registration:
--
--   pre     immunity, validation, outright cancellation
--   add     flat additions to the base amount
--   mult    scaling — handlers add to ev.scale, they never touch ev.amount
--   ─────   the fold: amount = amount * (1 + scale), exactly once
--   reduce  flat reductions, applied after scaling (armor, negation)
--   clamp   floors and ceilings
--   ─────   ev.min / ev.max applied
--   post    observation and side effects
--
-- Multipliers accumulate into `ev.scale` and fold once, so two +20% buffs are
-- +40% rather than +44%, and the mult phase is genuinely order-independent.
-- A game that wants diminishing returns instead changes `fold` and nothing
-- else.
--
-- Exposes:
--   effects.PHASES, effects.phase_rank(name), effects.valid_phase(name)
--   effects.sort_handlers(list)
--   effects.fold(ev)
--   effects.dispatch(ev, handlers, on_error) -> ev

local M = {}

M.PHASES = {
    pre    = 10,
    add    = 20,
    mult   = 30,
    reduce = 40,
    clamp  = 50,
    post   = 60,
}

M.DEFAULT_PHASE = "add"

function M.valid_phase(name)
    return M.PHASES[name] ~= nil
end

function M.phase_rank(name)
    return M.PHASES[name] or M.PHASES[M.DEFAULT_PHASE]
end

--- Sort handlers into the order they must run in.
---
--- The key is (phase, order, def, index) and every part of it is needed: phase
--- decides the arithmetic, `order` lets one definition sequence its own
--- handlers within a phase, and def+index break remaining ties deterministically
--- so the same set of effects always produces the same number. Nothing here
--- iterates a table with `pairs`.
--- @param handlers table  array of { phase, order, def, index, fn, ctx }
--- @return table  the same array, sorted in place
function M.sort_handlers(handlers)
    table.sort(handlers, function(a, b)
        local ra, rb = M.phase_rank(a.phase), M.phase_rank(b.phase)
        if ra ~= rb then return ra < rb end
        local oa, ob = a.order or 0, b.order or 0
        if oa ~= ob then return oa < ob end
        if a.def ~= b.def then return tostring(a.def) < tostring(b.def) end
        return (a.index or 0) < (b.index or 0)
    end)
    return handlers
end

--- Apply the accumulated multiplier, once.
function M.fold(ev)
    if type(ev.amount) == "number" and type(ev.scale) == "number" and ev.scale ~= 0 then
        ev.amount = ev.amount * (1 + ev.scale)
    end
    ev.scale = 0
    return ev
end

local function apply_bounds(ev)
    if type(ev.amount) ~= "number" then return end
    if ev.min and ev.amount < ev.min then ev.amount = ev.min end
    if ev.max and ev.amount > ev.max then ev.amount = ev.max end
end

--- Run `handlers` over `ev`, in phase order, and return `ev`.
---
--- Every handler is called inside a `pcall`: one effect written by one area
--- author must not be able to break combat for everyone. A handler that raises
--- is reported through `on_error` and skipped, and the pipeline continues with
--- whatever it had already computed.
---
--- Cancellation is checked after each handler, so a `pre` handler that sets
--- `ev.cancelled` stops the rest from running.
--- @param ev table
--- @param handlers table  from sort_handlers, or unsorted (sorted here)
--- @param on_error function|nil  function(err, handler)
--- @return table ev  the same table that was passed in
function M.dispatch(ev, handlers, on_error)
    if not handlers or #handlers == 0 then
        -- Nothing to run. Deliberately does not fold or clamp: an event with no
        -- handlers must come out exactly as it went in, and this is the path
        -- almost every event in the game takes.
        return ev
    end

    M.sort_handlers(handlers)
    ev.scale = ev.scale or 0

    local folded, bounded = false, false
    for _, h in ipairs(handlers) do
        if ev.cancelled then break end
        local rank = M.phase_rank(h.phase)
        if not folded and rank > M.PHASES.mult then
            M.fold(ev); folded = true
        end
        if not bounded and rank > M.PHASES.clamp then
            apply_bounds(ev); bounded = true
        end
        local ok, err = pcall(h.fn, ev, h.ctx)
        if not ok and on_error then on_error(err, h) end
    end

    if not folded then M.fold(ev) end
    if not bounded then apply_bounds(ev) end
    return ev
end

-- ─── Writing definitions ─────────────────────────────────────────────────────

--- An `on_apply` / `on_expire` that just tells the entity something.
---
--- Nearly every effect in the game had two of these, and every one of them was
--- the same three lines around a single string:
---
---     on_apply = function(ctx)
---         if ctx.entity.send then ctx.entity:send("Your thoughts sharpen.") end
---     end,
---
--- Sixteen copies of that is sixteen chances to forget the `send` guard — a mob
--- has no `send`, and effects land on mobs. Hoisting each to a named local
--- would have produced sixteen names for one idea, so this is the shape worth
--- having instead:
---
---     on_apply  = Effects.says("{cyan}Your thoughts sharpen.{/}"),
---     on_expire = Effects.says("{cyan}Your thoughts dull again.{/}"),
---
--- @param message string
--- @return function  suitable for on_apply, on_expire, or on_stack
function M.says(message)
    return function(ctx)
        local entity = ctx and ctx.entity
        if entity and entity.send then entity:send(message) end
    end
end

return M
