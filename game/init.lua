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
-- `reach_d` is the clearest case for why the split exists: it names a room id,
-- an area and a hardcoded table of prose. Nothing about it would mean anything
-- in another game. The test for this side of the line is whether a daemon
-- *names things* — `weather_d` names reeds and shutters, `gmcp_game_d` names a
-- package only this game has. `aggro_d`, `board_d` and `quest_d` named nothing,
-- and are in the mudlib.

-- After `tag_d` exists, which it does by now: the weather tick asks the tag
-- index which rooms are outdoors rather than walking the world every time.
ok, err = pcall(function() DAEMON.weather = require('daemons.weather_d') end)
if not ok then log("error", "Failed to load weather_d: " .. tostring(err)) end

-- Experience into levels. The mudlib owns `award_xp` and the `xp_gained`
-- pipeline; the *curve* is this game's — `THRESHOLDS` is a design document as
-- much as a table, and it is the whole reason this one stayed.
ok, err = pcall(function() DAEMON.level = require('daemons.level_d') end)
if not ok then log("error", "Failed to load level_d: " .. tostring(err)) end

-- The virtual provider for the drowned reach. After `world_d`, obviously, and
-- it registers itself on load.
ok, err = pcall(function() DAEMON.reach = require('daemons.reach_d') end)
if not ok then log("error", "Failed to load reach_d: " .. tostring(err)) end

-- ─── What a degree of success is worth ───────────────────────────────────────
--
-- `combat_d` computes `margin` on every swing and hands back a band. The mudlib
-- ships one band at power 1.0, so until a game says otherwise the margin is
-- computed and thrown away — which is what was happening here.
--
-- The margin is `threshold - roll`, **out of 100 and deliberately not divided
-- by the threshold**. So a 95%-to-hit attack that rolls 3 has margin 92 and is a
-- skill differential expressing itself; a 10%-to-hit attack that rolls 3 has
-- margin 7 and is luck. Luck gets a scrape. That is why the bands below are
-- absolute numbers rather than percentages of anything.
--
-- The floor band has to be `at = 0`, or a hit whose margin falls below every
-- band gets the last one by fallback and the table reads as if it did not.
if DAEMON.combat then
    ok, err = pcall(function()
        DAEMON.combat.define_degrees({
            { id = "graze",    at = 0,  power = 0.6 },
            { id = "hit",      at = 20, power = 1.0 },
            { id = "solid",    at = 45, power = 1.4 },
            -- Beat them by seventy and you chose where it landed. `combat_d`
            -- rerolls the location for this band, which is the difference
            -- between "hard" and "hard, in the throat".
            { id = "decisive", at = 70, power = 1.9, reroll_location = true },
        })
    end)
    if not ok then log("error", "Failed to define degrees: " .. tostring(err)) end
end

-- Abilities after traits and effects, because a spec names both: a `rank_trait`
-- has to exist to be present on anybody, and an `apply` names a definition.
-- Listed rather than discovered, matching `traits/` and `effects/` beside them —
-- the discovery argument in this file is about OLC-created *areas* being
-- invisible, and an ability file is code with functions in it, written by
-- whoever is already editing this one.
ok, err = pcall(function()
    DAEMON.ability.define_all(require('abilities.spells'))
    DAEMON.ability.define_all(require('abilities.techniques'))
end)
if not ok then log("error", "Failed to load abilities: " .. tostring(err)) end

-- After the abilities it projects. `spell_d` is now a vocabulary over
-- `ability_d`; `DAEMON.spell.cast` still works and still refuses the same way.
ok, err = pcall(function() DAEMON.spell = require('daemons.spell_d') end)
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

-- ─── The notice board ────────────────────────────────────────────────────────
-- The board is a mudlib mechanism; *what it is for* is content, the same way
-- the channels above are. A game that says nothing gets the defaults.

if DAEMON.board then
    ok, err = pcall(function()
        DAEMON.board.configure({
            categories = { "news", "trade", "help", "rp" },
            lifetime   = 14 * 24 * 3600,
        })
    end)
    if not ok then log("error", "Failed to configure the board: " .. tostring(err)) end
end

-- ─── Areas ───────────────────────────────────────────────────────────────────
-- Each area lives in its own subdirectory under game/areas/.
-- Area files return plain data tables. ROOM_D.load_area() processes them
-- into Room objects, then world_d registers them and records the source
-- for later resets.

if DAEMON.world and DAEMON.room then
    -- Areas are **discovered**, not listed.
    --
    -- This block used to name every one of them: a `pcall` per area, each
    -- requiring its rooms, items, mobs and shops by hand and then calling
    -- `register_area_source`. Two costs. An area OLC created was invisible until
    -- somebody edited this file — and OLC never called `register_area_source`
    -- at all, so `areas reset <new_area>` answered "No registered source" for
    -- every area it had ever made.
    --
    -- `areaload.load_all` runs in passes across all areas — items, then rooms,
    -- then mobs, then shops — which also removes a hazard this list had:
    -- `thornhollow.smithy` has a `down` exit into `collapsed_mine.adit`, and
    -- that worked only because the areas happened to be listed in the right
    -- order. See `mudlib/lib/areaload.lua`.
    local areaload = require('lib.areaload')

    local loaded, failures = areaload.load_all()
    for _, f in ipairs(failures) do
        log("error", "Failed to load area '" .. f.area .. "': " .. tostring(f.err))
        if DAEMON.journal then
            pcall(DAEMON.journal.error, "AREALOAD: " .. f.area .. ": " .. tostring(f.err))
        end
    end
    log("info", "Loaded " .. loaded .. " area(s) by discovery.")

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

    -- Spawners after `populate`, and for the same reason it is here at all: a
    -- world that has just loaded should be populated before the first player
    -- arrives rather than filling in over the next few heartbeats. `fill_all`
    -- counts what is already alive, so an area reset calls it again without the
    -- pantry ending up knee-deep in rats.
    if DAEMON.spawner then
        ok, err = pcall(DAEMON.spawner.fill_all)
        if not ok then log("error", "Failed to fill spawners: " .. tostring(err)) end
    end
else
    log("error", "Cannot register areas: world_d or room_d daemon failed to load.")
end

log("info", "Game world loaded successfully.")
