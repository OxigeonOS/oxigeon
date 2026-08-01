-- game/areas/wizard_workshop/rooms.lua — A musty old wizard's workshop
-- 7 rooms defined as pure data tables with logic separated at the top.
--
-- Puzzle: Three potions (red, blue, green) must be combined in the
-- correct order in the cauldron to create a purple teleportation potion.

-- ─── Actions (logic) ─────────────────────────────────────────────────────────

local function search_entrance(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end
    player:send("You rummage through the moth-eaten robes hanging from the coat hooks.")
    player:send("Dust cascades from the fabric. You find nothing but the faint scent of lavender.")
end

local function examine_laboratory(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if args[1] == "retort" or args_str:lower():find("retort") then
        player:send_lines(
            "You peer into the sealed glass retort. A viscous, amber liquid",
            "bubbles sluggishly inside, emitting tiny sparks of green light",
            "with each bubble that pops at the surface."
        )
    elseif args[1] == "stains" or args_str:lower():find("stains") then
        player:send_lines(
            "The crystallized reagent stains shimmer in unnatural hues —",
            "deep violet, electric blue, and a sickly chartreuse. They seem",
            "to shift color when viewed from different angles."
        )
    else
        player:send("What would you like to examine?")
    end
end

-- ─── Potion puzzle state ─────────────────────────────────────────────────────
--
-- Object state keys on "wizard_workshop.laboratory":
--   cauldron_potions  = number of correctly added potions (0..3)
--   cauldron_failed   = boolean, true if the mixture has exploded
--   cauldron_complete = boolean, true if the purple potion is ready
--   cauldron_empty    = boolean, true if the potion has been collected
--
-- Item IDs used:
--   "potion_red", "potion_blue", "potion_green" — reagent potions
--   "empty_vial" — needed to collect the result
--   "purple_potion" — the teleportation potion (defined in areas/wizard_workshop/items.lua)
--   "manasteel_bar" — treasure in the vault
--
-- The expected order is: red, blue, green.

local ROOM_ID = "wizard_workshop.laboratory"
local CORRECT_ORDER = { "potion_red", "potion_blue", "potion_green" }

local function reset_cauldron()
    set_object_state(ROOM_ID, "cauldron_potions", 0)
    set_object_state(ROOM_ID, "cauldron_failed", false)
    set_object_state(ROOM_ID, "cauldron_complete", false)
    set_object_state(ROOM_ID, "cauldron_empty", false)
end

local function search_laboratory(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    -- Check if player already has all the potions
    local found_any = false
    local potions = {
        { id = "potion_red",   name = "a small vial of swirling red liquid" },
        { id = "potion_blue",  name = "a small vial of shimmering blue liquid" },
        { id = "potion_green", name = "a small vial of bubbling green liquid" },
    }

    player:send_lines(
        "You rummage through the cluttered workbench, pushing aside broken alembics",
        "and scattered notes..."
    )

    for _, p in ipairs(potions) do
        if not player:has_item(p.id) then
            player:add_item(p.id)
            player:send("You find " .. p.name .. "!")
            found_any = true
        end
    end

    if not found_any then
        player:send("You've already found everything of interest here.")
    end
end

local function pour_potion(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    -- Check cauldron state
    local complete = get_object_state(ROOM_ID, "cauldron_complete")
    local empty = get_object_state(ROOM_ID, "cauldron_empty")

    if complete and not empty then
        player:send("The cauldron already contains a completed potion. Perhaps you should collect it.")
        return
    end

    -- Parse what potion they want to pour
    local potion_name = args_str:lower():gsub("^%s+", ""):gsub("%s+$", "")
    local potion_id = nil

    if potion_name:find("red") then
        potion_id = "potion_red"
    elseif potion_name:find("blue") then
        potion_id = "potion_blue"
    elseif potion_name:find("green") then
        potion_id = "potion_green"
    else
        player:send("Pour what? You could try: pour red, pour blue, or pour green.")
        return
    end

    -- Check player has the potion
    if not player:has_item(potion_id) then
        player:send("You don't have that potion.")
        return
    end

    -- Determine current step
    local step = get_object_state(ROOM_ID, "cauldron_potions") or 0
    local expected = CORRECT_ORDER[step + 1]

    -- Remove the potion from inventory regardless
    player:remove_item(potion_id)

    local color = potion_id:match("potion_(.+)")
    player:send("You carefully pour the " .. color .. " potion into the cauldron...")
    player:message_room(player.name .. " pours a potion into the cauldron.")

    if potion_id == expected then
        -- Correct order!
        step = step + 1
        set_object_state(ROOM_ID, "cauldron_potions", step)

        if step == 1 then
            player:send("The red liquid swirls in the cauldron, glowing faintly.")
        elseif step == 2 then
            player:send("The blue liquid merges with the red, creating a deep violet swirl.")
        elseif step == 3 then
            -- Puzzle complete!
            set_object_state(ROOM_ID, "cauldron_complete", true)
            player:send_lines(
                "The green liquid cascades in and the mixture erupts with light!",
                "The swirling liquid settles into a rich, luminous purple. Tiny motes",
                "of starlight drift up from the surface. The potion is complete."
            )
            player:message_room("The cauldron erupts with brilliant purple light!")
        end
    else
        -- Wrong order — BOOM!
        set_object_state(ROOM_ID, "cauldron_potions", 0)
        set_object_state(ROOM_ID, "cauldron_failed", true)

        player:send_lines(
            "The mixture hisses violently! The cauldron shudders and a gout of",
            "acrid smoke erupts from within, searing your face and hands!"
        )

        -- Deal 15% max HP damage
        local damage = math.floor(player.stats.max_hp * 0.15)
        local remaining = player:take_damage(damage)
        player:send("You take " .. damage .. " damage from the explosion! (HP: "
            .. remaining .. "/" .. player.stats.max_hp .. ")")

        player:send_lines(
            "The ruined mixture evaporates. The cauldron is unharmed, but your",
            "pride — and your skin — have seen better days."
        )

        player:message_room("The cauldron explodes with smoke and sparks!")

        -- Reset for another attempt — cauldron is fine
        DAEMON.ticker.after(3, "wizard_workshop.cauldron_reset", function()
            reset_cauldron()
        end)
    end
end

local function collect_potion(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    local complete = get_object_state(ROOM_ID, "cauldron_complete")
    local empty = get_object_state(ROOM_ID, "cauldron_empty")

    if not complete then
        player:send("The cauldron is empty. There's nothing to collect.")
        return
    end

    if empty then
        player:send("You've already collected the potion.")
        return
    end

    -- Need the empty vial
    if not player:has_item("empty_vial") then
        player:send("You need something to collect the potion in. An empty vial, perhaps?")
        return
    end

    -- Collect!
    player:remove_item("empty_vial")
    player:add_item("purple_potion")
    set_object_state(ROOM_ID, "cauldron_empty", true)

    player:send_lines(
        "You dip the empty vial into the swirling purple liquid. The potion",
        "flows eagerly into the glass as if drawn by invisible hands. The vial",
        "now pulses with a soft, otherworldly violet glow.",
        "You now have a purple potion!"
    )
end

local function take_vial(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end

    if player:has_item("empty_vial") then
        player:send("You already have the empty vial.")
        return
    end

    player:add_item("empty_vial")
    player:send_lines(
        "You pick up the empty crystal vial from the desk. It's surprisingly",
        "lightweight, the glass so thin it's almost invisible."
    )
end

local function taste_pantry(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end
    player:send_lines(
        "You reach toward one of the jars, but something about the way",
        "the pickled eyes swivel to track your hand gives you pause.",
        "Perhaps tasting random alchemical reagents is unwise."
    )
end

local function gaze_scrying(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end
    player:send_lines(
        "You stare into the depths of the obsidian mirror...",
        "The churning fog parts for a brief moment. You catch a glimpse",
        "of a vast, starlit void — and something in the void gazes back.",
        "A chill runs down your spine as the fog swallows the vision."
    )
end

local function read_archive(session_id, args_str, args)
    local player = get_player(session_id)
    if not player then return end
    player:send_lines(
        "You carefully pull a grimoire from the nearest shelf.",
        "The pages are covered in dense, arcane script that seems to",
        "writhe and rearrange itself as you watch. A headache begins",
        "forming behind your eyes, and you wisely close the book."
    )
end

-- ─── Treasure Vault actions ──────────────────────────────────────────────────

local VAULT_ID = "wizard_workshop.treasure_vault"

local function touch_orb(session_id, args_str, args)
    local target = args_str:lower():gsub("^%s+", ""):gsub("%s+$", "")
    local player = get_player(session_id)

    if target ~= "orb" and target ~= "the orb" then
        if player then player:send("Touch what?") end
        return
    end

    if not player then return end

    player:send_lines(
        "You place your hand on the crystalline orb. Reality bends and",
        "folds around you like a closing fist —",
        ""
    )

    -- Teleport back to the laboratory
    player:move_to(ROOM_ID)

    -- Arrival message
    player:message_room(player.name .. " materializes from thin air in a flash of violet light!")
end

local function take_manasteel(session_id, args_str, args)
    local target = args_str:lower():gsub("^%s+", ""):gsub("%s+$", "")
    if not (target:find("manasteel") or target:find("bars") or target:find("bar")) then
        local player = get_player(session_id)
        if player then player:send("Take what?") end
        return
    end

    local player = get_player(session_id)
    if not player then return end

    local taken = get_object_state(VAULT_ID, "manasteel_taken_" .. tostring(player.char_id))
    if taken then
        player:send("You've already taken your share of the manasteel.")
        return
    end

    -- Give them some bars
    for i = 1, 3 do
        player:add_item("manasteel_bar")
    end
    set_object_state(VAULT_ID, "manasteel_taken_" .. tostring(player.char_id), true)

    player:send_lines(
        "You heft three bars of manasteel from the stack. They're impossibly",
        "dense for their size, thrumming with a low, resonant hum that you",
        "feel in your bones rather than hear."
    )
end

-- ─── Room data ───────────────────────────────────────────────────────────────

return {
    _meta = {
        name   = "wizard_workshop",
        title  = "The Wizard's Workshop",
        author = "Oxigeon",
        level  = "1-5",
        status = "live",
    },

    -- Entrance
    {
        id    = "wizard_workshop.entrance",
        short = "Entrance to the Workshop",
        light = 1,
        smell = "Old dust and faint ozone.",
        sound = "The faint hum of magical wards struggling to persist.",

        description = [[
You stand in a circular foyer choked with decades of dust. A heavy oak door,
banded with iron and etched with faded protective wards, is sealed tight behind
you. The air is still, yet motes of dust dance as if caught in invisible currents
of residual magic. Along the curved walls, tarnished coat hooks hold the
moth-eaten remains of heavy velvet robes.]],

        exits = {
            north = "wizard_workshop.laboratory",
        },

        items = {
            door  = "The heavy oak door is covered in faded silver runes — protective wards, long since weakened by time.",
            robes = "Moth-eaten velvet robes hang limply from the hooks. They might have been magnificent once, dyed deep purple with gold trim.",
        },

        actions = {
            search = { func = search_entrance, hint = "search" },
        },
    },

    -- Laboratory (the cauldron puzzle room)
    {
        id    = "wizard_workshop.laboratory",
        short = "The Alchemical Laboratory",
        light = 2,
        smell = "A sharp, acrid tang of sulfur and burnt sugar.",
        sound = "A slow, rhythmic bubbling from a sealed glass retort.",

        description = function(room)
            local base = [[
A massive scarred workbench dominates the center of this grand chamber, cluttered
with shattered bubbling alembics and crystallized reagent stains that shimmer in
unnatural hues. A single, hovering magelight flickers intermittently near the
ceiling, casting long, erratic shadows across the soot-stained walls.]]

            -- Cauldron description changes based on puzzle state
            local cauldron_desc
            local complete = get_object_state("wizard_workshop.laboratory", "cauldron_complete")
            local empty = get_object_state("wizard_workshop.laboratory", "cauldron_empty")
            local step = get_object_state("wizard_workshop.laboratory", "cauldron_potions") or 0

            if complete and not empty then
                cauldron_desc = "\r\nA heavy iron cauldron sits near the workbench, filled with a luminous\r\npurple liquid. Tiny motes of starlight drift lazily from its surface."
            elseif step == 1 then
                cauldron_desc = "\r\nA heavy iron cauldron sits near the workbench. A faint red glow\r\nflickers from within."
            elseif step == 2 then
                cauldron_desc = "\r\nA heavy iron cauldron sits near the workbench. A deep violet swirl\r\nchurns inside it."
            else
                cauldron_desc = "\r\nA heavy iron cauldron sits near the workbench, its interior blackened\r\nby centuries of alchemical experiments. It appears to be empty."
            end

            return base .. cauldron_desc
        end,

        exits = {
            south = "wizard_workshop.entrance",
            north = "wizard_workshop.archive",
            west  = "wizard_workshop.pantry",
            east  = "wizard_workshop.scrying_chamber",
        },

        items = {
            workbench = "The oak workbench is deeply scarred by decades of acid spills and scorch marks. Crystallized reagent stains glitter across its surface like tiny gemstones.",
            magelight = "The hovering magelight is a pale sphere of bluish-white energy, flickering erratically. It seems to be running low on whatever power sustains it.",
            cauldron  = function(room)
                local complete = get_object_state("wizard_workshop.laboratory", "cauldron_complete")
                local empty = get_object_state("wizard_workshop.laboratory", "cauldron_empty")
                local step = get_object_state("wizard_workshop.laboratory", "cauldron_potions") or 0

                if complete and not empty then
                    return "The iron cauldron brims with luminous purple liquid. Tiny motes of starlight drift from the surface, and the air around it tastes of ancient magic."
                elseif step == 1 then
                    return "The iron cauldron holds a small amount of glowing red liquid that slowly swirls on its own."
                elseif step == 2 then
                    return "The iron cauldron contains a churning violet mixture that gives off wisps of arcane energy."
                else
                    return "A massive iron cauldron, its interior blackened by centuries of use. A faint residue of old magic clings to its walls."
                end
            end,
        },

        actions = {
            examine = { func = examine_laboratory, hint = "examine <something>" },
            search  = { func = search_laboratory, hint = "search" },
            pour    = { func = pour_potion, hint = "pour <color>" },
            collect = { func = collect_potion, hint = "collect" },
            -- NOTE: "drink" is now handled by the mudlib drink command + drinkable component.
            -- The purple potion's on_drink hook handles the teleportation.
        },
    },

    -- Pantry
    {
        id    = "wizard_workshop.pantry",
        short = "Reagent Pantry",
        light = 1,
        smell = "An overwhelming blend of dried mint, decay, and sharp vinegar.",
        sound = "The rustling of dried leaves disturbed by unseen drafts.",

        description = [[
Shadows cling tightly to the corners of this narrow, cramped storage room.
Towering shelves groan under the weight of hundreds of glass jars containing
strange, unidentifiable contents. Pickled eyes bob lazily in formaldehyde,
glowing blue fungi pulse with a faint luminescence, and bundles of dried herbs
hang upside down from the ceiling rafters. It feels as though the ingredients
themselves are watching you.]],

        exits = {
            east = "wizard_workshop.laboratory",
        },

        items = {
            jars  = "Hundreds of glass jars line the shelves, their contents ranging from mundane dried herbs to deeply unsettling specimens floating in murky preservatives.",
            fungi = "The glowing blue fungi pulse rhythmically, like a slow heartbeat, casting eerie shadows behind the jars.",
        },

        actions = {
            taste = { func = taste_pantry, hint = "taste" },
        },
    },

    -- Scrying Chamber
    {
        id    = "wizard_workshop.scrying_chamber",
        short = "The Scrying Chamber",
        light = 1,
        smell = "A metallic scent of ozone, like the air after a fierce lightning storm.",
        sound = "An absolute, oppressive silence that rings in your ears.",

        description = [[
Thick, moth-eaten velvet curtains drape the walls of this circular room,
absorbing all ambient light and sound. In the center stands a large obsidian
mirror, its surface swirling with a milky, churning fog that seems to pull at
your gaze. Intricate star charts and astrological alignments are chalked into
the floorboards, their lines worn but still humming with faint divination magic.]],

        exits = {
            west = "wizard_workshop.laboratory",
        },

        items = {
            mirror = "The obsidian mirror's surface churns with milky fog. For a moment, you think you see a face staring back — but it isn't yours.",
            charts = "Star charts and astrological alignments are chalked in precise detail across the floorboards, mapping constellations you don't recognize.",
        },

        actions = {
            gaze = { func = gaze_scrying, hint = "gaze into the mirror" },
        },
    },

    -- Archive (has the empty vial on the desk)
    {
        id    = "wizard_workshop.archive",
        short = "The Forbidden Archive",
        light = 2,
        smell = "The comforting, rich scent of old leather and foxed pages.",
        sound = "The soft flutter of pages turning on their own.",

        description = [[
Floor-to-ceiling bookshelves curve along the walls, packed tight with ancient,
leather-bound grimoires and crumbling scrolls. A heavy oak reading desk sits near
a shattered stained-glass window, covered in spilled ink and discarded quills.
The knowledge contained within these fragile pages feels palpable, an electric
tension in the air that raises the hairs on your arms. A few books float lazily
a few inches off the shelves, trapped in localized gravity anomalies.]],

        exits = {
            south = "wizard_workshop.laboratory",
        },

        items = {
            desk  = "The reading desk is covered in spilled ink, broken quill nibs, and half-finished notes in a cramped, frantic handwriting. Sitting among the clutter is a small, empty crystal vial — its glass so thin it's almost invisible.",
            vial  = "A small, empty crystal vial sits on the desk. The glass is paper-thin and perfectly clear. It looks like it was made for collecting alchemical samples.",
            books = "The grimoires are bound in dark leather, some chained to the shelves. A few float lazily, trapped in localized gravity anomalies.",
        },

        actions = {
            read = { func = read_archive, hint = "read" },
            take = { func = take_vial, hint = "take vial" },
        },
    },

    -- Treasure Vault (teleported to via purple potion)
    {
        id    = "wizard_workshop.treasure_vault",
        short = "The Hidden Treasure Vault",
        light = 3,
        smell = "Cold stone and the metallic tang of rare alloys.",
        sound = "A deep, resonant hum that seems to emanate from the metal itself.",

        description = function(room)
            local base = [[
You stand in a vaulted chamber carved from living rock, its walls veined with
threads of luminous crystal that pulse with a slow, rhythmic blue light. The air
is cold and still, untouched by time. This place has been sealed for centuries,
waiting for someone clever enough to find it.]]

            local details = "\r\nAgainst the far wall, a stack of shimmering manasteel bars catches the\r\ncrystal-light, their surfaces rippling with captive energy. In the center of\r\nthe chamber, a crystalline orb hovers a foot above a stone pedestal, rotating\r\nslowly and casting prismatic reflections across the walls."

            return base .. details
        end,

        exits = {},  -- No conventional exits — orb teleports back

        items = {
            orb       = "The crystalline orb is perfectly spherical, about the size of a fist. Inside, you can see what looks like a miniature storm of violet lightning. It rotates slowly above its pedestal, humming with power.",
            manasteel = "Bars of manasteel, each about a foot long and incredibly dense. The metal has a liquid-silver sheen and seems to vibrate faintly when touched. This is one of the rarest crafting materials in existence.",
            bars      = "Bars of manasteel, each about a foot long and incredibly dense. The metal has a liquid-silver sheen and seems to vibrate faintly when touched.",
            pedestal  = "A simple stone pedestal, worn smooth by ages. The crystalline orb hovers above it without any visible support.",
            crystal   = "Veins of luminous crystal thread through the rock walls, pulsing with a slow blue rhythm — like a heartbeat. They provide all the light in this sealed chamber.",
        },

        actions = {
            touch = { func = touch_orb, hint = "touch orb" },
            take  = { func = take_manasteel, hint = "take manasteel" },
        },
    },
}
