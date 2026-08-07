-- game/areas/thornhollow/custom.lua — the half of this area that is code.
--
-- OLC regenerates `rooms.lua`, `items.lua` and `mobs.lua` wholesale. This file
-- is the other side of that bargain: hand-written, never read or written by
-- OLC, and holding everything that cannot be expressed as data.
--
-- Patches are merged over the generated data **before** it is constructed, so a
-- patched `damage` reaches `weapon.from_data` rather than an already-built
-- component. See `mudlib/lib/patch.lua`.
--
-- ─── This area used to be four files ─────────────────────────────────────────
--
-- `init.lua` merged `square.lua`, `market.lua` and `undercroft.lua` with
-- `ROOM_D.merge`, which split the town by *place* so three builders could work
-- without touching each other's file. That is gone, and the reason is not
-- taste: `areaload.inspect` prefers `init.lua` over `rooms.lua`
-- unconditionally, so once the area is OLC-managed a generated `rooms.lua`
-- beside a surviving `init.lua` would never be read — every `olc save` writing
-- to a file the loader ignores. One `rooms.lua` is the price of being editable
-- in the game.
--
-- ─── Which fields split, and how ─────────────────────────────────────────────
--
--   `dialogue` is a schema `map`, so a patch **merges topic by topic**. Bellow's
--   five plain answers stay in `mobs.lua` as prose; only `sword`, which is a
--   function, is here.
--
--   `echoes` is a `record_array`, which a patch replaces **whole**. So the
--   apprentice's three echoes are all here, including the two that are plain
--   strings — splitting them is not available.

-- ─── The square and the undercroft ───────────────────────────────────────────

local function read_notices(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end
    player:send_lines(
        "The board is a slab of oak nailed to two posts, thick with layers of",
        "old parchment. Read it properly with the {cyan}board{/} command.")
end

local function drink_from_well(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    -- A cooldown rather than a room-state flag. Per-character state on a room
    -- is wiped by an area reset, which is how a "once a day" gate becomes
    -- "once every fifteen minutes" — see CLAUDE.md.
    if DAEMON and DAEMON.cooldown then
        if not DAEMON.cooldown.ready(player.char_id, "well_draught") then
            local left = DAEMON.cooldown.remaining(player.char_id, "well_draught")
            player:send("The water is cold and clear, but you have had your fill. ("
                .. math.ceil(left) .. "s)")
            return
        end
        DAEMON.cooldown.mark(player.char_id, "well_draught", 300)
    end

    player:send("You haul up the bucket and drink. The water tastes faintly of iron.")
    player:message_room(player.name .. " drinks from the well.")
    if DAEMON and DAEMON.trait then
        DAEMON.trait.adjust(player, "hp", 5)
    end
end

local function pry_flagstone(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    local ROOM = "thornhollow.crypt"
    if get_object_state(ROOM, "flagstone_lifted") then
        player:send("The flagstone is already up. The hole under it is empty.")
        return
    end

    -- A requirement met by a trait, not by an item: this is the one place in
    -- the town that cares how strong you are.
    if player:trait("strength") < 13 then
        player:send("You get your fingers under the edge and heave. It does not "
            .. "move, and your fingers say enough about that.")
        return
    end

    set_object_state(ROOM, "flagstone_lifted", true)
    player:send_lines(
        "The flagstone comes up with a sound like a held breath let go.",
        "Underneath, in a hollow the size of a hat, is a small brass key.")
    player:message_room(player.name .. " levers up a flagstone.")

    if DAEMON and DAEMON.items then
        DAEMON.items.spawn("brass_key", DAEMON.items.location("room", ROOM))
    end
end

-- ─── Item hooks, from items.lua ──────────────────────────────────────────────

local function lantern_toggle(item, char_id)
local player = DAEMON.character and DAEMON.character.get(char_id)
if not player then return end
-- The instance's id, not the template's: `item.id` on a resolved
-- pristine item *is* the template's, so the caller passes nothing
-- useful and this asks the inventory instead.
local Carry = require('lib.carry')
local entry = select(1, Carry.find(player, "lantern",
    { inventory = true, room = false, equipped = true }))
if not entry then return "You are not holding it." end

local lit = get_object_state(entry.id, "lit") == true
set_object_state(entry.id, "lit", not lit)
return lit and "You slide the hood shut. The light goes out."
    or "You open the hood. Warm yellow light fills the space around you."
end

local function healing_draught_drunk(item, player)
if DAEMON and DAEMON.trait then
    DAEMON.trait.adjust(player, "hp", 35)
end
end

local function antidote_drunk(item, player)
-- Named removal rather than a blanket clear: an antidote that also
-- stripped your blessings would be a trap wearing a helpful label.
if DAEMON and DAEMON.effect then
    local n = DAEMON.effect.remove(player, "marsh_poison", { reason = "antidote" })
    if n == 0 then
        player:send("Nothing in you objects to it. Perhaps you were not poisoned.")
    end
end
end

-- ─── Creature lfuns, from mobs.lua ───────────────────────────────────────────

--- Bellow sizes up what you are carrying before answering about swords.
local function smith_on_sword(mob, player)
    if player and player.equipment and player.equipment.weapon then
        return "Bellow glances at what you are carrying. \"That'll "
            .. "do for what's out there. Just about.\""
    end
    return "\"Unarmed, going west? That's one way to find out how "
        .. "deep it is.\""
end

--- An lfun echo: resolved through `Object.resolve` like any other property, so
--- it can read the world. What the apprentice is doing depends on the forge.
local function apprentice_at_the_forge(mob)
    if get_object_state("thornhollow.smithy", "forge_lit") then
        return "The apprentice bank the coals down and steps back "
            .. "from the heat."
    end
    return "The apprentice looks at the banked forge and sighs."
end

return {
    rooms = {
        ["thornhollow.square"] = {
            actions = {
                -- A room action that shadows nothing, and one that shadows a
                -- system command. Dispatch order puts room actions first, so
                -- `drink` here is the well rather than a potion — the
                -- precedence rule made visible rather than only documented.
                notices = { func = read_notices, hint = "read the notices" },
                drink   = { func = drink_from_well, hint = "drink from the well" },
            },
        },
        ["thornhollow.crypt"] = {
            actions = {
                pry = { func = pry_flagstone, hint = "pry the flagstone" },
            },
        },
    },

    items = {
        ["hooded_lantern"]  = { on_use   = lantern_toggle },
        ["healing_draught"] = { on_drink = healing_draught_drunk },
        ["marsh_antidote"]  = { on_drink = antidote_drunk },
    },

    mobs = {
        ["town_smith"] = {
            dialogue = { sword = smith_on_sword },
        },
        ["forge_apprentice"] = {
            echoes = {
                { text = "The apprentice pumps the bellows in a long, even rhythm.", weight = 4 },
                { text = "The apprentice drops something, and does not look up.", weight = 2 },
                { text = apprentice_at_the_forge, weight = 1 },
            },
        },
    },

    --- Anything this area needs doing once its data has loaded.
    ---
    --- Called last, and called again on every `areas reset`, so it has to be
    --- idempotent. That is not a style note: the reset path exists precisely to
    --- re-run this, and a version that spawned on every call would fill the
    --- undercroft with chests one reset at a time.
    on_load = function(area_name)
        -- The town strongbox is an *instance* in a room rather than a template
        -- in a registry: a particular chest with particular contents, not the
        -- idea of a chest.
        if not (DAEMON and DAEMON.items) then return end

        local room_id = "thornhollow.undercroft"
        for _, entry in ipairs(DAEMON.items.in_room(room_id)) do
            if entry.template == "vault_chest" then return end
        end

        DAEMON.items.spawn("vault_chest", DAEMON.items.location("room", room_id))
    end,
}
