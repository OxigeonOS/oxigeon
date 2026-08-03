-- game/cmds/navigate.lua — Plan a route without freezing the game.
--
-- The worked example from `compute.md`, made real. A breadth-first search over
-- the world graph is not slow; it is slow *enough*, and it gets slower every
-- time somebody adds an area. The shape of the fix does not change with the
-- size of the problem, so it is worth having before the problem is bad.
--
--   navigate thornhollow.square      plan and show the route
--   navigate walk                    follow the last route you planned
--   navigate cancel                  give up on a route still being planned
--
-- ─── The important part ──────────────────────────────────────────────────────
--
-- The route is **revalidated before it is walked**, not when it arrives. A
-- compute result is a proposal about a world that has since changed: nothing
-- stopped the game while the job ran, which is the entire point, so a door may
-- have shut and a virtual room may have been evicted and regenerated. Walking a
-- stale route is how a player ends up somewhere the map says they cannot be.

local M = {}
M.name = 'navigate'
M.aliases = { 'nav', 'route' }
M.category = 'navigation'
M.summary = 'Work out a route to somewhere, off the game thread.'
M.usage = {
    "navigate <room_id>   plan a route",
    "navigate walk        follow the route you planned",
    "navigate cancel      abandon one still being planned",
}
M.permission = nil

--- char_id -> { rooms, path, to, planned_at }. Memory tier by the rule in
--- state-cache.md: a route is worth nothing after a restart, and a route
--- somebody planned an hour ago is worth nothing either.
M._routes = {}
--- char_id -> compute job id, so `cancel` has something to cancel.
M._pending = {}

--- How far into the virtual grid to expand the graph. An infinite area cannot
--- be enumerated, so the graph is grown outward from where you are, bounded.
local REACH_RADIUS = 25

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then pcall(DAEMON.journal.error, message) end
end

--- The graph to plan over, including as much of the reach as is worth carrying.
local function build_graph(from)
    local expand = nil
    if DAEMON.reach and DAEMON.reach.coords and select(1, DAEMON.reach.coords(from)) then
        expand = {
            provider = DAEMON.reach.neighbours,
            from     = from,
            radius   = REACH_RADIUS,
        }
    end
    return DAEMON.world.exit_graph({ expand = expand })
end

function M.execute(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    local verb = (args[1] or ""):lower()

    if verb == "" then
        player:send_lines(M.usage)
        return
    end

    if verb == "cancel" then
        local job = M._pending[player.char_id]
        if not job then
            player:send("You are not planning anything.")
            return
        end
        M._pending[player.char_id] = nil
        local cancelled = type(compute_cancel) == "function" and compute_cancel(job)
        player:send(cancelled and "{yellow}Route abandoned.{/}"
            or "{yellow}It had already finished.{/}")
        return
    end

    if verb == "walk" then
        M.walk(player)
        return
    end

    -- ─── Plan ────────────────────────────────────────────────────────────────
    local destination = args_str
    if not DAEMON.world.get_room(destination) then
        player:send("{red}There is no room called '" .. destination .. "'.{/}")
        return
    end

    local from = DAEMON.world.get_character_room(player.char_id)
    if not from then
        player:send("{red}You are nowhere.{/}")
        return
    end
    if from == destination then
        player:send("You are already there.")
        return
    end

    if type(compute) ~= "function" then
        player:send("{red}Route planning is not available on this server.{/}")
        return
    end

    local graph = build_graph(from)
    local nodes = 0
    for _ in pairs(graph) do nodes = nodes + 1 end

    -- `compute.pathfind` lives in the mudlib: breadth-first search over a
    -- graph of room ids is not this game's algorithm. What to do with a route
    -- is, and that is this file.
    local id, err = compute("compute.pathfind", "route", {
        graph = graph,
        from = from,
        to = destination,
        -- Bounded by the graph it was given, so a disconnected destination
        -- costs a bounded search rather than the whole world.
        max_nodes = nodes * 4,
    }, {
        tag = "navigate:" .. tostring(player.char_id) .. ":" .. destination,
        deadline_ms = 3000,
    })

    if not id then
        -- `nil` is only for mistakes correct code never makes. Everything
        -- operational, a full queue included, arrives through the hook.
        player:send("{red}Cannot plan a route right now: " .. tostring(err) .. "{/}")
        return
    end

    M._pending[player.char_id] = id
    player:send("{cyan}Plotting a course over " .. nodes .. " rooms...{/}")
    player:send("The game does not stop while this happens. Carry on.")
end

--- Called from `on_compute_result`. Wired in `game/init.lua`, because the
--- mudlib's hook has to dispatch to whoever asked.
function M.on_result(id, ok, value, err, meta)
    local tag = type(meta) == "table" and meta.tag
    if type(tag) ~= "string" then return false end
    local char_id, destination = tag:match("^navigate:(%d+):(.+)$")
    if not char_id then return false end
    char_id = tonumber(char_id)

    M._pending[char_id] = nil
    local player = DAEMON.character and DAEMON.character.get(char_id)
    if not player then return true end   -- they logged out; nothing to say

    if not ok then
        player:send("{red}Route planning failed: " .. tostring(err)
            .. " (" .. tostring(meta.kind) .. "){/}")
        return true
    end
    if type(value) ~= "table" or (value.cost or -1) < 0 then
        player:send("{yellow}" .. (value and value.error or "No route.") .. "{/}")
        return true
    end

    M._routes[char_id] = {
        rooms      = value.rooms,
        path       = value.path,
        to         = destination,
        planned_at = os_time(),
    }

    local steps = {}
    for _, step in ipairs(value.path) do steps[#steps + 1] = step.dir end
    player:send("{green}Route found: " .. value.cost .. " step(s).{/}")
    player:send("  " .. table.concat(steps, ", "))
    player:send("Walk it with {cyan}navigate walk{/}.")
    return true
end

--- Follow the last route planned, checking it is still there first.
function M.walk(player)
    local route = M._routes[player.char_id]
    if not route then
        player:send("You have not planned a route.")
        return
    end

    -- **The revalidation.** A compute result is a proposal about a world that
    -- has since changed. Between planning and walking, a door may have shut, an
    -- area may have reset, and a virtual room may have been evicted and
    -- regenerated. Checked at *walk* time rather than at arrival time on
    -- purpose: a route planned and walked ten minutes apart is the case that
    -- matters, and checking on arrival would only have caught the fast one.
    local still, broke = DAEMON.world.still_connected(route.rooms, player)
    if not still then
        M._routes[player.char_id] = nil
        player:send("{yellow}The way has changed since you set out"
            .. (broke and (" — at " .. broke) or "") .. ".{/}")
        player:send("Plan it again.")
        return
    end

    local here = DAEMON.world.get_character_room(player.char_id)
    if here ~= route.rooms[1] then
        M._routes[player.char_id] = nil
        player:send("{yellow}You are not where you were when you planned that.{/}")
        return
    end

    local movement = require('lib.movement')
    for _, step in ipairs(route.path) do
        movement.move(player.session_id, step.dir)
    end

    M._routes[player.char_id] = nil
    player:send("{green}You arrive at " .. route.to .. ".{/}")
end

return M
