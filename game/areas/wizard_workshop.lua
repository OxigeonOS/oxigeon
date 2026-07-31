-- game/areas/wizard_workshop.lua — A musty old wizard's workshop
-- 5 rooms defined as pure data tables with logic separated at the top.

-- ─── Actions (logic) ─────────────────────────────────────────────────────────

local function search_entrance(session_id, args_str, args)
    send(session_id, "You rummage through the moth-eaten robes hanging from the coat hooks.\r\n")
    send(session_id, "Dust cascades from the fabric. You find nothing but the faint scent of lavender.\r\n")

end

local function examine_laboratory(session_id, args_str, args)
    if args[1] == "retort" or args_str:lower():find("retort") then
        send(session_id, "You peer into the sealed glass retort. A viscous, amber liquid\r\n")
        send(session_id, "bubbles sluggishly inside, emitting tiny sparks of green light\r\n")
        send(session_id, "with each bubble that pops at the surface.\r\n")
    elseif args[1] == "stains" or args_str:lower():find("stains") then
        send(session_id, "The crystallized reagent stains shimmer in unnatural hues —\r\n")
        send(session_id, "deep violet, electric blue, and a sickly chartreuse. They seem\r\n")
        send(session_id, "to shift color when viewed from different angles.\r\n")
    else
        send(session_id, "What would you like to examine?\r\n")
    end

end

local function taste_pantry(session_id, args_str, args)
    send(session_id, "You reach toward one of the jars, but something about the way\r\n")
    send(session_id, "the pickled eyes swivel to track your hand gives you pause.\r\n")
    send(session_id, "Perhaps tasting random alchemical reagents is unwise.\r\n")

end

local function gaze_scrying(session_id, args_str, args)
    send(session_id, "You stare into the depths of the obsidian mirror...\r\n")
    send(session_id, "The churning fog parts for a brief moment. You catch a glimpse\r\n")
    send(session_id, "of a vast, starlit void — and something in the void gazes back.\r\n")
    send(session_id, "A chill runs down your spine as the fog swallows the vision.\r\n")

end

local function read_archive(session_id, args_str, args)
    send(session_id, "You carefully pull a grimoire from the nearest shelf.\r\n")
    send(session_id, "The pages are covered in dense, arcane script that seems to\r\n")
    send(session_id, "writhe and rearrange itself as you watch. A headache begins\r\n")
    send(session_id, "forming behind your eyes, and you wisely close the book.\r\n")

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

    -- Laboratory
    {
        id    = "wizard_workshop.laboratory",
        short = "The Alchemical Laboratory",
        light = 2,
        smell = "A sharp, acrid tang of sulfur and burnt sugar.",
        sound = "A slow, rhythmic bubbling from a sealed glass retort.",

        description = [[
A massive scarred workbench dominates the center of this grand chamber, cluttered
with shattered bubbling alembics and crystallized reagent stains that shimmer in
unnatural hues. A single, hovering magelight flickers intermittently near the
ceiling, casting long, erratic shadows across the soot-stained walls. Forgotten
experiments lie abandoned on scorched tables, their alchemical reactions frozen
in time. To the east and west, arched doorways beckon.]],

        exits = {
            south = "wizard_workshop.entrance",
            north = "wizard_workshop.archive",
            west  = "wizard_workshop.pantry",
            east  = "wizard_workshop.scrying_chamber",
        },

        items = {
            workbench = "The oak workbench is deeply scarred by decades of acid spills and scorch marks. Crystallized reagent stains glitter across its surface like tiny gemstones.",
            magelight = "The hovering magelight is a pale sphere of bluish-white energy, flickering erratically. It seems to be running low on whatever power sustains it.",
        },

        actions = {
            examine = { func = examine_laboratory, hint = "examine <something>" },
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

    -- Archive
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
            desk  = "The reading desk is covered in spilled ink, broken quill nibs, and half-finished notes in a cramped, frantic handwriting.",
            books = "The grimoires are bound in dark leather, some chained to the shelves. A few float lazily, trapped in localized gravity anomalies.",
        },

        actions = {
            read = { func = read_archive, hint = "read" },
        },
    },
}
