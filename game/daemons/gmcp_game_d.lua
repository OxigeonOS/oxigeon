-- game/daemons/gmcp_game_d.lua — This game's own GMCP packages.
--
-- `Char`, `Room` and `Core` are conventions every MUD client knows.
-- `Game.Quest` is not: it is this game's, and a client that wants to draw a
-- quest tracker has to be told about it. That is the point of a custom package
-- — and the reason it lives in the game layer rather than in `gmcp_d`.
--
-- Registering one is a line:
--
--     DAEMON.gmcp.on("Game.Quest", function(session_id, data) ... end)
--
-- and pushing one is `send_gmcp`. The mudlib's dispatcher never changes.

local M = {}

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

--- Everything the character is doing, for a client-side quest tracker.
--- @param session_id string
--- @return boolean
function M.send_quests(session_id)
    if not (DAEMON and DAEMON.gmcp and DAEMON.quest) then return false end
    if not DAEMON.gmcp.wants(session_id, "Game.Quest") then return false end

    local player = get_player(session_id)
    if not player then return false end

    local list = {}
    for _, entry in ipairs(DAEMON.quest.journal(player)) do
        if entry.active then
            list[#list + 1] = {
                id       = entry.quest.id,
                name     = entry.quest.name,
                progress = entry.progress,
                needed   = entry.quest.objective.count,
                ready    = entry.ready,
            }
        end
    end

    local ok, err = pcall(send_gmcp, session_id, "Game.Quest", list)
    if not ok then log_error("GMCP_GAME_D: could not send Game.Quest: " .. tostring(err)) end
    return ok
end

--- Inbound: a client asking for the quest list rather than waiting to be told.
---
--- Worth supporting because a client that reconnects, or one whose user just
--- opened the tracker panel, has no other way to ask.
DAEMON.gmcp.on("Game.Quest.Request", function(session_id, data)
    M.send_quests(session_id)
end)

--- Inbound: a client saying which quest the user has selected, so the game can
--- highlight it. Stored on the session rather than persisted — a UI selection
--- is memory-tier by any reading of the rule.
DAEMON.gmcp.on("Game.Quest.Track", function(session_id, data)
    local id = type(data) == "table" and data.id or data
    if type(id) ~= "string" then return end
    if not DAEMON.quest.get(id) then
        pcall(send_gmcp, session_id, "Game.Quest.Track", { id = id, ok = false })
        return
    end
    M._tracking = M._tracking or {}
    M._tracking[session_id] = id
    pcall(send_gmcp, session_id, "Game.Quest.Track", { id = id, ok = true })
end)

--- Push on the events that change what a tracker should show. The client is
--- never asked to poll, which is the whole reason GMCP exists.
if DAEMON and DAEMON.event then
    local function push(data)
        if type(data) ~= "table" or not data.char_id then return end
        local player = DAEMON.character and DAEMON.character.get(data.char_id)
        if player and player.session_id then M.send_quests(player.session_id) end
    end

    local ok, err = pcall(function()
        DAEMON.event.on("quest.accepted",  "gmcp_game.accept",   push)
        DAEMON.event.on("quest.completed", "gmcp_game.complete", push)
    end)
    if not ok then log_error("GMCP_GAME_D: could not subscribe: " .. tostring(err)) end
end

log("info", "gmcp_game_d loaded")

return M
