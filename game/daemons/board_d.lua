-- game/daemons/board_d.lua — A notice board, and the first real consumer of
-- the document store's query half.
--
-- A 906-line document store shipped with twelve `db_*` efuns and only three of
-- them had a caller anywhere in `mudlib/` or `game/` — `db_get`, `db_put` and
-- `db_delete`, all from `cache_d`. The entire query and atomic-merge half had
-- never been used by game code at all.
--
-- A notice board is the smallest thing that genuinely needs it:
--
--   db_insert   posting, with a generated id
--   db_find     listing and searching, with every filter operator
--   db_count    "3 notices" without materialising three notices
--   db_get      reading one
--   db_incr     view counts, atomically — two people reading at once must not
--               lose a count to a read-modify-write race
--   db_update   editing, as a recursive merge rather than a rewrite
--   db_unset    withdrawing a field
--   db_delete   removing a notice
--   db_exists   "is that still there" without deserialising it
--
-- Game layer, not mudlib: a board is content. What is on it, who may post, how
-- long a notice lives and what the categories are are all this game's decisions.
--
-- ─── Why the document store rather than the state cache ─────────────────────
--
-- The state cache is for state a *subsystem* owns and rewrites — effects,
-- cooldowns, counters. A notice is a document: written once, read many times by
-- people who are not its author, and queried by fields it does not own. The
-- cache would make that a scan over every scope; the store makes it a filter.

local M = {}

local NOTICES = "board_notices"

--- Categories this board accepts. Freeform would let one typo make a category
--- nobody can find again, and a board is small enough that a closed list costs
--- nothing.
M.CATEGORIES = { "news", "trade", "help", "rp" }

--- How long a notice lives, in seconds. Two weeks: long enough to be worth
--- posting, short enough that the board is not an archive.
local LIFETIME = 14 * 24 * 3600

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

local function available()
    return type(db_insert) == "function" and type(db_find) == "function"
end

--- The document out of its envelope.
---
--- `db_get` and `db_find` return a *record* — `{ id, collection, created_at,
--- data }` — and the extra `.data` is what keeps a document's own fields from
--- colliding with the store's. Every caller here wants the notice rather than
--- the envelope, so the unwrapping happens once, at the boundary, with `id`
--- folded in because that is the one part of the envelope anyone needs.
--- @param rec table|nil
--- @return table|nil
local function unwrap(rec)
    if type(rec) ~= "table" then return nil end
    local doc = rec.data
    if type(doc) ~= "table" then return nil end
    doc.id = rec.id
    return doc
end

local function unwrap_all(rows)
    local out = {}
    for i, rec in ipairs(rows or {}) do out[i] = unwrap(rec) end
    return out
end

--- @param category string|nil
--- @return boolean
function M.is_category(category)
    if type(category) ~= "string" then return false end
    for _, c in ipairs(M.CATEGORIES) do
        if c == category:lower() then return true end
    end
    return false
end

-- ─── Posting ─────────────────────────────────────────────────────────────────

--- Put a notice up.
--- @param player table
--- @param category string
--- @param subject string
--- @param body string
--- @return string|nil id, string|nil why
function M.post(player, category, subject, body)
    if not available() then return nil, "The board is not working." end
    if not M.is_category(category) then
        return nil, "Categories are: " .. table.concat(M.CATEGORIES, ", ")
    end
    subject = (subject or ""):gsub("^%s+", ""):gsub("%s+$", "")
    body = (body or ""):gsub("^%s+", ""):gsub("%s+$", "")
    if #subject == 0 then return nil, "A notice needs a subject." end
    if #body == 0 then return nil, "A notice needs something in it." end
    -- The store refuses a document over `documents.max_bytes` by name, which is
    -- the right place for the hard limit. This is the polite one, so a player
    -- pasting an essay gets a sentence rather than a raised error.
    if #body > 4000 then return nil, "That is too long for a notice." end

    local now = os_time()
    local ok, id = pcall(db_insert, NOTICES, {
        category = category:lower(),
        subject  = subject,
        body     = body,
        author   = player.name,
        char_id  = player.char_id,
        posted   = now,
        expires  = now + LIFETIME,
        views    = 0,
    })
    if not ok then
        log_error("BOARD_D: could not post a notice: " .. tostring(id))
        return nil, "The notice would not stick."
    end

    if DAEMON and DAEMON.event then
        pcall(DAEMON.event.emit, "board.posted", {
            id = id, char_id = player.char_id, category = category:lower(),
        })
    end
    return id, nil
end

-- ─── Reading ─────────────────────────────────────────────────────────────────

--- The notices, newest first.
---
--- `expires` is filtered with `>` rather than by a sweep task: a notice that has
--- run out should stop being listed the moment it does, and a sweep would make
--- that "within a few minutes of when it does". Expired rows are collected by
--- `M.sweep`, which is housekeeping rather than correctness.
--- @param category string|nil
--- @param opts table|nil  { limit, offset }
--- @return table  array of records
function M.list(category, opts)
    if not available() then return {} end
    opts = opts or {}

    local filter = { expires = { [">"] = os_time() } }
    if M.is_category(category) then filter.category = category:lower() end

    local ok, rows = pcall(db_find, NOTICES, filter, {
        sort = "posted", order = "desc",
        limit = opts.limit or 20, offset = opts.offset or 0,
    })
    if not ok then
        log_error("BOARD_D: could not list notices: " .. tostring(rows))
        return {}
    end
    return unwrap_all(rows)
end

--- How many live notices there are, without materialising any of them.
--- @return number
function M.count(category)
    if type(db_count) ~= "function" then return 0 end
    local filter = { expires = { [">"] = os_time() } }
    if M.is_category(category) then filter.category = category:lower() end
    local ok, n = pcall(db_count, NOTICES, filter)
    return ok and n or 0
end

--- Resolve a possibly-abbreviated id to a whole one.
---
--- Ids are uuids, and asking somebody to retype thirty-six characters to read a
--- notice is asking them not to. The listing shows the first eight; this takes
--- either that or the whole thing.
---
--- An **ambiguous** prefix is refused rather than guessed. Two notices whose
--- ids share a prefix is unlikely and picking one of them silently is the kind
--- of wrong that only shows up as somebody deleting the wrong notice.
--- @param prefix string
--- @return string|nil id, string|nil why
function M.resolve_id(prefix)
    if type(prefix) ~= "string" or #prefix == 0 then return nil, "Which notice?" end
    if not available() then return nil, "The board is not working." end

    -- The whole thing, most of the time.
    if type(db_exists) == "function" then
        local ok, there = pcall(db_exists, NOTICES, prefix)
        if ok and there then return prefix end
    end

    -- Otherwise scan. The board is bounded by its own sweep, and this only
    -- happens when somebody typed an abbreviation.
    local ok, rows = pcall(db_find, NOTICES, {}, { limit = 500 })
    if not ok then return nil, "The board is not working." end

    local found
    for _, rec in ipairs(rows) do
        if type(rec.id) == "string" and rec.id:sub(1, #prefix) == prefix then
            if found then return nil, "That could be more than one notice." end
            found = rec.id
        end
    end
    if not found then return nil, "There is no such notice." end
    return found
end

--- One notice, and a view counted for it.
---
--- `db_incr` rather than read-modify-write: two people opening the same notice
--- in the same tick must not lose a count, and this is the operation that
--- exists so the game thread does not need a transaction to say so.
--- @param id string
--- @return table|nil
function M.read(id)
    if type(db_get) ~= "function" then return nil end
    local ok, rec = pcall(db_get, NOTICES, id)
    local doc = ok and unwrap(rec)
    if not doc then return nil end

    if type(db_incr) == "function" then
        pcall(db_incr, NOTICES, id, "views", 1)
        -- The stored count is now one higher than the copy just read, and the
        -- reader should see the view they just made rather than the one before.
        doc.views = (doc.views or 0) + 1
    end
    return doc
end

--- Search every notice's subject and body.
---
--- Two queries and a merge rather than one: the filter language has no `or`,
--- deliberately — an operator set that grows an expression tree stops being a
--- filter and starts being a query language nobody asked for. Two `like`s and a
--- dedupe is honest about the cost.
--- @param text string
--- @return table  array of records
function M.search(text)
    if not available() or type(text) ~= "string" or #text == 0 then return {} end
    local pattern = "%" .. text .. "%"
    local live = { [">"] = os_time() }

    local seen, out = {}, {}
    for _, field in ipairs({ "subject", "body" }) do
        local ok, rows = pcall(db_find, NOTICES,
            { [field] = { like = pattern }, expires = live },
            { sort = "posted", order = "desc", limit = 50 })
        if ok then
            for _, row in ipairs(unwrap_all(rows)) do
                if row and not seen[row.id] then
                    seen[row.id] = true
                    out[#out + 1] = row
                end
            end
        end
    end

    table.sort(out, function(a, b) return (a.posted or 0) > (b.posted or 0) end)
    return out
end

--- Everything one person posted. `in` rather than a scan, so a leaderboard-style
--- question over several authors is one query.
--- @param names table  array of author names
--- @return table
function M.by_authors(names)
    if not available() or type(names) ~= "table" or #names == 0 then return {} end
    local ok, rows = pcall(db_find, NOTICES,
        { author = { ["in"] = names }, expires = { [">"] = os_time() } },
        { sort = "posted", order = "desc", limit = 50 })
    return ok and unwrap_all(rows) or {}
end

-- ─── Editing and removing ────────────────────────────────────────────────────

--- Edit a notice you wrote.
---
--- `db_update` is a recursive merge, so this touches `subject` and `edited` and
--- leaves `views`, `posted` and everything else exactly as it was. Writing the
--- whole document back would race with a `db_incr` from someone reading it.
--- @return boolean ok, string|nil why
function M.edit(player, id, subject, body)
    if type(db_update) ~= "function" then return false, "The board is not working." end

    local doc = type(db_get) == "function"
        and unwrap(select(2, pcall(db_get, NOTICES, id)))
    if not doc then return false, "There is no such notice." end
    if doc.char_id ~= player.char_id then return false, "That is not your notice." end

    local patch = { edited = os_time() }
    if type(subject) == "string" and #subject > 0 then patch.subject = subject end
    if type(body) == "string" and #body > 0 then patch.body = body end

    local ok, changed = pcall(db_update, NOTICES, id, patch)
    if not ok then
        log_error("BOARD_D: could not edit '" .. tostring(id) .. "': " .. tostring(changed))
        return false, "The notice would not change."
    end
    return changed == true
end

--- Take a notice down. The author, or anyone with the moderation permission.
--- @return boolean ok, string|nil why
function M.remove(player, id, is_staff)
    if type(db_delete) ~= "function" then return false, "The board is not working." end
    if type(db_exists) == "function" then
        local ok, there = pcall(db_exists, NOTICES, id)
        if ok and not there then return false, "There is no such notice." end
    end

    local doc = type(db_get) == "function"
        and unwrap(select(2, pcall(db_get, NOTICES, id)))
    if not doc then return false, "There is no such notice." end
    if doc.char_id ~= player.char_id and not is_staff then
        return false, "That is not your notice."
    end

    local ok, removed = pcall(db_delete, NOTICES, id)
    if not ok then
        log_error("BOARD_D: could not remove '" .. tostring(id) .. "': " .. tostring(removed))
        return false, "The notice would not come down."
    end

    -- A moderation action, so it belongs in the audit trail rather than the
    -- journal: the question it answers is "who did this", not "what broke".
    if doc.char_id ~= player.char_id and DAEMON and DAEMON.audit then
        pcall(DAEMON.audit.log, "board.remove", true,
            "removed notice " .. id .. " by " .. tostring(doc.author))
    end
    return removed == true
end

--- Strip the `sticky` flag from a notice, using `db_unset`.
---
--- This exists because Lua tables cannot hold `nil`, so RFC 7396's
--- delete-by-null is unreachable through `db_update` from Lua — removing a
--- field needs its own operation, and this is the one place the board needs it.
--- @return boolean
function M.unstick(id)
    if type(db_unset) ~= "function" then return false end
    local ok, done = pcall(db_unset, NOTICES, id, "sticky")
    return ok and done == true
end

--- Delete notices that have run out. Housekeeping — `list` already filters them
--- out, so this is about the table not growing forever.
--- @return number  removed
function M.sweep()
    if not available() or type(db_delete) ~= "function" then return 0 end
    local ok, rows = pcall(db_find, NOTICES,
        { expires = { ["<="] = os_time() } }, { limit = 100 })
    if not ok then return 0 end

    local n = 0
    for _, row in ipairs(rows) do
        local dok = pcall(db_delete, NOTICES, row.id)
        if dok then n = n + 1 end
    end
    if n > 0 then log("info", "BOARD_D: swept " .. n .. " expired notice(s)") end
    return n
end

-- ─── The sweep task ──────────────────────────────────────────────────────────

if DAEMON and DAEMON.task then
    local ok, err = pcall(DAEMON.task.schedule, {
        id       = "board.sweep",
        interval = 3600,
        label    = "Remove expired notices",
        func     = function() return M.sweep() end,
    })
    if not ok then
        log_error("BOARD_D: could not schedule the sweep: " .. tostring(err))
    end
end

log("info", "board_d loaded")

return M
