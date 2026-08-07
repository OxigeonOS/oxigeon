-- mudlib/cmds/queue.lua — What you have committed to doing next.
--
-- A track is a lane of intent with its own pace: combat now, crafting and
-- gathering later. This shows one, and lets you change what an empty one does.
--
-- The thing worth knowing, and the reason there is a command at all: an ability
-- used while you are recovering is **queued, not refused**. A refusal makes you
-- a metronome watching a timer; a queue makes the interesting decision "what do
-- I commit to next", which is the whole shape of the mini-game.

local Queues = require('lib.queues')

local M = {}

M.name       = "queue"
-- Not `q`: the game layer's `quest` already claims it, and a mudlib command
-- taking an alias out from under a game command would silently change what an
-- already-typed key does.
M.aliases    = { "actions" }
M.category   = "combat"
M.summary    = "What you have queued, and what an empty queue does."
M.usage      = {
    "queue                     the combat track",
    "queue <track>             another lane — crafting, gathering",
    "queue clear [<track>]     drop what is pending",
    "queue next <ability> [at <target>]   jump the queue",
    "queue auto|idle|repeat    what an empty queue does",
}
M.permission = nil

local POLICY_HELP = {
    auto   = "keep acting on your own",
    idle   = "stand there until you say otherwise",
    ["repeat"] = "do the last thing again",
}

local function show(player, track)
    local Q = DAEMON.queue
    local spec = Q.track(track)
    if not spec then
        player:send("{red}There is no '" .. track .. "' track.{/} Tracks: "
            .. table.concat(Q.tracks(), ", "))
        return
    end

    local lines = { "{cyan}" .. track .. "{/}" }

    local left = Q.roundtime(player, track)
    if left > 0 then
        lines[#lines + 1] = "  Recovering: {yellow}" .. math.ceil(left) .. "s{/}"
    else
        lines[#lines + 1] = "  Recovering: {green}ready{/}"
    end

    local entries = Q.list(player, track)
    if #entries == 0 then
        lines[#lines + 1] = "  Queued:     {dim}(nothing){/}"
    else
        for i, e in ipairs(entries) do
            local at = e.target and (" at " .. require('lib.render').display_name(e.target)) or ""
            lines[#lines + 1] = string.format("  %d. %s%s", i, tostring(e.id or e.kind), at)
        end
    end

    local policy = Q.policy(player, track)
    lines[#lines + 1] = "  When empty: " .. policy
        .. " {dim}(" .. (POLICY_HELP[policy] or "") .. "){/}"

    local history = Q.history(player, track)
    if #history > 0 then
        local ids = {}
        for _, e in ipairs(history) do ids[#ids + 1] = tostring(e.id or e.kind) end
        lines[#lines + 1] = "  {dim}Recently: " .. table.concat(ids, ", ") .. "{/}"
    end

    player:send_lines(lines)
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not (DAEMON and DAEMON.queue) then
        player:send("{red}Action queues are unavailable (queue_d is not loaded).{/}")
        return
    end

    args_str = (args_str or ""):gsub("^%s+", ""):gsub("%s+$", "")
    local verb, rest = args_str:match("^(%S+)%s*(.*)$")
    verb = (verb or ""):lower()

    if verb == "" then return show(player, "combat") end

    if verb == "clear" then
        local track = rest ~= "" and rest or "combat"
        local n = DAEMON.queue.clear(player, track)
        player:send(n > 0
            and ("{green}Dropped " .. n .. " queued action(s).{/}")
            or "Nothing was queued.")
        return
    end

    if POLICY_HELP[verb] then
        local ok, why = DAEMON.queue.set_policy(player, "combat", verb)
        if not ok then return player:send("{red}" .. tostring(why) .. "{/}") end
        player:send("{green}With nothing queued you will now " .. POLICY_HELP[verb] .. ".{/}")
        return
    end

    if verb == "next" then
        if not DAEMON.ability then
            return player:send("{red}Abilities are unavailable.{/}")
        end
        local id, target = rest:match("^(%S+)%s+at%s+(.+)$")
        if not id then id, target = rest:match("^(%S+)%s+(.+)$") end
        if not id then id = rest end
        if id == "" then return player:send_lines(M.usage) end

        local ok, why = DAEMON.queue.enqueue(player, "combat",
            { kind = "ability", id = id:lower(), target = target }, { front = true })
        if not ok then return player:send("{red}" .. tostring(why) .. "{/}") end
        player:send("{green}Queued " .. id:lower() .. " next.{/}")
        return
    end

    -- Anything else is a track name. Last, so a future subcommand can never be
    -- shadowed by somebody's track — the same ordering rule `olc` uses.
    if DAEMON.queue.track(verb) then return show(player, verb) end

    player:send("{red}Unknown option '" .. verb .. "'.{/}")
    player:send_lines(M.usage)
end

return M
