-- game/areas/greywater_marsh/custom.lua — hand-written. OLC never reads or
-- writes this file.
--
-- What this area exists to prove:
--
--   lfun descriptions   every room's prose asks `weather_d` what it is doing.
--                       Nothing pushes; the room asks when it is looked at, so
--                       there is no per-room state to keep in step.
--   light by weather    an outdoor room's level is its own minus what the sky
--                       is doing, so fog makes the marsh genuinely dark
--   the herb node       a **durable** cooldown, 24 hours, per character — and
--                       the point is that it survives an area reset, which is
--                       the bug that started all of this
--   damage types        the Wisp deals `magic`, which is what makes the warded
--                       cloak's resist table worth having
--
-- The prose, the exits, the scenery and the stat blocks are in `rooms.lua` and
-- `mobs.lua`, which OLC owns and regenerates wholesale. Everything below is a
-- function, which is precisely why it is here instead.

--- The weather line, or nothing when there is no weather daemon. Every
--- description ends with this, which is the whole lfun demonstration: the room
--- is a function of the world rather than a string that has to be rewritten.
local function sky()
    if not (DAEMON and DAEMON.weather) then return "" end
    local line = DAEMON.weather.effects().ambience
    return line and ("\r\n\r\n" .. line) or ""
end

--- Fog is the one that changes what you can *do* rather than only what you
--- read: at ten feet of visibility the causeway is a rumour.
local function fogbound()
    return DAEMON and DAEMON.weather and DAEMON.weather.current() == "fog"
end

-- ─── The herb node ───────────────────────────────────────────────────────────
--
-- The original bug in one place: a "once per 24 hours" gate stored as room
-- object state was really "once per 15 minutes", because an area reset wipes
-- object state. Per-character state does not belong on a room. This uses
-- `DAEMON.cooldown` with a 24-hour duration, which is over the durable
-- threshold and so is written through — it survives a reset *and* a restart.

local HERB_COOLDOWN = 24 * 3600

local function gather_herbs(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if not (DAEMON and DAEMON.cooldown) then
        player:send("You cannot find anything worth taking.")
        return
    end

    if not DAEMON.cooldown.ready(player.char_id, "greywater_herbs") then
        local left = DAEMON.cooldown.remaining(player.char_id, "greywater_herbs")
        player:send("The bed is picked over. Give it a day. ("
            .. math.ceil(left / 3600) .. "h)")
        return
    end

    DAEMON.cooldown.mark(player.char_id, "greywater_herbs", HERB_COOLDOWN)

    player:send_lines(
        "You go in to the elbow and come out with a fistful of pale forked root,",
        "warm to the touch and smelling of the bottom.")
    player:message_room(player.name .. " reaches into the water and pulls out a handful of root.")
    player:add_item("dried_marshroot")
    player:add_item("dried_marshroot")

    if DAEMON.trait and DAEMON.trait.has(player, "herbalism") then
        DAEMON.trait.adjust(player, "herbalism", 1)
    end
end

-- ─── The causeway ────────────────────────────────────────────────────────────

local function leave_the_stone(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    player:send_lines(
        "You put a foot off the causeway. The reed mat holds for a moment and",
        "then does not, and you are in to the chest in water the temperature of",
        "a cellar.")
    player:message_room(player.name .. " steps off the causeway and goes in.")

    -- The one place the marsh poisons you for a decision rather than for a
    -- fight. Applied through the ordinary effect path, so an antidote removes
    -- it and a resistance would blunt it.
    if DAEMON and DAEMON.effect then
        DAEMON.effect.apply(player, "marsh_poison", { source = "room:greywater" })
    end
    if DAEMON and DAEMON.trait then
        DAEMON.trait.adjust(player, "hp", -8)
    end
end

-- ─── Descriptions ────────────────────────────────────────────────────────────

local function causeway_head_description(room)
    local base = [[
The laid stone begins here and runs west in a line so straight it is obviously
older than the town. On both sides the reeds start immediately and go on until
they stop being reeds and start being water. Somebody has stacked cairns at
intervals, and somebody else has knocked several of them over.]]
    if fogbound() then
        base = base .. "\r\n\r\nThe next cairn is a suggestion. The one after it is not there."
    end
    return base .. sky()
end

local function causeway_mid_description(room)
return [[
Halfway. The stone here is wetter and the joints have opened, so there is water
running *under* the causeway as well as beside it. A drowned willow leans out
over the north side, its roots holding a raft of matter nobody wants to look at
closely.]] .. sky()
end

local function herb_beds_description(room)
local base = [[
A shelf of firmer ground where the marshroot grows, which is the only reason
anybody comes off the stone on purpose. The apothecary's people have cut a
narrow path in and marked it with stakes; the stakes have been re-driven often
enough that several are new.]]
return base .. sky()
end

local function stilt_village_description(room)
local base = [[
Eleven houses on piles, and the water is up to the floor of nine of them. The
walkways between are gone except for their posts. Nothing was taken when this
was left — there are still pots on the shelves — which is the part that stops
people coming out here twice.]]
if fogbound() then
    base = base .. "\r\n\r\nIn the fog the houses are shapes, and the shapes are not all houses."
end
return base .. sky()
end

local function deep_water_description(room)
return [[
The reeds stop. Past the last of the stilt houses the bottom goes away and the
water turns from grey to a green so dark it reads as black. Something has laid a
line of pale stones out into it, and the line goes further than the light does.]] .. sky()
end

-- ─── Creatures ───────────────────────────────────────────────────────────────

--- `survives_death`, so respawning does not clear it. A curse you can remove by
--- walking into a rat is not a curse.
local function wisp_mark_target(mob, target)
    if DAEMON and DAEMON.effect and math.random(100) <= 30 then
        DAEMON.effect.apply(target, "wisp_mark", {
            source = "mob:" .. tostring(mob.id),
        })
    end
end

-- `marsh_lurker`'s bite used to be declared here, and a near-identical copy of
-- it lived in the mine. It is now `marsh.venomous`'s `on_combat`, in
-- `game/prototypes/beasts.lua` — "this creature's attacks poison" is the
-- creature's business, and combat should not grow a special case per monster.

return {
    rooms = {
        ["greywater_marsh.causeway_head"] = {
            description = causeway_head_description,
            sound = function() return fogbound()
                and "Nothing. The fog takes the sound off the water."
                or "Reeds, and water moving in more than one direction." end,
            actions = {
                wade = { func = leave_the_stone, hint = "wade off the causeway" },
            },
        },
        ["greywater_marsh.causeway_mid"] = {
            description = causeway_mid_description,
            actions = {
                wade = { func = leave_the_stone, hint = "wade off the causeway" },
            },
        },
        ["greywater_marsh.herb_beds"] = {
            description = herb_beds_description,
            actions = {
                gather = { func = gather_herbs, hint = "gather marshroot" },
            },
        },
        ["greywater_marsh.stilt_village"] = {
            description = stilt_village_description,
        },
        ["greywater_marsh.deep_water"] = {
            description = deep_water_description,
        },
    },

    mobs = {
        ["greywater_wisp"] = {
            on_combat = wisp_mark_target,
        },
    },
}
