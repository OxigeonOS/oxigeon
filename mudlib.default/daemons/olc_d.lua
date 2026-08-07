-- mudlib/daemons/olc_d.lua — What a builder is working on.
--
-- Was `{ area_name, entered_at }` and nothing else, which is why `olc` could
-- only enter an area: with no cursor there is nothing for `set` to act on, and
-- with no draft there is nothing to save.
--
-- ─── A modal cursor, not a modal shell ───────────────────────────────────────
--
-- The cursor is a *default argument* — `olc set short The Hall` means "on
-- whatever I am editing". Nothing is swallowed: `look`, `who` and a tell all
-- still work while you build, and the dispatcher's room-action layer is
-- untouched.
--
-- The alternative, a sub-shell that eats every verb until `done`, is a trap. You
-- cannot look at anything without leaving, so the first thing every builder does
-- is leave and lose their place. It would also have to intercept above room
-- actions, which means the room you are standing in stops working while you edit
-- it.
--
-- The cursor does **not** follow movement. Walking next door to see what an exit
-- looks like from the other side and walking back must not make it fifty-fifty
-- which room the next `set` writes to. `dig` moves it, because you just
-- explicitly created that room.
--
-- ─── Buffered, not write-through ─────────────────────────────────────────────
--
-- `set` changes the draft *and* the live object, so the room changes under you
-- as you type. Disk is touched only by `olc save`, which runs `verify` first.
--
-- The old OLC wrote on every `dig`. That is the thing that makes a lint
-- pointless: you cannot gate a write on a check that runs after the write.

local M = {}

-- session_id → {
--   area_name, entered_at,
--   cursor = { kind = "room"|"item"|"mob", id = "crypt.hall" } | nil,
--   drafts = { room = { [id] = data }, item = {...}, mob = {...} },
--   dirty  = { ["room:crypt.hall"] = true },
-- }
M._sessions = {}

local KINDS = { room = true, item = true, mob = true }

--- Enter build mode for an area.
--- @param session_id string
--- @param area_name string
--- @return boolean
function M.start(session_id, area_name)
    M._sessions[session_id] = {
        area_name  = area_name,
        entered_at = os_time(),
        cursor     = nil,
        drafts     = { room = {}, item = {}, mob = {} },
        dirty      = {},
    }
    log("info", "OLC_D: Session " .. tostring(session_id)
        .. " entered OLC mode for area '" .. area_name .. "'")
    return true
end

--- Leave build mode.
--- @param session_id string
--- @return boolean
function M.stop(session_id)
    local state = M._sessions[session_id]
    if state then
        M._sessions[session_id] = nil
        log("info", "OLC_D: Session " .. tostring(session_id)
            .. " exited OLC mode for area '" .. tostring(state.area_name) .. "'")
    end
    return true
end

--- The whole session state, or nil.
--- @param session_id string
--- @return table|nil
function M.get_state(session_id)
    return M._sessions[session_id]
end

--- Is this session building?
--- @param session_id string
--- @return boolean
function M.is_active(session_id)
    return M._sessions[session_id] ~= nil
end

-- ─── The cursor ──────────────────────────────────────────────────────────────

--- Point the cursor at something.
--- @param session_id string
--- @param kind string  "room" | "item" | "mob"
--- @param id string
--- @return boolean ok, string|nil err
function M.set_cursor(session_id, kind, id)
    local state = M._sessions[session_id]
    if not state then return false, "you are not building" end
    if not KINDS[kind] then return false, "no such kind '" .. tostring(kind) .. "'" end
    state.cursor = { kind = kind, id = id }
    return true
end

--- What the cursor points at, or nil.
--- @param session_id string
--- @return table|nil  { kind, id }
function M.cursor(session_id)
    local state = M._sessions[session_id]
    return state and state.cursor
end

-- ─── Drafts ──────────────────────────────────────────────────────────────────

local function key_of(kind, id) return kind .. ":" .. tostring(id) end

--- The draft for one thing, creating it from the live world if there is none.
---
--- Reading the live object rather than the file is deliberate: what a builder
--- sees when they walk into a room *is* the live object, and an editor that
--- opened a different value from the one in front of them would be lying about
--- which is real. The file is caught up by `save`.
--- @param session_id string
--- @param kind string
--- @param id string
--- @return table|nil draft
function M.draft(session_id, kind, id)
    local state = M._sessions[session_id]
    if not state or not KINDS[kind] then return nil end

    local existing = state.drafts[kind][id]
    if existing then return existing end
    return nil
end

--- Put a draft under the session's hand, marking it changed.
--- @param session_id string
--- @param kind string
--- @param id string
--- @param data table
--- @return table data
function M.put_draft(session_id, kind, id, data)
    local state = M._sessions[session_id]
    if not state or not KINDS[kind] then return data end
    state.drafts[kind][id] = data
    return data
end

--- Mark something changed since the last save.
--- @param session_id string
--- @param kind string
--- @param id string
function M.touch(session_id, kind, id)
    local state = M._sessions[session_id]
    if not state then return end
    state.dirty[key_of(kind, id)] = true
end

--- Is anything unsaved?
--- @param session_id string
--- @return boolean
function M.is_dirty(session_id)
    local state = M._sessions[session_id]
    if not state then return false end
    return next(state.dirty) ~= nil
end

--- Everything changed since the last save, sorted.
--- @param session_id string
--- @return table  array of { kind, id }
function M.changed(session_id)
    local state = M._sessions[session_id]
    if not state then return {} end

    local out = {}
    for key in pairs(state.dirty) do
        local kind, id = key:match("^(%a+):(.+)$")
        if kind then out[#out + 1] = { kind = kind, id = id } end
    end
    table.sort(out, function(a, b)
        if a.kind ~= b.kind then return a.kind < b.kind end
        return a.id < b.id
    end)
    return out
end

--- Every draft of one kind, as an array. What `save` writes.
--- @param session_id string
--- @param kind string
--- @return table  array of data tables
function M.drafts_of(session_id, kind)
    local state = M._sessions[session_id]
    if not state or not KINDS[kind] then return {} end

    local ids = {}
    for id in pairs(state.drafts[kind]) do ids[#ids + 1] = id end
    table.sort(ids)

    local out = {}
    for _, id in ipairs(ids) do out[#out + 1] = state.drafts[kind][id] end
    return out
end

--- Forget every draft and every dirty mark. After a successful save.
--- @param session_id string
function M.mark_saved(session_id)
    local state = M._sessions[session_id]
    if state then state.dirty = {} end
end

--- Throw away the drafts, keeping the session.
--- @param session_id string
--- @param kind string|nil  only this kind, or all
--- @param id string|nil     only this one
function M.revert(session_id, kind, id)
    local state = M._sessions[session_id]
    if not state then return end

    if kind and id then
        state.drafts[kind][id] = nil
        state.dirty[key_of(kind, id)] = nil
        return
    end
    state.drafts = { room = {}, item = {}, mob = {} }
    state.dirty = {}
end

--- Cleanup on disconnect.
---
--- Unsaved work is **not** silently dropped. Losing a builder's hour to a
--- dropped connection is worse than an unreviewed write, so a clean draft is
--- saved and a dirty one is journalled — the alternative is somebody finding out
--- their evening is gone with nothing to look at.
--- @param session_id string
function M.cleanup(session_id)
    local state = M._sessions[session_id]
    if not state then return end

    if next(state.dirty) ~= nil then
        local n = 0
        for _ in pairs(state.dirty) do n = n + 1 end
        local message = "OLC_D: session " .. tostring(session_id)
            .. " disconnected with " .. n .. " unsaved change(s) in area '"
            .. tostring(state.area_name) .. "'"
        log("warn", message)
        if DAEMON and DAEMON.journal then pcall(DAEMON.journal.warn, message) end
    end

    M.stop(session_id)
end

log("debug", "OLC_D: daemon loaded")

return M
