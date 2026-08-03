-- game/areas/greywater_marsh/rooms.lua — The drowned marsh west of town.
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

local WEATHER = "greywater_marsh"

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
-- `task_list.md`'s original bug in one place: a "once per 24 hours" gate stored
-- as room object state was really "once per 15 minutes", because an area reset
-- wipes object state. Per-character state does not belong on a room. This uses
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

return {
    _meta = {
        name   = "greywater_marsh",
        title  = "Greywater Marsh",
        author = "Oxigeon",
        level  = "3-12",
        status = "live",
    },

    {
        id    = "greywater_marsh.causeway_head",
        short = "The Head of the Causeway",
        light = 3,
        tags  = { "outdoor", "marsh" },
        smell = "Standing water and rot, and under it something mineral.",
        sound = function() return fogbound()
            and "Nothing. The fog takes the sound off the water."
            or "Reeds, and water moving in more than one direction." end,

        description = function(room)
            local base = [[
The laid stone begins here and runs west in a line so straight it is obviously
older than the town. On both sides the reeds start immediately and go on until
they stop being reeds and start being water. Somebody has stacked cairns at
intervals, and somebody else has knocked several of them over.]]
            if fogbound() then
                base = base .. "\r\n\r\nThe next cairn is a suggestion. The one after it is not there."
            end
            return base .. sky()
        end,

        exits = {
            east = "thornhollow.west_gate",
            west = "greywater_marsh.causeway_mid",
        },

        items = {
            cairns = "Stacked flat stones, waist high. Several have been pushed "
                  .. "over, which took some doing.",
            reeds  = "Grey-green and taller than a person. They move when there "
                  .. "is no wind, which the townsfolk say is fish.",
        },

        actions = {
            wade = { func = leave_the_stone, hint = "wade off the causeway" },
        },
    },

    {
        id    = "greywater_marsh.causeway_mid",
        short = "The Causeway",
        light = 2,
        tags  = { "outdoor", "marsh" },
        smell = "Rot, closer now.",
        sound = "Water under the stone, which should not be possible.",

        description = function(room)
            return [[
Halfway. The stone here is wetter and the joints have opened, so there is water
running *under* the causeway as well as beside it. A drowned willow leans out
over the north side, its roots holding a raft of matter nobody wants to look at
closely.]] .. sky()
        end,

        exits = {
            east  = "greywater_marsh.causeway_head",
            west  = "greywater_marsh.stilt_village",
            north = "greywater_marsh.herb_beds",
        },

        items = {
            willow = "A drowned willow, dead for years and still standing, "
                  .. "holding up a mat of reed and worse.",
            joints = "The stone joints have opened. Water goes under here.",
        },

        actions = {
            wade = { func = leave_the_stone, hint = "wade off the causeway" },
        },
    },

    {
        id    = "greywater_marsh.herb_beds",
        short = "The Herb Beds",
        light = 2,
        tags  = { "outdoor", "marsh", "resource" },
        smell = "Green, and sharply medicinal.",
        sound = "Insects, in a marsh that has almost nothing else living in it.",

        description = function(room)
            local base = [[
A shelf of firmer ground where the marshroot grows, which is the only reason
anybody comes off the stone on purpose. The apothecary's people have cut a
narrow path in and marked it with stakes; the stakes have been re-driven often
enough that several are new.]]
            return base .. sky()
        end,

        exits = { south = "greywater_marsh.causeway_mid" },

        items = {
            stakes = "Marker stakes, several of them new. Whatever moves them "
                  .. "does not take them away.",
            root   = "Pale forked marshroot, growing in the shallows. Warm to "
                  .. "the touch, which nobody has explained.",
        },

        actions = {
            gather = { func = gather_herbs, hint = "gather marshroot" },
        },
    },

    {
        id    = "greywater_marsh.stilt_village",
        short = "The Stilt Village",
        light = 1,
        tags  = { "outdoor", "marsh", "ruin" },
        smell = "Wet timber and old smoke.",
        sound = "Timber working against timber, all around, at no particular rate.",

        description = function(room)
            local base = [[
Eleven houses on piles, and the water is up to the floor of nine of them. The
walkways between are gone except for their posts. Nothing was taken when this
was left — there are still pots on the shelves — which is the part that stops
people coming out here twice.]]
            if fogbound() then
                base = base .. "\r\n\r\nIn the fog the houses are shapes, and the shapes are not all houses."
            end
            return base .. sky()
        end,

        exits = {
            east = "greywater_marsh.causeway_mid",
            west = "greywater_marsh.deep_water",
        },

        items = {
            houses = "Eleven houses on oak piles. The doors are open and the "
                  .. "shelves are not empty.",
            pots   = "Cooking pots, still on their shelves, with the handles "
                  .. "worn where somebody held them.",
        },
    },

    {
        id    = "greywater_marsh.deep_water",
        short = "The Deep Water",
        light = 1,
        tags  = { "outdoor", "marsh", "dangerous" },
        smell = "Cold. Only cold.",
        sound = "Nothing above the water at all.",

        description = function(room)
            return [[
The reeds stop. Past the last of the stilt houses the bottom goes away and the
water turns from grey to a green so dark it reads as black. Something has laid a
line of pale stones out into it, and the line goes further than the light does.]] .. sky()
        end,

        exits = { east = "greywater_marsh.stilt_village" },

        items = {
            stones = "Pale flat stones, laid in a line out into the deep water. "
                  .. "They are laid, not fallen.",
        },
    },
}
