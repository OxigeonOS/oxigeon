-- mudlib/lib/messaging.lua — Getting a line to the people who should read it.
--
-- `send_to_room` sends one finished string to everybody. That is right for a
-- line nobody is *in* — "The gate grinds shut." — and wrong for almost every
-- line combat and abilities produce, where the attacker should read "you swing"
-- and everybody else should read a name.
--
-- `tell`/`broadcast`/`announce` take an **authored template plus a cast of
-- roles** and render per reader through `lib/render.lua`. The existing two
-- functions are untouched and still the right call for a line with no
-- participants.
--
-- The cost rule: a broadcast **parses once and renders once per distinct role
-- set**, not once per viewer. A forty-person room watching one attacker hit one
-- target does three renders, because everybody who is neither participant reads
-- the same sentence.

local render = require('lib.render')

local M = {}

function M.find_session_for_character(char_id)
    local sessions = all_sessions()
    for _, sid in ipairs(sessions) do
        local s = get_session(sid)
        if s and s.character_id == char_id then
            return sid
        end
    end
    return nil
end

--- Send a message to all characters in a room, excluding specified characters.
-- @param room_id string           The room to broadcast to
-- @param text string              The message text
-- @param exclude_char_id number|table|nil  A single char_id or an array of char_ids to exclude
function M.send_to_room(room_id, text, exclude_char_id)
    local room = DAEMON.world.get_room(room_id)
    if not room then return end

    -- Normalize exclude list: single value → set, table → set, nil → empty set
    local excluded = {}
    if type(exclude_char_id) == "table" then
        for _, id in ipairs(exclude_char_id) do
            excluded[id] = true
        end
    elseif exclude_char_id ~= nil then
        excluded[exclude_char_id] = true
    end

    for _, char_id in ipairs(room:get_characters()) do
        if not excluded[char_id] then
            -- Use the Player object when available for color/wrap support
            local player_obj = DAEMON.character and DAEMON.character.get(char_id)
            if player_obj then
                player_obj:send(text)
            else
                local sid = M.find_session_for_character(char_id)
                if sid then
                    send(sid, text .. "\r\n")
                end
            end
        end
    end
end

-- ─── Per-viewer sending ──────────────────────────────────────────────────────

--- Deliver finished text to one entity, however it can be reached.
---
--- A mob has no `send` and that is not an error — it is a target, not a reader.
local function deliver(entity, text)
    if type(entity) ~= "table" or text == nil or text == "" then return false end
    if type(entity.send) == "function" then
        pcall(entity.send, entity, text)
        return true
    end
    if entity.char_id ~= nil then
        local sid = M.find_session_for_character(entity.char_id)
        if sid then
            send(sid, text .. "\r\n")
            return true
        end
    end
    return false
end

--- Every entity in a room that can read something.
--- @param room_id string
--- @return table  array of entities
local function readers(room_id)
    local out = {}
    local room = DAEMON.world and DAEMON.world.get_room(room_id)
    if not room or not room.get_characters then return out end
    for _, char_id in ipairs(room:get_characters()) do
        local who = DAEMON.character and DAEMON.character.get(char_id)
        -- A character with no live Player object still gets the line through
        -- its session, so a rehydrating login does not silently miss messages.
        out[#out + 1] = who or { char_id = char_id }
    end
    return out
end

--- Render one authored line for one reader and send it.
--- @param entity table   both the reader and, usually, a role in `ctx`
--- @param template string|nil
--- @param ctx table|nil
--- @return boolean  whether anything was sent
function M.tell(entity, template, ctx)
    if template == nil or template == "" then return false end
    return deliver(entity, render.render(template, ctx, entity))
end

--- Render one authored line for everybody in a room, per reader.
--- @param opts table|nil { exclude = entity|char_id|{…}, include = { entities } }
--- @return number  how many were sent to
function M.broadcast(room_id, template, ctx, opts)
    if template == nil or template == "" or type(room_id) ~= "string" then return 0 end
    opts = opts or {}

    local skip = {}
    local function bar(who)
        if type(who) == "table" then
            if who.char_id ~= nil then skip[who.char_id] = true end
        elseif who ~= nil then
            skip[who] = true
        end
    end
    if type(opts.exclude) == "table" and opts.exclude.char_id == nil
        and opts.exclude.id == nil then
        for _, who in ipairs(opts.exclude) do bar(who) end
    else
        bar(opts.exclude)
    end

    local viewers = {}
    for _, who in ipairs(readers(room_id)) do
        if not (who.char_id ~= nil and skip[who.char_id]) then viewers[#viewers + 1] = who end
    end
    for _, who in ipairs(opts.include or {}) do viewers[#viewers + 1] = who end
    if #viewers == 0 then return 0 end

    local lines = render.render_for(template, ctx, viewers)
    local n = 0
    for _, who in ipairs(viewers) do
        if deliver(who, lines[who]) then n = n + 1 end
    end
    return n
end

--- The same, with the room taken from whoever is acting.
---
--- The common case: an ability or a swing is announced where the actor is
--- standing, and a caller should not have to look that up.
--- @return number
function M.announce(template, ctx, opts)
    local actor = ctx and (ctx.actor or ctx.user)
    if type(actor) ~= "table" then return 0 end

    local room_id = actor.room_id
    if room_id == nil and actor.char_id ~= nil and DAEMON.world then
        room_id = DAEMON.world.get_character_room(actor.char_id)
    end
    if type(room_id) ~= "string" then return 0 end

    return M.broadcast(room_id, template, ctx, opts)
end

return M
