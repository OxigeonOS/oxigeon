-- mudlib/compute/pathfind.lua — Route finding, on a worker thread.
--
-- A **compute module**: pure Lua, no efuns, no access to the live world. It is
-- handed a graph that was copied across the boundary and returns a path. It
-- cannot see a door open while it runs, which is not a limitation to work
-- around — it is the reason the answer has to be revalidated before it is used.
--
-- In the mudlib rather than the game layer: breadth-first search over a graph
-- of room ids is not this game's algorithm, it is *an* algorithm, and every
-- game that has rooms wants it. What to do with a route — whether to walk it,
-- whether to charge for it, what to say — is content, and that is
-- `game/cmds/navigate.lua`.
--
-- Why this is worth taking off the game thread at all: the whole game runs on
-- one Lua thread, so a breadth-first search over an eighty-by-eighty grid
-- freezes every player for its duration. It is not a long freeze. It is a
-- freeze that gets longer every time somebody adds an area, and the shape of
-- the fix does not change with the size of the problem.

local M = {}

--- Breadth-first, because every exit costs the same.
---
--- Dijkstra would be the same code with a priority queue and would buy nothing
--- until an exit has a weight — a locked door that costs ten, a boat that costs
--- one. Worth noting rather than writing.
---
--- @param args table  { graph = { room_id = { dir = room_id } }, from, to,
---                      max_nodes }
--- @return table  { path = { { dir, room } }, rooms = { room_id }, cost, visited }
function M.route(args)
    if type(args) ~= "table" or type(args.graph) ~= "table" then
        return { path = {}, rooms = {}, cost = -1, error = "no graph" }
    end
    local graph, from, to = args.graph, args.from, args.to
    if type(from) ~= "string" or type(to) ~= "string" then
        return { path = {}, rooms = {}, cost = -1, error = "no endpoints" }
    end
    if from == to then
        return { path = {}, rooms = { from }, cost = 0, visited = 1 }
    end
    if not graph[from] then
        return { path = {}, rooms = {}, cost = -1, error = "you are nowhere the map knows" }
    end

    -- A ceiling, so a disconnected destination costs a bounded search rather
    -- than the whole graph. The caller sets it from how big the graph was.
    local max_nodes = tonumber(args.max_nodes) or 20000

    local came_from = {}          -- room -> { room = previous, dir = direction }
    local seen = { [from] = true }
    local queue, head = { from }, 1
    local visited = 0

    while head <= #queue do
        local current = queue[head]
        head = head + 1
        visited = visited + 1

        if visited > max_nodes then
            return { path = {}, rooms = {}, cost = -1, visited = visited,
                     error = "too far to plan" }
        end

        -- A long search should notice it is out of time and say so, rather
        -- than being killed and reported as a timeout with nothing in it. The
        -- intrinsic is one of the three a worker has.
        if visited % 512 == 0 then
            if type(compute_cancelled) == "function" and compute_cancelled() then
                return { path = {}, rooms = {}, cost = -1, visited = visited,
                         error = "cancelled" }
            end
            if type(compute_deadline_ms) == "function" and compute_deadline_ms() < 50 then
                return { path = {}, rooms = {}, cost = -1, visited = visited,
                         error = "ran out of time" }
            end
        end

        -- Sorted, so two runs over one graph give the same path. `pairs` order
        -- would make a route that changes between identical requests, which is
        -- indistinguishable from a bug in the world.
        local exits = graph[current] or {}
        local dirs = {}
        for dir in pairs(exits) do dirs[#dirs + 1] = dir end
        table.sort(dirs)

        for _, dir in ipairs(dirs) do
            local next_room = exits[dir]
            if type(next_room) == "string" and not seen[next_room] then
                seen[next_room] = true
                came_from[next_room] = { room = current, dir = dir }

                if next_room == to then
                    -- Walk back and reverse.
                    local path, rooms = {}, { to }
                    local cursor = to
                    while cursor ~= from do
                        local step = came_from[cursor]
                        table.insert(path, 1, { dir = step.dir, room = cursor })
                        table.insert(rooms, 1, step.room)
                        cursor = step.room
                    end
                    return { path = path, rooms = rooms, cost = #path, visited = visited }
                end

                queue[#queue + 1] = next_room
            end
        end
    end

    return { path = {}, rooms = {}, cost = -1, visited = visited,
             error = "there is no way there from here" }
end

--- How far everything is from one room. For a "what is near me" question, and
--- for a test that wants a job that takes measurably longer than `route`.
--- @param args table  { graph, from, limit }
--- @return table  { distances = { room_id = n }, reached }
function M.reachable(args)
    if type(args) ~= "table" or type(args.graph) ~= "table" then
        return { distances = {}, reached = 0 }
    end
    local graph, from = args.graph, args.from
    if not graph[from] then return { distances = {}, reached = 0 } end

    local limit = tonumber(args.limit) or 10000
    local distances = { [from] = 0 }
    local queue, head = { from }, 1
    local reached = 0

    while head <= #queue and reached < limit do
        local current = queue[head]
        head = head + 1
        reached = reached + 1

        for _, next_room in pairs(graph[current] or {}) do
            if type(next_room) == "string" and distances[next_room] == nil then
                distances[next_room] = distances[current] + 1
                queue[#queue + 1] = next_room
            end
        end
    end

    return { distances = distances, reached = reached }
end

return M
