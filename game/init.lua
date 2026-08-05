-- game/init.lua — Game content layer entry point
-- Loaded by the engine after mudlib/init.lua.
-- Registers game-specific areas. All infrastructure (daemons, commands,
-- tasks, libraries) lives in mudlib/. This file handles authored content only.

local ok, err

-- ─── Attributes and effects ──────────────────────────────────────────────────
-- Traits before effects: effect_d refuses a modifier aimed at a gauge or a
-- counter, and it needs the trait definitions to know which those are.
--
-- `seal()` works out the evaluation order and reports anything broken — a
-- dependency on a trait that does not exist, or a cycle, named as a path. A
-- broken trait answers with its default rather than taking the server down.

if DAEMON.trait then
    ok, err = pcall(function()
        DAEMON.trait.define_all(require('traits.core'))
        -- Skills are traits in no seed set, so registering them costs a
        -- character nothing until one is learned. Sealing once, after both
        -- files, so the topological order is worked out a single time.
        DAEMON.trait.define_all(require('traits.skills'))
        if not DAEMON.trait.seal() then
            log("warn", "Some traits are broken — see the journal; they will use their defaults")
        end
    end)
    if not ok then log("error", "Failed to load traits: " .. tostring(err)) end
else
    log("warn", "trait_d is not loaded — this game has no attributes")
end

if DAEMON.effect then
    ok, err = pcall(function()
        DAEMON.effect.define_all(require('effects.core'))
        -- What the marsh does to you: a damage-over-time on the shared
        -- heartbeat, a `condition` over `lib/checks.lua` predicates, and a
        -- curse that survives dying.
        DAEMON.effect.define_all(require('effects.marsh'))
        DAEMON.effect.define_all(require('effects.mine'))
    end)
    if not ok then log("error", "Failed to load effects: " .. tostring(err)) end
end

-- ─── Game daemons ────────────────────────────────────────────────────────────
-- Systems that are *this game's* policy rather than the driver's. Each is
-- wrapped so a broken one does not take the rest of the layer down, the same
-- way the mudlib loads its own.
--
-- `aggro_d` is the clearest case for why the split exists: the driver ships
-- `Mobile.aggressive` and the `room.entered` event, and takes no position on
-- whether an aggressive creature attacks, how long it waits, or whether it
-- cares about level. That is content, so it lives here.

ok, err = pcall(function() DAEMON.aggro = require('daemons.aggro_d') end)
if not ok then log("error", "Failed to load aggro_d: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.board = require('daemons.board_d') end)
if not ok then log("error", "Failed to load board_d: " .. tostring(err)) end

-- After `tag_d` exists, which it does by now: the weather tick asks the tag
-- index which rooms are outdoors rather than walking the world every time.
ok, err = pcall(function() DAEMON.weather = require('daemons.weather_d') end)
if not ok then log("error", "Failed to load weather_d: " .. tostring(err)) end

-- Experience into levels. The mudlib owns `award_xp` and the `xp_gained`
-- pipeline; the *curve* is this game's, so it listens to `player.xp_gained`
-- exactly as `aggro_d` listens to `room.entered`.
ok, err = pcall(function() DAEMON.level = require('daemons.level_d') end)
if not ok then log("error", "Failed to load level_d: " .. tostring(err)) end

ok, err = pcall(function() DAEMON.quest = require('daemons.quest_d') end)
if not ok then log("error", "Failed to load quest_d: " .. tostring(err)) end

-- The virtual provider for the drowned reach. After `world_d`, obviously, and
-- it registers itself on load.
ok, err = pcall(function() DAEMON.reach = require('daemons.reach_d') end)
if not ok then log("error", "Failed to load reach_d: " .. tostring(err)) end

ok, err = pcall(function()
    DAEMON.spell = require('daemons.spell_d')
    DAEMON.spell.register_all(require('spells.core'))
end)
if not ok then log("error", "Failed to load spell_d: " .. tostring(err)) end

-- This game's own GMCP packages. **After `quest_d`**, which it reads, and after
-- `gmcp_d`, whose dispatcher it registers with. `Game.Quest` is not a
-- convention any client knows — it is this game's, which is why it is here and
-- not in the mudlib.
ok, err = pcall(function() DAEMON.gmcp_game = require('daemons.gmcp_game_d') end)
if not ok then log("error", "Failed to load gmcp_game_d: " .. tostring(err)) end

-- `navigate` submits compute jobs and has to be told when they come back. The
-- mudlib's `on_compute_result` dispatches through this list rather than
-- knowing what a route is — whoever submitted a job knows what to do with the
-- answer, and the driver does not.
ok, err = pcall(function()
    local navigate = require('cmds.navigate')
    COMPUTE_HANDLERS = COMPUTE_HANDLERS or {}
    COMPUTE_HANDLERS[#COMPUTE_HANDLERS + 1] = navigate.on_result
end)
if not ok then log("error", "Failed to register the navigate handler: " .. tostring(err)) end

-- ─── Roles ───────────────────────────────────────────────────────────────────
-- Which roles exist is policy, so it is content. Idempotent by construction —
-- `create_role` on a role that exists is a no-op and so is a repeated grant —
-- which is what lets the roles be *declared in a file* rather than provisioned
-- by a migration nobody remembers to run.

ok, err = pcall(function() require('setup_roles').apply() end)
if not ok then log("error", "Failed to set up roles: " .. tostring(err)) end

-- ─── Channels ────────────────────────────────────────────────────────────────
-- Which channels exist is content. `chat` and `newbie` are open; `staff` is
-- gated by a permission, which is what makes the channel-name shortcut worth
-- testing — `staff hello` has to refuse for a player and work for staff.

if DAEMON.channel then
    ok, err = pcall(function()
        DAEMON.channel.create("chat",   { title = "Chat",   colour = "cyan" })
        DAEMON.channel.create("newbie", { title = "Newbie", colour = "green" })
        DAEMON.channel.create("trade",  { title = "Trade",  colour = "yellow" })
        DAEMON.channel.create("staff",  { title = "Staff",  colour = "red",
                                          permission = "channel.staff" })
    end)
    if not ok then log("error", "Failed to create channels: " .. tostring(err)) end
end

-- ─── Areas ───────────────────────────────────────────────────────────────────
-- Each area lives in its own subdirectory under game/areas/.
-- Area files return plain data tables. ROOM_D.load_area() processes them
-- into Room objects, then world_d registers them and records the source
-- for later resets.

if DAEMON.world and DAEMON.room then
    -- wizard_workshop
    ok, err = pcall(function()
        -- Load items first so they're available if rooms reference them
        if DAEMON.items then
            local ww_items = require('areas.wizard_workshop.items')
            DAEMON.items.register_all(ww_items)
            -- Weapons, armour and containers. Separate from `items.lua`
            -- because that file is the workshop's puzzle and this is the
            -- gear that makes the equipment half of the object model real.
            DAEMON.items.register_all(require('areas.wizard_workshop.gear'))
        end

        local area_data = require('areas.wizard_workshop.rooms')
        local rooms = DAEMON.room.load_area(area_data)
        DAEMON.world.register_area(rooms)
        DAEMON.world.register_area_source(
            "wizard_workshop",
            "areas.wizard_workshop.rooms",
            "areas.wizard_workshop.items"
        )

        -- Creature *templates*. Populating happens once at the end, after
        -- every area has registered its rooms — a template whose `spawn_room`
        -- is in an area that has not loaded yet cannot be spawned.
        if DAEMON.mobs then
            DAEMON.mobs.register_all(require('areas.wizard_workshop.mobs'))
        end
    end)
    if not ok then
        log("error", "Failed to load area 'wizard_workshop': " .. tostring(err))
    end

    -- thornhollow — one area across three room files, joined by ROOM_D.merge.
    ok, err = pcall(function()
        if DAEMON.items then
            DAEMON.items.register_all(require('areas.thornhollow.items'))
        end

        local rooms = DAEMON.room.load_area(require('areas.thornhollow.init'))
        DAEMON.world.register_area(rooms)
        DAEMON.world.register_area_source(
            "thornhollow",
            "areas.thornhollow.init",
            "areas.thornhollow.items"
        )

        if DAEMON.mobs then
            DAEMON.mobs.register_all(require('areas.thornhollow.mobs'))
        end

        -- Shops after the rooms they stand in: `register` indexes by room, and
        -- a shop pointing at a room that does not exist yet is a shop nobody
        -- can find and no error anywhere.
        if DAEMON.shop then
            DAEMON.shop.register_all(require('areas.thornhollow.shops'))
        end

        -- The town strongbox is an *instance* in a room rather than a template
        -- in a registry: a particular chest with particular contents, not the
        -- idea of a chest. Idempotent, because an area reset re-runs this.
        if DAEMON.items then
            local vault_room = DAEMON.items.location("room", "thornhollow.undercroft")
            local already = false
            for _, entry in ipairs(DAEMON.items.in_room("thornhollow.undercroft")) do
                if entry.template == "vault_chest" then already = true break end
            end
            if not already then
                DAEMON.items.spawn("vault_chest", vault_room)
            end
        end
    end)
    if not ok then
        log("error", "Failed to load area 'thornhollow': " .. tostring(err))
    end

    -- greywater_marsh — lfun descriptions keyed on the weather, aggressive
    -- creatures, and the durable herb cooldown.
    ok, err = pcall(function()
        local rooms = DAEMON.room.load_area(require('areas.greywater_marsh.rooms'))
        DAEMON.world.register_area(rooms)
        DAEMON.world.register_area_source(
            "greywater_marsh",
            "areas.greywater_marsh.rooms"
        )
        if DAEMON.mobs then
            DAEMON.mobs.register_all(require('areas.greywater_marsh.mobs'))
        end
    end)
    if not ok then
        log("error", "Failed to load area 'greywater_marsh': " .. tostring(err))
    end

    -- collapsed_mine — dark rooms, a locked door, a lever puzzle and the boss.
    ok, err = pcall(function()
        if DAEMON.items then
            DAEMON.items.register_all(require('areas.collapsed_mine.items'))
        end
        local rooms = DAEMON.room.load_area(require('areas.collapsed_mine.rooms'))
        DAEMON.world.register_area(rooms)
        DAEMON.world.register_area_source(
            "collapsed_mine",
            "areas.collapsed_mine.rooms",
            "areas.collapsed_mine.items"
        )
        if DAEMON.mobs then
            DAEMON.mobs.register_all(require('areas.collapsed_mine.mobs'))
        end
    end)
    if not ok then
        log("error", "Failed to load area 'collapsed_mine': " .. tostring(err))
    end

    -- Quests after the rooms, creatures and items they name: a `visit`
    -- objective naming a room that does not exist is a quest nobody can finish
    -- and no error anywhere.
    if DAEMON.quest then
        ok, err = pcall(function()
            DAEMON.quest.register_all(require('quests.thornhollow'))
        end)
        if not ok then log("error", "Failed to register quests: " .. tostring(err)) end
    end

    -- Creatures last, once every room in every area exists for them to stand
    -- in. `populate` is idempotent — a template already at its `count` is left
    -- alone — so an area reset calls it again without the world filling up.
    if DAEMON.mobs then
        ok, err = pcall(DAEMON.mobs.populate)
        if not ok then log("error", "Failed to populate mobs: " .. tostring(err)) end
    end
else
    log("error", "Cannot register areas: world_d or room_d daemon failed to load.")
end

log("info", "Game world loaded successfully.")
