local M = {}

M._rooms = {}
M._locations = {}
M._virtuals = {}    -- prefix → generator function
M._area_meta = {}   -- area_name → metadata table

-- ─── Area metadata ───────────────────────────────────────────────────────────

--- Store metadata for a loaded area. Called by ROOM_D.load_area().
-- @param meta table  The _meta table from the area file
function M.set_area_meta(area_name, meta)
    M._area_meta[area_name] = meta
    log("debug", "world_d: Stored metadata for area '" .. area_name .. "'")
end

--- Get metadata for a loaded area.
-- @param area_name string
-- @return table|nil
function M.get_area_meta(area_name)
    return M._area_meta[area_name]
end

--- Get all loaded area metadata.
-- @return table  { area_name → meta }
function M.all_area_meta()
    local copy = {}
    for name, meta in pairs(M._area_meta) do
        copy[name] = meta
    end
    return copy
end

-- ─── Room registry ───────────────────────────────────────────────────────────

function M.get_room(room_id)
    -- 1. Check static registry
    if M._rooms[room_id] then
        return M._rooms[room_id]
    end

    -- 2. Check virtual providers by prefix
    local prefix = room_id:match("^([^%.]+)")
    if prefix and M._virtuals[prefix] then
        local ok, room = pcall(M._virtuals[prefix], room_id)
        if ok and room then
            M._rooms[room_id] = room  -- cache while occupied
            return room
        elseif not ok then
            log("error", "world_d: Virtual provider '" .. prefix
                .. "' failed for '" .. room_id .. "': " .. tostring(room))
        end
    end

    return nil
end

function M.register_room(room)
    if not room or not room.id then
        log("warn", "world_d: Attempted to register a room with no ID")
        return
    end
    if M._rooms[room.id] then
        log("warn", "world_d: Overwriting existing room '" .. room.id .. "'")
    end
    M._rooms[room.id] = room

    -- Feed the reverse index as rooms arrive. `index` replaces rather than
    -- adds, so an area reload cannot leave a room's old tags behind — which is
    -- the failure mode an index has and a linear scan does not.
    if DAEMON and DAEMON.tag then
        pcall(DAEMON.tag.index, "room", room.id, room.tags)
    end
end

function M.register_area(rooms_array)
    if type(rooms_array) ~= "table" then
        log("error", "world_d: register_area called with non-table argument")
        return
    end
    local count = 0
    for _, room in ipairs(rooms_array) do
        M.register_room(room)
        count = count + 1
    end
    log("info", "world_d: Registered " .. count .. " rooms in area.")
end

-- ─── Virtual providers ───────────────────────────────────────────────────────

--- Register a virtual room provider for a given room ID prefix.
-- When get_room() can't find a room in the static registry, it checks
-- virtual providers by matching the prefix (everything before the first dot).
-- The generator function receives the full room_id and must return a Room object
-- (via ROOM_D.from_data()) or nil.
-- @param prefix string         e.g. "ocean", "desert", "sky"
-- @param generator function    function(room_id) → Room|nil
function M.register_virtual(prefix, generator)
    if type(prefix) ~= "string" or prefix == "" then
        log("error", "world_d: register_virtual requires a non-empty string prefix")
        return
    end
    if type(generator) ~= "function" then
        log("error", "world_d: register_virtual requires a function generator")
        return
    end
    M._virtuals[prefix] = generator
    log("info", "world_d: Registered virtual provider for prefix '" .. prefix .. "'")
end

--- Unregister a virtual provider.
function M.unregister_virtual(prefix)
    M._virtuals[prefix] = nil
end

--- List registered virtual prefixes.
function M.virtual_prefixes()
    local prefixes = {}
    for prefix, _ in pairs(M._virtuals) do
        prefixes[#prefixes + 1] = prefix
    end
    return prefixes
end

--- Evict a cached virtual room (e.g. when no players are in it).
-- Only removes from the room registry — does not affect static rooms.
--
-- The room *id* is the persistence, not the cached object: a virtual room is
-- regenerated from its coordinates on the next visit, identical to the one
-- thrown away. Its object state goes with it for the same reason a mob's does
-- on despawn — the store is keyed by object id and nothing else prunes it, and
-- an infinite grid over a registry nobody prunes is an unbounded leak rather
-- than a large one.
function M.evict_virtual(room_id)
    local prefix = room_id:match("^([^%.]+)")
    if prefix and M._virtuals[prefix] then
        M._rooms[room_id] = nil
        if type(clear_object_state) == "function" then
            pcall(clear_object_state, room_id)
        end
        log("debug", "world_d: Evicted virtual room '" .. room_id .. "'")
    end
end

--- Is this a virtual room, and is there nobody left in it?
---
--- Called on every departure, so it has to be cheap: one pattern match against
--- the prefix table, and the occupant count only when that says yes.
local function virtual_and_empty(room_id)
    if type(room_id) ~= "string" then return false end
    local prefix = room_id:match("^([^%.]+)")
    if not (prefix and M._virtuals[prefix]) then return false end

    local room = M._rooms[room_id]
    if not room then return false end
    local occupants = room.get_characters and room:get_characters() or {}
    return #occupants == 0
end

--- Drop a virtual room the moment its last occupant leaves.
---
--- `evict_virtual` had **zero callers** — not in `mudlib/`, `game/`, `tests/`
--- or `src/`. `world-building.md` said a virtual room is "cached in the
--- registry while occupied"; nothing un-cached it, so `_rooms` accumulated
--- every virtual room ever generated, each holding its exits table, contents,
--- actions and description closures. Bounded for a small ocean; unbounded for
--- an infinite grid.
---
--- Deliberately eager rather than on a sweep timer: the condition is exact and
--- known at exactly this moment, and a timer would need to re-derive it for
--- every cached room on every tick to learn the same thing later.
local function evict_if_abandoned(room_id)
    if virtual_and_empty(room_id) then
        M.evict_virtual(room_id)
    end
end

-- ─── The exit graph ──────────────────────────────────────────────────────────

--- Every room's exits, as plain data.
---
--- For [`compute()`](../../docs/src/lua-api/compute.md): a worker VM has no
--- efuns and cannot see the world, so the world has to be copied to it. Plain
--- strings only — no Room objects, no closures — because the marshaller refuses
--- both and a graph that cannot cross the boundary is a graph nobody can plan
--- a route over.
---
--- **Virtual rooms are included only if they are cached**, which is to say only
--- if somebody is standing in one. An infinite grid cannot be enumerated, and
--- pretending otherwise is how a pathfinder comes to hang. A caller that wants
--- to plan a route *through* a virtual area passes `expand`, which asks the
--- provider for the neighbours of what it already has, breadth-first, up to a
--- bound.
---
--- @param opts table|nil  { expand = { provider = fn, from = room_id,
---                                     radius = n } }
--- @return table  room_id -> { direction = room_id }
function M.exit_graph(opts)
    local graph = {}

    local function exits_of(room)
        local out = {}
        for dir, exit in pairs(room.exits or {}) do
            local target = type(exit) == "table" and exit.target or exit
            -- A hidden exit is still an exit — you can walk it if you know the
            -- direction — so it belongs in a route. A `check` that refuses is
            -- what `still_connected` is for, and it is checked when the route
            -- is *used* rather than when it is planned, because a locked door
            -- may be open by then.
            if type(target) == "string" then out[dir] = target end
        end
        return out
    end

    for room_id, room in pairs(M._rooms) do
        graph[room_id] = exits_of(room)
    end

    local expand = opts and opts.expand
    if type(expand) == "table" and type(expand.provider) == "function"
        and type(expand.from) == "string" then
        -- Breadth-first from one room, asking the provider for neighbours it
        -- has not been asked for yet. Bounded, because the whole point of a
        -- virtual area is that it does not end.
        -- `radius + 1` passes, so a radius of 3 means "every room within three
        -- steps has an entry". Pass *i* fills the rooms at distance *i-1*, so
        -- looping `radius` times would fill only to `radius - 1` — an
        -- off-by-one that shows up as a pathfinder that cannot see the last
        -- ring it was told to consider.
        local radius = tonumber(expand.radius) or 20
        local frontier, next_frontier = { expand.from }, {}
        local seen = { [expand.from] = true }

        for _ = 1, radius + 1 do
            for _, room_id in ipairs(frontier) do
                if not graph[room_id] then
                    local ok, neighbours = pcall(expand.provider, room_id)
                    graph[room_id] = (ok and type(neighbours) == "table") and neighbours or {}
                end
                for _, target in pairs(graph[room_id]) do
                    if type(target) == "string" and not seen[target] then
                        seen[target] = true
                        next_frontier[#next_frontier + 1] = target
                    end
                end
            end
            if #next_frontier == 0 then break end
            frontier, next_frontier = next_frontier, {}
        end
    end

    return graph
end

--- Is this route still walkable *now*?
---
--- The most important function on this page, and the reason it exists is worth
--- stating: **a compute result is a proposal about a world that has since
--- changed**, never an authoritative fact. Nothing stopped the game while the
--- job ran — that is the entire point of running it off-thread — so anything it
--- computed may be stale by the time you have it.
---
--- Checks the exits still exist *and* that their `check` functions still pass,
--- which is the half a graph cannot carry: a locked door is an exit that is
--- there and refuses.
--- @param rooms table   array of room ids, in order
--- @param player table|nil  for exit checks that read the walker
--- @return boolean ok, string|nil where it broke
function M.still_connected(rooms, player)
    if type(rooms) ~= "table" or #rooms < 2 then return true end

    for i = 1, #rooms - 1 do
        local room = M.get_room(rooms[i])
        if not room then return false, rooms[i] end

        local linked = false
        for _, exit in pairs(room.exits or {}) do
            local target = type(exit) == "table" and exit.target or exit
            if target == rooms[i + 1] then
                if type(exit) == "table" and type(exit.check) == "function" then
                    local ok, passed = pcall(exit.check, player)
                    linked = ok and passed == true
                else
                    linked = true
                end
                if linked then break end
            end
        end
        if not linked then return false, rooms[i] end
    end
    return true
end

-- ─── Character location tracking ─────────────────────────────────────────────

--- Announce an arrival or a departure.
---
--- `room.entered`, `room.left` and `character.left` are documented event names
--- that nothing emitted — the naming convention existed and the events did not,
--- so an aggro handler or a quest trigger had nothing to listen to. They are
--- emitted from here rather than from `movement.lua` because every path into a
--- room goes through these three functions: walking, teleporting, logging in
--- and being moved by a room action. Emitting from the walk would have covered
--- one of the four.
---
--- Protected, and after the state change: a listener that raises must not leave
--- a character half-moved, and one that asks where somebody is must get the
--- answer that is now true.
local function announce(event, data)
    if not (DAEMON and DAEMON.event) then return end
    local ok, err = pcall(DAEMON.event.emit, event, data)
    if not ok then
        log("error", "world_d: emitting '" .. event .. "' failed: " .. tostring(err))
        if DAEMON.journal then
            pcall(DAEMON.journal.error, "WORLD_D: '" .. event .. "' listener raised: "
                .. tostring(err))
        end
    end
end

function M.move_character(char_id, target_room_id)
    local old_room_id = M._locations[char_id]
    if old_room_id then
        local old_room = M._rooms[old_room_id]
        if old_room then
            old_room:remove_character(char_id)
        end
    end

    local new_room = M.get_room(target_room_id) -- uses virtual fallback
    if new_room then
        new_room:add_character(char_id)
        M._locations[char_id] = target_room_id
        -- After the arrival, not before: walking one step within the virtual
        -- grid would otherwise evict the room you are about to re-enter, and
        -- doing it in this order also means a move that failed leaves the room
        -- you are still standing in alone.
        if old_room_id and old_room_id ~= target_room_id then
            evict_if_abandoned(old_room_id)
        end

        if old_room_id and old_room_id ~= target_room_id then
            announce("room.left", {
                char_id = char_id, room_id = old_room_id, to_room_id = target_room_id,
            })
        end
        announce("room.entered", {
            char_id = char_id, room_id = target_room_id, from_room_id = old_room_id,
        })
        return true
    end
    log("warn", "world_d: move_character failed — room '"
        .. tostring(target_room_id) .. "' not found")
    return false
end

function M.get_character_room(char_id)
    return M._locations[char_id]
end

function M.get_character_room_obj(char_id)
    local room_id = M._locations[char_id]
    if room_id then
        return M.get_room(room_id) -- uses virtual fallback
    end
    return nil
end

function M.place_character(char_id, room_id)
    local room = M.get_room(room_id) -- uses virtual fallback
    if room then
        room:add_character(char_id)
        M._locations[char_id] = room_id
        log("debug", "world_d: Placed character " .. tostring(char_id)
            .. " in room '" .. room_id .. "'")
        announce("room.entered", { char_id = char_id, room_id = room_id, from_room_id = nil })
    else
        log("error", "world_d: Cannot place character " .. tostring(char_id)
            .. " — room '" .. tostring(room_id) .. "' does not exist!")
    end
end

function M.remove_character(char_id)
    local room_id = M._locations[char_id]
    if room_id then
        local room = M._rooms[room_id]
        if room then
            room:remove_character(char_id)
        end
        M._locations[char_id] = nil
        log("debug", "world_d: Removed character " .. tostring(char_id)
            .. " from room '" .. room_id .. "'")
        -- Disconnecting inside the virtual grid is the common case for the
        -- last occupant leaving, so it has to evict too — otherwise every
        -- player who ever logged out at sea leaves a room behind.
        evict_if_abandoned(room_id)

        announce("room.left", { char_id = char_id, room_id = room_id, to_room_id = nil })
        -- Distinct from `room.left`: leaving a room and leaving the world are
        -- different events, and a listener cleaning up per-character timers
        -- wants the second one.
        announce("character.left", { char_id = char_id, room_id = room_id })
    else
        log("debug", "world_d: remove_character called for character "
            .. tostring(char_id) .. " who had no location")
    end
end

-- ─── Area source tracking & resets ───────────────────────────────────────────

-- Stores the require-path for each area so we can reload it fresh.
-- area_name → { module = "areas.wizard_workshop.rooms", items_module = "areas.wizard_workshop.items" }
M._area_sources = {}

local function log_error(message)
    log("error", message)
    if DAEMON and DAEMON.journal then
        pcall(function() DAEMON.journal.error(message) end)
    end
end

--- Register the Lua module paths for an area so it can be reloaded later.
-- @param area_name     string  e.g. "wizard_workshop"
-- @param room_module   string  require-path for rooms, e.g. "areas.wizard_workshop.rooms"
-- @param items_module  string|nil  optional require-path for items
function M.register_area_source(area_name, room_module, items_module)
    if type(area_name) ~= "string" or area_name == "" then
        log("warn", "world_d: register_area_source requires a non-empty area name")
        return
    end
    if type(room_module) ~= "string" or room_module == "" then
        log("warn", "world_d: register_area_source requires a non-empty room module path")
        return
    end
    M._area_sources[area_name] = {
        module       = room_module,
        items_module = items_module,
    }
    log("debug", "world_d: Registered area source '" .. area_name
        .. "' (module=" .. room_module .. ")")
end

--- Get all room IDs belonging to a given area (prefix match on room_id).
-- @param area_name string  e.g. "wizard_workshop"
-- @return table  array of room ID strings
function M.get_area_rooms(area_name)
    local prefix = area_name .. "."
    local result = {}
    for room_id, _ in pairs(M._rooms) do
        if room_id:sub(1, #prefix) == prefix then
            result[#result + 1] = room_id
        end
    end
    return result
end

--- Reset a single area: reload its Lua module, rebuild Room objects,
-- clear object state for all rooms, and re-place characters.
-- Players currently in the area stay in their room (the Room object is new
-- but the ID is the same).
-- @param area_name string  e.g. "wizard_workshop"
-- @return boolean, string  success, message
function M.reset_area(area_name)
    local source = M._area_sources[area_name]
    if not source then
        return false, "No registered source for area '" .. area_name .. "'"
    end

    log("info", "world_d: Resetting area '" .. area_name .. "'...")

    -- 1. Collect characters currently in this area's rooms
    local prefix = area_name .. "."
    local chars_in_area = {}  -- { char_id = room_id }
    for char_id, room_id in pairs(M._locations) do
        if room_id:sub(1, #prefix) == prefix then
            chars_in_area[char_id] = room_id
        end
    end

    -- 2. Remove characters from old Room objects so they don't hold stale refs
    for char_id, room_id in pairs(chars_in_area) do
        local old_room = M._rooms[room_id]
        if old_room then
            local ok, err = pcall(old_room.remove_character, old_room, char_id)
            if not ok then
                log_error("world_d: Failed to remove char " .. tostring(char_id)
                    .. " from old room during reset: " .. tostring(err))
            end
        end
    end

    -- 3. Purge old rooms belonging to this area
    local old_room_ids = M.get_area_rooms(area_name)
    for _, room_id in ipairs(old_room_ids) do
        M._rooms[room_id] = nil
    end

    -- 4. Clear object state for all rooms in this area
    if type(clear_object_state) == "function" then
        for _, room_id in ipairs(old_room_ids) do
            local ok, err = pcall(clear_object_state, room_id)
            if not ok then
                log_error("world_d: Failed to clear object state for '"
                    .. room_id .. "': " .. tostring(err))
            end
        end
    end

    -- 5. Purge the require cache so the module is re-evaluated fresh
    package.loaded[source.module] = nil
    if source.items_module then
        package.loaded[source.items_module] = nil
    end

    -- 6. Re-require items (must come before rooms in case rooms reference items)
    if source.items_module and DAEMON.items then
        local ok, err = pcall(function()
            local items = require(source.items_module)
            DAEMON.items.register_all(items)
        end)
        if not ok then
            log_error("world_d: Failed to reload items for area '"
                .. area_name .. "': " .. tostring(err))
        end
    end

    -- 7. Re-require the area module, rebuild rooms, register them
    local ok, err = pcall(function()
        local area_data = require(source.module)
        local rooms = DAEMON.room.load_area(area_data)
        M.register_area(rooms)
    end)
    if not ok then
        log_error("world_d: Failed to reload area '" .. area_name
            .. "': " .. tostring(err))
        return false, "Reload failed: " .. tostring(err)
    end

    -- 8. Re-place characters into the new Room objects
    for char_id, room_id in pairs(chars_in_area) do
        local new_room = M._rooms[room_id]
        if new_room then
            new_room:add_character(char_id)
            -- _locations[char_id] still points to the same room_id, no change needed
        else
            -- Room disappeared during reload — move character to start room
            local start = config and config("game.start_room") or nil
            if start and M._rooms[start] then
                M._rooms[start]:add_character(char_id)
                M._locations[char_id] = start
                log("warn", "world_d: Room '" .. room_id
                    .. "' gone after reset, moved char " .. tostring(char_id)
                    .. " to start room")
            else
                log_error("world_d: Room '" .. room_id
                    .. "' gone after reset and no start room available for char "
                    .. tostring(char_id))
            end
        end
    end

    local msg = "Area '" .. area_name .. "' reset successfully ("
        .. #old_room_ids .. " rooms, "
        .. (function()
            local c = 0
            for _ in pairs(chars_in_area) do c = c + 1 end
            return c
        end)() .. " characters repositioned)"
    log("info", "world_d: " .. msg)

    if DAEMON and DAEMON.journal then
        pcall(function() DAEMON.journal.info("WORLD_D: " .. msg) end)
    end

    return true, msg
end

--- Reset all registered areas.
-- @return number  count of areas successfully reset
function M.reset_all_areas()
    local count = 0
    for area_name, _ in pairs(M._area_sources) do
        local ok, msg = M.reset_area(area_name)
        if ok then
            count = count + 1
        else
            log_error("world_d: reset_all_areas — failed for '"
                .. area_name .. "': " .. tostring(msg))
        end
    end
    log("info", "world_d: reset_all_areas complete (" .. count .. " areas reset)")
    return count
end

return M

