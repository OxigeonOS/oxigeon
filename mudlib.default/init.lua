-- mudlib/init.lua — Oxigeon Mudlib Entry Point
-- Loaded by the Oxigeon driver at startup.
-- Defines on_connect, on_input, on_disconnect, on_gmcp, on_load, on_unload globally.

-- ─── Daemon registry ─────────────────────────────────────────────────────────
-- DAEMON is a global table; all daemons attach themselves here on load.
DAEMON = {}

-- Load core daemons (order matters: audit_d and journal_d are foundational)
local ok, err

ok, err = pcall(function() DAEMON.journal = require("daemons.journal_d") end)
if not ok then log("warn", "Failed to load journal_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.audit   = require("daemons.audit_d") end)
if not ok then log("warn", "Failed to load audit_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.ticker  = require("daemons.ticker_d") end)
if not ok then log("warn", "Failed to load ticker_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.event   = require("daemons.event_d") end)
if not ok then log("warn", "Failed to load event_d daemon: " .. tostring(err)) end

-- The state cache underpins cooldowns, effects and combat, so it loads before
-- any of them: each declares its namespaces at require time.
ok, err = pcall(function() DAEMON.cache   = require("daemons.cache_d") end)
if not ok then log("warn", "Failed to load cache_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.cooldown = require("daemons.cooldown_d") end)
if not ok then log("warn", "Failed to load cooldown_d daemon: " .. tostring(err)) end

-- Traits before effects: effect_d refuses a modifier aimed at a gauge, and it
-- needs the trait definitions loaded to know which ones those are.
ok, err = pcall(function() DAEMON.trait   = require("daemons.trait_d") end)
if not ok then log("warn", "Failed to load trait_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.effect  = require("daemons.effect_d") end)
if not ok then log("warn", "Failed to load effect_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.mobs    = require("daemons.mob_d") end)
if not ok then log("warn", "Failed to load mob_d daemon: " .. tostring(err)) end

-- After `mob_d`, because it spawns from templates, and after `ticker_d`, which
-- it registers one heartbeat with at load. It must exist **before the world is
-- registered**: `world_d.register_room` tells it about each room as that room
-- arrives, so a spawner_d loaded afterwards would have missed every room in
-- every area and would sit there with an empty index.
ok, err = pcall(function() DAEMON.spawner = require("daemons.spawner_d") end)
if not ok then log("warn", "Failed to load spawner_d daemon: " .. tostring(err)) end

-- Before combat and abilities, both of which register or read a track. It needs
-- only cache, cooldown, trait and ticker at load; it reaches the other two
-- through DAEMON at tick time, which is after everything has loaded.
ok, err = pcall(function() DAEMON.queue   = require("daemons.queue_d") end)
if not ok then log("warn", "Failed to load queue_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.combat  = require("daemons.combat_d") end)
if not ok then log("warn", "Failed to load combat_d daemon: " .. tostring(err)) end

-- Last of the mechanics group, because it composes all of them: it spends
-- gauges through trait_d, applies effects, gates on cooldown_d, holds a cast in
-- ticker_d and cache_d, resolves a target through mob_d and world_d, and starts
-- a fight through combat_d. Nothing reaches back the other way.
ok, err = pcall(function() DAEMON.ability = require("daemons.ability_d") end)
if not ok then log("warn", "Failed to load ability_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.prompt  = require("daemons.prompt_d") end)
if not ok then log("warn", "Failed to load prompt_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.channel = require("daemons.channel_d") end)
if not ok then log("warn", "Failed to load channel_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.death   = require("daemons.death_d") end)
if not ok then log("warn", "Failed to load death_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.task    = require("daemons.task_d") end)
if not ok then log("warn", "Failed to load task_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.gmcp    = require("daemons.gmcp_d") end)
if not ok then log("warn", "Failed to load gmcp_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.pager   = require("daemons.pager_d") end)
if not ok then log("warn", "Failed to load pager_d daemon: " .. tostring(err)) end

-- The line editor. Beside the pager because they are the same shape and share
-- an interception point in `Commands.dispatch`.
ok, err = pcall(function() DAEMON.editor  = require("daemons.editor_d") end)
if not ok then log("warn", "Failed to load editor_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.snoop   = require("daemons.snoop_d") end)
if not ok then log("warn", "Failed to load snoop_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.room    = require("daemons.room_d") end)
if not ok then log("warn", "Failed to load room_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.character = require("daemons.character_d") end)
if not ok then log("warn", "Failed to load character_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.world   = require("daemons.world_d") end)
if not ok then log("warn", "Failed to load world_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.codegen = require("daemons.codegen_d") end)
if not ok then log("warn", "Failed to load codegen_d daemon: " .. tostring(err)) end

-- The content linter, after codegen because it reads areas through it.
ok, err = pcall(function() DAEMON.verify  = require("daemons.verify_d") end)
if not ok then log("warn", "Failed to load verify_d daemon: " .. tostring(err)) end

-- Adoption, which is the linter plus a flag flip and so comes after it.
ok, err = pcall(function() DAEMON.adopt   = require("daemons.adopt_d") end)
if not ok then log("warn", "Failed to load adopt_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.olc     = require("daemons.olc_d") end)
if not ok then log("warn", "Failed to load olc_d daemon: " .. tostring(err)) end

-- Where each session is standing in the file tree. Separate from olc_d because
-- `cd` outlives a build session — `olc done` should not throw your working
-- directory away.
ok, err = pcall(function() DAEMON.fs      = require("daemons.fs_d") end)
if not ok then log("warn", "Failed to load fs_d daemon: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.items   = require("daemons.item_d") end)
if not ok then log("warn", "Failed to load item_d daemon: " .. tostring(err)) end

-- The tag index, before anything that registers tagged things: `room_d` and
-- `mob_d` feed it as they load, and an index that starts existing halfway
-- through the world load has a hole in it exactly where nobody will look.
ok, err = pcall(function() DAEMON.tag     = require("daemons.tag_d") end)
if not ok then log("warn", "Failed to load tag_d daemon: " .. tostring(err)) end

-- Shops need items and tasks, both of which are already up.
ok, err = pcall(function() DAEMON.shop    = require("daemons.shop_d") end)
if not ok then log("warn", "Failed to load shop_d daemon: " .. tostring(err)) end

-- Creatures that attack on sight. It subscribes to `room.entered`,
-- `combat.started` and `character.left` **at load**, so everything those
-- handlers reach — mob_d, combat_d, world_d, character_d, ticker_d — has to be
-- up first. Nothing calls it; the events do.
ok, err = pcall(function() DAEMON.aggro   = require("daemons.aggro_d") end)
if not ok then log("warn", "Failed to load aggro_d daemon: " .. tostring(err)) end

-- The notice board. After `task_d`, which it asks for the expiry sweep, and
-- after `audit_d`, which it writes a removal to.
ok, err = pcall(function() DAEMON.board   = require("daemons.board_d") end)
if not ok then log("warn", "Failed to load board_d daemon: " .. tostring(err)) end

-- The quest engine. Last of the world group, because an objective names a room,
-- an item, a creature or a kill and it wants all four registries up.
--
-- Here rather than in the game layer because none of it is one game's: the
-- objective kinds, the three storage tiers and the reward shape are a schema,
-- and *which quests exist* is already a separate file the game registers.
ok, err = pcall(function() DAEMON.quest   = require("daemons.quest_d") end)
if not ok then log("warn", "Failed to load quest_d daemon: " .. tostring(err)) end

-- Ensure the first account is always admin (covers pre-existing databases)
if type(set_admin) == "function" then
    pcall(set_admin, 1, true)
end

-- ─── Global utility functions ────────────────────────────────────────────────
-- These are available to all Lua code (mudlib and game) without require().

--- Get the Player object for a session.
-- Wraps the session → character → Player lookup so area/game code never
-- needs to know about session infrastructure.
-- @param session_id string
-- @return Player|nil  The Player object, or nil if not logged in
function get_player(session_id)
    local session = get_session(session_id)
    if not session or not session.character_id then return nil end
    if DAEMON and DAEMON.character then
        return DAEMON.character.get(session.character_id)
    end
    return nil
end

-- ─── System Tasks ────────────────────────────────────────────────────────────
-- Tasks live in mudlib/tasks/ and are registered using ticker_d.
-- Intervals are pulled from server.toml via the config() efun.

-- Autosave — periodically save all loaded player data to prevent data loss
if DAEMON.ticker and DAEMON.character then
    local autosave_interval = config("game.autosave_seconds") or 300
    if autosave_interval > 0 then
        local autosave = require('tasks.autosave')
        DAEMON.ticker.every(autosave_interval, "system.autosave", autosave.run)
        log("info", "Autosave timer registered (every " .. autosave_interval .. "s)")
    else
        log("info", "Autosave disabled (autosave_seconds = 0)")
    end
end

-- Area reset — periodically reload area Lua and clear transient state
if DAEMON.ticker and DAEMON.world then
    local reset_interval = config("game.area_reset_seconds") or 900
    if reset_interval > 0 then
        local area_reset = require('tasks.area_reset')
        DAEMON.ticker.every(reset_interval, "system.area_reset", area_reset.run)
        log("info", "Area reset timer registered (every " .. reset_interval .. "s)")
    else
        log("info", "Area resets disabled (area_reset_seconds = 0)")
    end
end

-- ─── Command dispatcher ──────────────────────────────────────────────────────
local login    = require("login")
local commands = require("lib.commands")

--- Called when a new client connects (before authentication)
function on_connect(session_id)
    log("debug", "New connection: " .. session_id)
    set_session_state(session_id, "authenticating")
    login.greet(session_id)
end

--- Called when a player types a line of input
function on_input(session_id, text)
    local session = get_session(session_id)
    if not session then return end

    if session.state == "authenticating" then
        login.handle_input(session_id, text)
    elseif session.state == "playing" then
        commands.dispatch(session_id, text)
    end
end

--- Called when an off-thread password hash finishes.
-- Argon2 runs on a worker pool rather than the Lua thread, so `authenticate`
-- and `create_account` answer here instead of returning a value.
function on_auth_result(session_id, kind, account, err)
    login.on_result(session_id, kind, account, err)
end

--- Which character a disconnecting session was playing.
---
--- `get_session` is the obvious answer and **cannot be relied on here**: the
--- driver destroys the session record before it calls this hook — see
--- `driver.rs`, where `handler.disconnect` runs and only then is
--- `LuaCommand::OnDisconnect` queued — so by the time Lua asks, `get_session`
--- returns nil. Every cleanup step below hung off that lookup, which meant that
--- on an ordinary disconnect none of them ran: the character stayed loaded,
--- stayed standing in the room, and kept a Player pointing at a session id that
--- no longer existed.
---
--- The symptom is not local to the player who left. The next person to walk
--- into that room announces their arrival to everyone in it, `send_to_room`
--- resolves the ghost through `character_d.get`, and `player:send` calls the
--- `send` efun with the dead id — so *their* movement command raises "Session
--- not found" for a session they never had.
---
--- Nor is the ordering in the driver something to swap and be done with:
--- `OnDisconnect` crosses a channel to the Lua thread, so sending it before the
--- session is destroyed makes the race narrower rather than closing it. The
--- fallback belongs here, where the answer is known for certain.
---
--- The Player already knows which session it belongs to: `session_id` is
--- stamped on it when the character enters. That makes the loaded set an index
--- we get for free rather than a second mapping to keep in step, and it is
--- self-correcting — a Player that was never entered has no session id and
--- cannot be matched by accident.
---
--- `get_session` is still tried first, and still through `pcall`: it *raises*
--- on a malformed id rather than returning nil, and it remains the cheaper and
--- more direct answer on any driver that keeps the record long enough.
--- @param session_id string
--- @return number|nil char_id
local function disconnecting_character(session_id)
    local got, session = pcall(get_session, session_id)
    if got and type(session) == "table" and session.character_id then
        return session.character_id
    end

    if not (DAEMON and DAEMON.character and DAEMON.character.all_loaded) then return nil end
    local ok, loaded = pcall(DAEMON.character.all_loaded)
    if not ok or type(loaded) ~= "table" then return nil end

    for char_id, player in pairs(loaded) do
        if type(player) == "table" and player.session_id == session_id then
            return char_id
        end
    end
    return nil
end

--- Called when a client disconnects
function on_disconnect(session_id)
    log("debug", "Disconnected: " .. session_id)

    -- Save and unload character data, then remove from world.
    -- Each step is individually protected so a failure in one doesn't
    -- prevent cleanup in subsequent steps.
    local char_id = disconnecting_character(session_id)

    if char_id then

        -- The counterpart to `player.login`, emitted first so a listener sees
        -- the character while they still exist — after `character.unload` there
        -- is no Player left to ask anything about.
        if DAEMON and DAEMON.event then
            local ok, err = pcall(DAEMON.event.emit, "player.logout", {
                char_id = char_id, session_id = session_id,
            })
            if not ok then
                log("error", "a player.logout listener raised: " .. tostring(err))
            end
        end

        -- Remove from channel subscriber lists (in-memory only; saved list is preserved)
        if DAEMON and DAEMON.channel then
            local ok, err = pcall(DAEMON.channel.leave_all, char_id)
            if not ok then
                log("error", "Failed to clean up channel subscriptions for "
                    .. tostring(char_id) .. ": " .. tostring(err))
            end
        end

        -- Stop any fight they were in, so nothing keeps swinging at a
        -- character who is no longer here.
        if DAEMON and DAEMON.combat then
            local ok, err = pcall(DAEMON.combat.disengage_all, char_id)
            if not ok then
                log("error", "Failed to leave combat for "
                    .. tostring(char_id) .. ": " .. tostring(err))
            end
        end

        -- Drop anything they were in the middle of. Before `evict_owner`,
        -- because cancelling a cast may adjust a gauge and that wants flushing
        -- with the rest.
        if DAEMON and DAEMON.queue then
            local ok, err = pcall(DAEMON.queue.cleanup, char_id)
            if not ok then
                log("error", "Failed to clear the action queue for "
                    .. tostring(char_id) .. ": " .. tostring(err))
            end
        end

        if DAEMON and DAEMON.ability then
            local ok, err = pcall(DAEMON.ability.cleanup, char_id)
            if not ok then
                log("error", "Failed to clear abilities for "
                    .. tostring(char_id) .. ": " .. tostring(err))
            end
        end

        -- Write out everything the state cache is holding for this character.
        -- Before character_d.unload, which drops the Player object the effect
        -- scopes are keyed on.
        if DAEMON and DAEMON.cache then
            local ok, err = pcall(DAEMON.cache.evict_owner, char_id)
            if not ok then
                log("error", "Failed to flush cached state for "
                    .. tostring(char_id) .. ": " .. tostring(err))
                if DAEMON.journal then
                    DAEMON.journal.error("CACHE_D flush failed on disconnect for char "
                        .. tostring(char_id) .. ": " .. tostring(err))
                end
            end
        end

        -- Take their items out of the world index. **After** the save below,
        -- not before: `to_save` folds a container's contents onto its entry by
        -- reading them out of that index, so releasing first would write every
        -- backpack empty. Ordering here is the whole correctness argument, and
        -- it is why the release is a step of its own rather than part of
        -- `unload`.
        local function release_items()
            if not (DAEMON and DAEMON.items and DAEMON.character) then return end
            local ok, err = pcall(function()
                local player = DAEMON.character.get(char_id)
                if player then require('lib.carry').release(player) end
            end)
            if not ok then
                log("error", "Failed to release items for "
                    .. tostring(char_id) .. ": " .. tostring(err))
            end
        end

        -- Save persisted character data before cleanup
        if DAEMON and DAEMON.character then
            local ok, err = pcall(DAEMON.character.unload, char_id)
            if not ok then
                log("error", "Failed to unload character data for "
                    .. tostring(char_id) .. ": " .. tostring(err))
                if DAEMON.journal then
                    DAEMON.journal.error("CHARACTER_D unload failed on disconnect for char "
                        .. tostring(char_id) .. ": " .. tostring(err))
                end
            end
        end

        -- Now that the save has read them, the instances can go.
        release_items()

        -- Remove character from the world
        if DAEMON and DAEMON.world then
            local ok, err = pcall(DAEMON.world.remove_character, char_id)
            if not ok then
                log("error", "Failed to remove character "
                    .. tostring(char_id) .. " from world: " .. tostring(err))
            end
        end
    end

    -- Forget what the client said it supported. Session ids are not reused, so
    -- a table keyed on them grows for the life of the process otherwise — the
    -- same shape as every other per-session table cleaned up here.
    if DAEMON and DAEMON.gmcp and DAEMON.gmcp.forget then
        local ok, err = pcall(DAEMON.gmcp.forget, session_id)
        if not ok then
            log("error", "Failed to clear GMCP support list: " .. tostring(err))
        end
    end

    -- Clean up OLC session if active
    if DAEMON and DAEMON.olc then
        local ok, err = pcall(DAEMON.olc.cleanup, session_id)
        if not ok then
            log("error", "Failed to cleanup OLC session: " .. tostring(err))
        end
    end

    -- Clean up the shell's working directory
    if DAEMON and DAEMON.fs then
        local ok, err = pcall(DAEMON.fs.cleanup, session_id)
        if not ok then
            log("error", "Failed to cleanup fs session: " .. tostring(err))
        end
    end

    -- Clean up snoop relationships
    if DAEMON and DAEMON.snoop then
        local ok, err = pcall(DAEMON.snoop.cleanup, session_id)
        if not ok then
            log("error", "Failed to cleanup snoop session: " .. tostring(err))
        end
    end

    -- Clean up any open editor. Left open, every subsequent line from this
    -- session would be buffered as prose into a buffer nobody will ever save.
    if DAEMON and DAEMON.editor then
        local ok, err = pcall(DAEMON.editor.cleanup, session_id)
        if not ok then
            log("error", "Failed to cleanup editor session: " .. tostring(err))
        end
    end

    -- Clean up pager state
    if DAEMON and DAEMON.pager then
        local ok, err = pcall(DAEMON.pager.stop, session_id)
        if not ok then
            log("error", "Failed to cleanup pager session: " .. tostring(err))
        end
    end

    login.cleanup(session_id)
end

--- Called when a GMCP message is received
--- A GMCP message arrived from a client.
---
--- This used to log the package name and return, so a client could negotiate
--- GMCP, announce what it supports and send `Core.Hello`, and the game never
--- read a word of it. Dispatched by `gmcp_d` now, which knows the standard
--- packages and lets a game register its own.
function on_gmcp(session_id, package, data)
    if not (DAEMON and DAEMON.gmcp) then
        log("debug", "GMCP from " .. tostring(session_id) .. ": " .. tostring(package))
        return
    end
    local ok, err = pcall(DAEMON.gmcp.receive, session_id, package, data)
    if not ok then
        log("error", "on_gmcp: dispatch failed for '" .. tostring(package)
            .. "': " .. tostring(err))
        if DAEMON.journal then
            pcall(DAEMON.journal.error, "GMCP dispatch failed for '"
                .. tostring(package) .. "': " .. tostring(err))
        end
    end
end

--- Called once by the driver before the Lua VM stops, on a clean shutdown.
-- The last chance to write anything held in memory: CHARACTER_D is a cache
-- that only reaches the database on an autosave tick or a disconnect, so
-- without this every restart discards up to autosave_seconds of progress for
-- everyone still online.
--
-- The driver waits for this to return, bounded by game.shutdown_timeout_seconds
-- — so it must finish, and every step is protected so one failure cannot skip
-- the rest.
function on_shutdown()
    log("info", "Shutdown: flushing game state")
    if DAEMON and DAEMON.journal then
        pcall(DAEMON.journal.info, "Server shutting down — flushing game state")
    end

    -- Everything the state cache is holding — effects, cooldowns, counters.
    -- First, because it is the cheapest of the two and there is a deadline.
    if DAEMON and DAEMON.cache then
        local ok, flushed = pcall(DAEMON.cache.flush_all, { reason = "shutdown" })
        if not ok then
            log("error", "Shutdown cache flush failed: " .. tostring(flushed))
            if DAEMON.journal then
                pcall(DAEMON.journal.error, "SHUTDOWN: cache flush failed: " .. tostring(flushed))
            end
        else
            log("info", "Shutdown: flushed " .. tostring(flushed) .. " cached scope(s)")
        end
    end

    -- Save every loaded character. Same task the autosave ticker runs, so
    -- there is one definition of "what needs writing".
    if DAEMON and DAEMON.character then
        local ok, err = pcall(function() require('tasks.autosave').run() end)
        if not ok then
            log("error", "Shutdown autosave failed: " .. tostring(err))
            if DAEMON.journal then
                pcall(DAEMON.journal.error, "SHUTDOWN: autosave failed: " .. tostring(err))
            end
        end
    end

    log("info", "Shutdown: flush complete")
end

--- Called when a driver-side timer fires
function on_timer(id)
    if DAEMON and DAEMON.ticker then
        DAEMON.ticker.fire(id)
    end
end

--- Called when a `compute()` job finishes, whatever it finished with.
---
--- Exactly one of these fires for every job that `compute` returned an id for,
--- and none for a job it refused — so this is the *only* place a job's result
--- can arrive, and dispatch has to be complete.
---
--- Dispatched by handler list rather than by a `if tag matches` chain: whoever
--- submitted the job knows what to do with the answer, and the mudlib does not
--- and should not. `COMPUTE_HANDLERS` is an array of functions returning true
--- when they have claimed a result.
COMPUTE_HANDLERS = COMPUTE_HANDLERS or {}

function on_compute_result(id, ok, value, err, meta)
    meta = meta or {}

    for _, handler in ipairs(COMPUTE_HANDLERS) do
        local handled, claimed = pcall(handler, id, ok, value, err, meta)
        if not handled then
            log("error", "on_compute_result: a handler raised: " .. tostring(claimed))
            if DAEMON and DAEMON.journal then
                pcall(DAEMON.journal.error,
                    "COMPUTE: handler for job " .. tostring(id) .. " raised: " .. tostring(claimed))
            end
        elseif claimed then
            return
        end
    end

    -- Nobody claimed it. Worth saying out loud: a result with no reader is a
    -- job somebody submitted and then stopped caring about, which is either a
    -- bug or a handler that was hot-reloaded out from under it.
    log("warn", "on_compute_result: nothing claimed job " .. tostring(id)
        .. " (" .. tostring(meta.kind) .. ")")
end

--- Called before a module is hot-reloaded
function on_unload(module_name)
    log("info", "Unloading module: " .. module_name)
    if DAEMON.journal then
        DAEMON.journal.info("Module unloading: " .. module_name)
    end
end

--- Called after a module is hot-reloaded
function on_load(module_name)
    log("info", "Loaded module: " .. module_name)
    if DAEMON.journal then
        DAEMON.journal.info("Module reloaded: " .. module_name)
    end

    -- Re-bind DAEMON references so they point to the newly loaded module.
    -- The hot-reload system updates package.loaded, but DAEMON.x still
    -- points to the old table unless we reassign here.
    local daemon_map = {
        ["daemons.journal_d"]    = "journal",
        ["daemons.audit_d"]      = "audit",
        ["daemons.ticker_d"]     = "ticker",
        ["daemons.event_d"]      = "event",
        ["daemons.prompt_d"]     = "prompt",
        ["daemons.channel_d"]    = "channel",
        ["daemons.death_d"]      = "death",
        ["daemons.task_d"]       = "task",
        ["daemons.gmcp_d"]       = "gmcp",
        ["daemons.pager_d"]      = "pager",
        ["daemons.editor_d"]     = "editor",
        ["daemons.snoop_d"]      = "snoop",
        ["daemons.room_d"]       = "room",
        ["daemons.character_d"]  = "character",
        ["daemons.world_d"]      = "world",
        ["daemons.codegen_d"]    = "codegen",
        ["daemons.verify_d"]     = "verify",
        ["daemons.adopt_d"]      = "adopt",
        ["daemons.olc_d"]        = "olc",
        ["daemons.fs_d"]         = "fs",
        ["daemons.item_d"]       = "items",
        ["daemons.cache_d"]      = "cache",
        ["daemons.cooldown_d"]   = "cooldown",
        ["daemons.trait_d"]      = "trait",
        ["daemons.effect_d"]     = "effect",
        ["daemons.mob_d"]        = "mobs",
        ["daemons.spawner_d"]    = "spawner",
        ["daemons.combat_d"]     = "combat",
        ["daemons.ability_d"]    = "ability",
        ["daemons.queue_d"]      = "queue",
        ["daemons.quest_d"]      = "quest",
        ["daemons.board_d"]      = "board",
        ["daemons.aggro_d"]      = "aggro",
    }

    -- Convert slash-separated path to dot-separated require path
    local require_path = module_name:gsub("/", "."):gsub("\\", ".")
    local key = daemon_map[require_path]
    if key then
        local loaded = package.loaded[require_path]
        if loaded then
            DAEMON[key] = loaded
            log("info", "Re-bound DAEMON." .. key .. " after reload")
            -- Trait values are memoized against the definition generation, and
            -- a reloaded trait_d or effect_d is a new generation. Without this
            -- every online character would keep answering with values computed
            -- by the code that was just replaced.
            if (key == "trait" or key == "effect") and DAEMON.trait then
                pcall(DAEMON.trait.bump_all)
            end
        end
    end

    -- Flush command cache so reloaded command modules are picked up
    local commands = package.loaded["lib.commands"]
    if commands and commands.flush_cache then
        commands.flush_cache()
    end

    -- And the component index, for the same reason: a component added or
    -- edited during a session should describe itself without a restart.
    local components = package.loaded["components"]
    if components and components.flush_cache then
        components.flush_cache()
    end

    -- The schema index, for the same reason: an edited `schema/room.lua` should
    -- take effect on reload rather than at the next restart.
    local schemas = package.loaded["schema"]
    if schemas and schemas.flush_cache then
        schemas.flush_cache()
    end

    -- And the prototype index. `areaload` flushes this too, so an `areas reset`
    -- re-reads prototypes without help; this is for `reload` on the prototype
    -- file itself, where nothing else would notice.
    local prototypes = package.loaded["prototypes"]
    if prototypes and prototypes.flush_cache then
        prototypes.flush_cache()
    end

    -- And the body index, so an edited layout takes effect on reload.
    local bodies = package.loaded["body"]
    if bodies and bodies.flush_cache then
        bodies.flush_cache()
    end
end
