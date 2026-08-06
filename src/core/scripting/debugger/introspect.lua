-- Frame introspection for the debug adapter.
--
-- Loaded once at startup with the real `debug` table as its only argument. That
-- table is stashed in the Lua registry and removed from _G, so this closure is
-- the only thing in the process that can reach it.
--
-- Marshalling lives here rather than in Rust because `debug.getlocal`'s level
-- arithmetic is far easier to get right (and to test) from Lua, and because
-- `tostring`, metatables and `pcall` are all free on this side.

local dbg = ...

local H = {}
local handles, hseq = {}, 0

-- Expanding a Player reaches the whole daemon graph, so everything is capped.
local MAX_CHILDREN = 200
local MAX_STR = 256

--- Level arithmetic, as measured by tests/debug_hook_spike.rs:
---   level 0 = the `debug.*` C function itself
---   level 1 = the H.* function that called it
---   level 2 = the paused Lua frame  <-- DAP frame 0
--- Rust invokes every H.* function directly from the hook callback, which is
--- not a Lua stack frame, so this offset is the same for all of them.
local LEVEL_BASE = 2

local function trunc(s)
    if #s > MAX_STR then return s:sub(1, MAX_STR - 1) .. "..." end
    return s
end

-- How many entries a collapsed table preview shows before eliding.
local PREVIEW_ITEMS = 5
local PREVIEW_STR = 24

--- One entry inside a table preview. Nested tables collapse to `{...}` rather
--- than recursing — the pane is for scanning, and children are one click away.
local function inline(v)
    local t = type(v)
    if t == "string" then
        local s = string.format("%q", v)
        if #s > PREVIEW_STR then s = s:sub(1, PREVIEW_STR - 4) .. '..."' end
        return s
    elseif t == "table" then
        return "{...}"
    elseif t == "function" or t == "userdata" or t == "thread" then
        return "<" .. t .. ">"
    end
    return tostring(v)
end

--- Summarise a table's contents instead of printing its address.
---
--- `table: 0x025d651ea7b0` tells you nothing; `{name = "varuser", hp = 100, ...}
--- (28)` tells you what you are looking at without expanding it.
local function preview_table(t)
    -- A mudlib object that defines __tostring knows how it wants to be shown.
    local mt = getmetatable(t)
    if mt and rawget(mt, "__tostring") then
        local ok, s = pcall(tostring, t)
        if ok and type(s) == "string" then return trunc(s) end
    end

    local n = #t
    local parts, shown, total = {}, 0, 0

    for i = 1, n do
        total = total + 1
        if shown < PREVIEW_ITEMS then
            parts[#parts + 1] = inline(t[i])
            shown = shown + 1
        end
    end

    -- Hash part, sorted so the preview is stable between stops.
    local keys = {}
    for k in pairs(t) do
        if not (type(k) == "number" and k >= 1 and k <= n and k % 1 == 0) then
            keys[#keys + 1] = k
        end
    end
    total = total + #keys
    table.sort(keys, function(a, b) return tostring(a) < tostring(b) end)
    for _, k in ipairs(keys) do
        if shown >= PREVIEW_ITEMS then break end
        parts[#parts + 1] = tostring(k) .. " = " .. inline(t[k])
        shown = shown + 1
    end

    if total == 0 then return "{}" end
    local body = table.concat(parts, ", ")
    if shown < total then body = body .. ", ..." end
    local open, close = "{", "}"
    if n == total then open, close = "[", "]" end   -- pure sequence
    return trunc(open .. body .. close .. "  (" .. total .. ")")
end

--- Render a value for display, never raising: a __tostring metamethod on a
--- game object is mudlib code and can fail.
local function describe(v)
    local t = type(v)
    if t == "string" then
        return trunc(string.format("%q", v)), t
    elseif t == "table" then
        local ok, s = pcall(preview_table, v)
        return (ok and s or "<table: preview failed>"), t
    elseif t == "userdata" or t == "function" or t == "thread" then
        local ok, s = pcall(tostring, v)
        return trunc(ok and s or ("<" .. t .. ": __tostring failed>")), t
    end
    return tostring(v), t
end

local function alloc(spec)
    hseq = hseq + 1
    handles[hseq] = spec
    return hseq
end

--- Only tables get a child reference. Following __index would walk into the
--- daemon graph and never stop.
local function ref_for(v)
    if type(v) == "table" then return alloc({ kind = "table", value = v }) end
    return 0
end

local function entry(name, v)
    local text, t = describe(v)
    return { name = name, value = text, type = t, ref = ref_for(v) }
end

--- Internal slots are named like "(*temporary)" and "(for index)"; they are
--- noise in a variables pane.
local function is_internal(name)
    return name:sub(1, 1) == "("
end

--- Drop every handle. Called on resume — references must not outlive the stop
--- that created them, or the pane would show values from a dead frame.
function H.reset()
    handles, hseq = {}, 0
end

--- Scopes for a DAP frame index.
function H.scopes(frame)
    local level = frame + LEVEL_BASE
    local info = dbg.getinfo(level, "fS")
    if not info then return {} end

    local out = {}
    out[#out + 1] = { name = "Locals", ref = alloc({ kind = "locals", level = level }), expensive = false }
    if info.func and dbg.getupvalue(info.func, 1) then
        out[#out + 1] = { name = "Upvalues", ref = alloc({ kind = "upvalues", func = info.func }), expensive = false }
    end
    out[#out + 1] = { name = "Globals", ref = alloc({ kind = "globals" }), expensive = true }
    return out
end

--- Expand a reference into its children.
function H.expand(ref)
    local spec = handles[ref]
    if not spec then return {} end
    local out = {}

    if spec.kind == "locals" then
        local i = 1
        while #out < MAX_CHILDREN do
            local name, value = dbg.getlocal(spec.level, i)
            if not name then break end
            -- Later declarations shadow earlier ones; keep the last.
            if not is_internal(name) then
                local replaced = false
                for k = 1, #out do
                    if out[k].name == name then out[k] = entry(name, value); replaced = true; break end
                end
                if not replaced then out[#out + 1] = entry(name, value) end
            end
            i = i + 1
        end

    elseif spec.kind == "upvalues" then
        local i = 1
        while #out < MAX_CHILDREN do
            local name, value = dbg.getupvalue(spec.func, i)
            if not name then break end
            if not is_internal(name) then out[#out + 1] = entry(name, value) end
            i = i + 1
        end

    elseif spec.kind == "globals" then
        for k, v in pairs(_G) do
            if #out >= MAX_CHILDREN then break end
            if type(k) == "string" then out[#out + 1] = entry(k, v) end
        end
        table.sort(out, function(a, b) return a.name < b.name end)

    elseif spec.kind == "frozen" then
        -- Rows captured at the moment of the stop. The stack they came from may
        -- be suspended — or gone — by the time anyone asks, so there is nothing
        -- to walk: the values are already here.
        for _, row in ipairs(spec.rows) do
            if #out >= MAX_CHILDREN then break end
            out[#out + 1] = entry(row.name, row.value)
        end

    elseif spec.kind == "table" then
        local t = spec.value
        for k, v in pairs(t) do
            if #out >= MAX_CHILDREN then break end
            out[#out + 1] = entry(tostring(k), v)
        end
        table.sort(out, function(a, b) return a.name < b.name end)
        local mt = getmetatable(t)
        if mt and #out < MAX_CHILDREN then out[#out + 1] = entry("(metatable)", mt) end
    end

    return out
end

--- Build an environment that reads a frame's locals and upvalues.
---
--- The snapshot is eager rather than a live __index proxy over
--- `debug.getlocal`: a metamethod runs at a different stack depth than the
--- function that captured the level, so a live lookup would resolve against the
--- wrong frame. Reads are exact; writes are refused.
---
--- @param level integer  absolute level **as seen from inside this function**.
---   Callers must add 1 for this function's own frame — `debug.getlocal` counts
---   from whoever calls it, so nesting shifts every level.
local function frame_env(level)
    -- Values live in a side table, leaving the environment itself empty.
    -- Storing them directly would defeat __newindex, which only fires for
    -- absent keys — assigning to a captured local would then silently succeed,
    -- which is precisely the case worth refusing.
    local snap, has = {}, {}
    local function capture(name, value)
        if not is_internal(name) then
            snap[name], has[name] = value, true   -- later declarations shadow earlier
        end
    end

    local info = dbg.getinfo(level, "f")
    if info and info.func then
        local i = 1
        while true do
            local name, value = dbg.getupvalue(info.func, i)
            if not name then break end
            capture(name, value)
            i = i + 1
        end
    end

    local i = 1
    while true do
        local name, value = dbg.getlocal(level, i)
        if not name then break end
        capture(name, value)
        i = i + 1
    end

    -- `has` is tracked separately from `snap` so a local whose value really is
    -- nil still shadows a global of the same name.
    return setmetatable({}, {
        __index = function(_, k)
            if has[k] then return snap[k] end
            return _G[k]
        end,
        __newindex = function() error("assignment is not supported in evaluate", 0) end,
    })
end

--- Compile `src` with `env` as its environment, on whichever Lua this is.
---
--- 5.1 (LuaJIT) has `loadstring` + `setfenv`; 5.2 removed both and folded the
--- environment into `load`'s fourth argument. `frame_env` already returns a
--- proxy table, which is exactly what that argument wants, so the two paths
--- differ only in spelling.
---
--- `"t"` refuses precompiled bytecode, matching the text-only loaders the
--- sandbox installs — an evaluator that accepted a binary chunk would be a way
--- around them.
local compile
if loadstring then
    compile = function(src, env)
        local chunk, err = loadstring(src, "=eval")
        if not chunk then return nil, err end
        setfenv(chunk, env)
        return chunk
    end
else
    compile = function(src, env)
        return load(src, "=eval", "t", env)
    end
end

--- Compile and run `src` against `env`. Touches no debug.* API, so its own
--- stack depth is irrelevant and it is safe to call from any nesting.
local function exec(env, src)
    -- Try as an expression first so `player.name` works, then as a statement.
    local chunk, err = compile("return " .. src, env)
    if not chunk then chunk, err = compile(src, env) end
    if not chunk then return false, err end
    return pcall(chunk)
end

--- Point at the `.`-versus-`:` slip, when that is plainly what happened.
---
--- `player.is_alive()` calls a method with no `self`, and Lua reports it from
--- inside the callee: "mobile.lua:114: attempt to index a nil value (local
--- 'self')" — a file and a line that have nothing to do with what you typed.
--- Everything else here reports Lua's error verbatim, and this is the one case
--- where verbatim sends you to the wrong place.
---
--- Only fires when the expression really does contain `.name(`, so it cannot
--- misread an unrelated nil `self`.
local function self_hint(src, err)
    if not err:find("local 'self'", 1, true) then return err end
    local dotted = src:match("[%w_%]%)]%.([%w_]+)%s*%(")
    if not dotted then return err end
    return err .. "  (did you mean :" .. dotted .. "() ? a `.` call passes no self)"
end

--- Evaluate an expression as if it ran inside a frame.
function H.eval(frame, src)
    -- +1 for frame_env's own frame; see its contract.
    local env = frame_env(frame + LEVEL_BASE + 1)
    local ok, result = exec(env, src)
    if not ok then return false, self_hint(src, tostring(result)), "error", 0 end

    local text, t = describe(result)
    return true, text, t, ref_for(result)
end

--- Evaluate a breakpoint condition.
---
--- Returns `ok, truthy, err`. A condition that raises is reported rather than
--- swallowed: silently never stopping is indistinguishable from a broken
--- breakpoint, and silently always stopping is just as confusing.
function H.cond(frame, src)
    local env = frame_env(frame + LEVEL_BASE + 1)
    local ok, result = exec(env, src)
    if not ok then return false, false, self_hint(src, tostring(result)) end
    -- Lua truthiness: only nil and false are falsy.
    return true, (result and true or false), nil
end

-- ─── Capturing a stop, for a VM that will not still be there ─────────────────
--
-- Everything above resolves against the *live* stack: a handle holds a level,
-- and `debug.getlocal(level, i)` walks the thread that is running now. That is
-- correct while the hook blocks the thread, because nothing else can run.
--
-- Once a stop is a coroutine *yield*, it stops being correct. The engine parks
-- the suspended thread and carries on serving other players, so by the time the
-- debug client asks what a local holds, the stack those levels described is
-- suspended and the running thread is somebody else's command.
--
-- So a stop is captured at the moment it happens, from inside the hook, while
-- the frames are still the current ones. What comes out has no stack in it:
-- frozen rows for the panes, and the same eager environment `frame_env` already
-- built for evaluation. Answering afterwards touches no `debug.*` at all.

local captures, cseq = {}, 0

--- Freeze one frame's locals and upvalues into ordered rows.
local function freeze(level)
    local locals, upvals = {}, {}

    local info = dbg.getinfo(level, "fS")
    if not info then return nil end

    if info.func then
        local i = 1
        while true do
            local name, value = dbg.getupvalue(info.func, i)
            if not name then break end
            if not is_internal(name) then
                upvals[#upvals + 1] = { name = name, value = value }
            end
            i = i + 1
        end
    end

    local i = 1
    while true do
        local name, value = dbg.getlocal(level, i)
        if not name then break end
        if not is_internal(name) then
            -- Later declarations shadow earlier ones, as in `H.expand`.
            local replaced = false
            for k = 1, #locals do
                if locals[k].name == name then locals[k] = { name = name, value = value }; replaced = true; break end
            end
            if not replaced then locals[#locals + 1] = { name = name, value = value } end
        end
        i = i + 1
    end

    return { locals = locals, upvals = upvals, has_upvals = #upvals > 0 }
end

--- Capture the paused frames. Call only from the hook, before yielding.
--- @param levels integer  how many frames to keep
--- @return integer  capture id, or 0 if there was nothing to capture
function H.capture(levels)
    local frames = {}
    for frame = 0, math.max(0, levels - 1) do
        -- +1 for this function's own frame, as `frame_env` documents.
        local level = frame + LEVEL_BASE + 1
        local frozen = freeze(level)
        if not frozen then break end
        frozen.env = frame_env(level)
        frames[frame] = frozen
    end
    if not frames[0] then return 0 end

    cseq = cseq + 1
    captures[cseq] = frames
    return cseq
end

--- Drop a capture. Called on resume, with the handles it allocated.
function H.release(cap)
    captures[cap] = nil
end

--- Scopes for a frame of a captured stop. Same shape as `H.scopes`, but the
--- handles hold values rather than stack levels.
function H.cap_scopes(cap, frame)
    local frames = captures[cap]
    local f = frames and frames[frame]
    if not f then return {} end

    local out = {}
    out[#out + 1] = { name = "Locals", ref = alloc({ kind = "frozen", rows = f.locals }), expensive = false }
    if f.has_upvals then
        out[#out + 1] = { name = "Upvalues", ref = alloc({ kind = "frozen", rows = f.upvals }), expensive = false }
    end
    -- Globals are live by nature and shared, so there is nothing to freeze.
    out[#out + 1] = { name = "Globals", ref = alloc({ kind = "globals" }), expensive = true }
    return out
end

--- Evaluate against a captured frame's environment.
function H.cap_eval(cap, frame, src)
    local frames = captures[cap]
    local f = frames and frames[frame]
    if not f then return false, "that frame is no longer available", "error", 0 end

    local ok, result = exec(f.env, src)
    if not ok then return false, tostring(result), "error", 0 end

    local text, t = describe(result)
    return true, text, t, ref_for(result)
end

return H
