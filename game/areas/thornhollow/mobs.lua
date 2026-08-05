-- game/areas/thornhollow/mobs.lua — Who is in the town.
--
-- Between them these exercise the mob fields that had no reader:
--
--   dialogue     the smith, Hobb and the apothecary — `talk` and `ask`
--   faction      the guards, so an attack on one brings the other
--   stationary   the guards, who never wander
--   echoes       the drunk and the apprentice, weighted and lfun
--   patrol       the watchman, who walks the town at night
--   unique       the watchman: one of him, however often he is populated
--
-- The two lfuns below are named above the data, so the tables read as what the
-- creatures *are* rather than as prose with programs in the middle of them.

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
    -- ─── Shopkeepers ─────────────────────────────────────────────────────────
    {
        id          = "town_smith",
        name        = "smith",
        short       = "Bellow the smith",
        description = "A wide, unhurried woman with forearms like ship's rope "
                   .. "and a burn scar over one eye that she does not explain.",
        stats       = { hp = 90, max_hp_flat = 90, strength = 18, dexterity = 11,
                        constitution = 16, level = 8 },
        xp_award    = 0,
        stationary  = true,
        unique      = true,
        faction     = "thornhollow",
        spawn_room  = "thornhollow.smithy",
        count       = 1,
        tags        = { "merchant", "townsfolk" },

        dialogue = {
            greeting = "Bellow looks up from the bench. \"Aye. Buy something or "
                    .. "stand somewhere else.\"",
            ore      = "\"Nothing's come out of that mine in two years. What I "
                    .. "work now is what I can buy off the barges, and the "
                    .. "barges are getting choosy.\"",
            mine     = "\"Collapsed. Third level went and took the second with "
                    .. "it. Ask at the Bell — somebody down there still goes.\"",
            son      = "The hammer stops. \"There isn't one.\" The hammer starts "
                    .. "again.",
            marsh    = "\"Don't. And if you do, stay on the stone. The stone is "
                    .. "there for a reason and the reason is still there.\"",
            -- The lfun form: an answer that depends on what you are carrying.
            sword    = smith_on_sword,
        },
    },

    {
        id          = "town_hobb",
        name        = "hobb",
        short       = "Hobb, who keeps the store",
        description = "A narrow man with a ledger he never writes in and an "
                   .. "expression of permanent, mild disappointment.",
        stats       = { hp = 40, max_hp_flat = 40, strength = 8, dexterity = 10,
                        constitution = 9, level = 3 },
        xp_award    = 0,
        stationary  = true,
        unique      = true,
        faction     = "thornhollow",
        spawn_room  = "thornhollow.general_store",
        count       = 1,
        tags        = { "merchant", "townsfolk" },

        dialogue = {
            greeting = "\"Everything's on the shelves. If it isn't on the "
                    .. "shelves I haven't got it.\"",
            credit   = "Hobb points at the card by the door without looking at it.",
            rope     = "\"Twenty foot. Tarred both ends. If you want forty foot "
                    .. "buy two and tie them, and don't tell me about it after.\"",
            lantern  = "\"Hooded. You'll want hooded. Down there an open flame "
                    .. "is a way of telling everything where you are.\"",
        },
    },

    {
        id          = "town_apothecary",
        name        = "apothecary",
        short       = "the apothecary",
        description = "Stooped from twenty years of a ceiling six inches too "
                   .. "low, with green under every fingernail.",
        stats       = { hp = 35, max_hp_flat = 35, strength = 7, dexterity = 12,
                        intelligence = 16, wisdom = 15, constitution = 8, level = 4 },
        xp_award    = 0,
        stationary  = true,
        unique      = true,
        faction     = "thornhollow",
        spawn_room  = "thornhollow.apothecary",
        count       = 1,
        tags        = { "merchant", "townsfolk" },

        dialogue = {
            greeting = "\"Mind your head. Everyone says that and everyone is "
                    .. "still right.\"",
            poison   = "\"Marsh fever. It's in the water and the water is in "
                    .. "everything. Antidote's on the counter and it's cheaper "
                    .. "than the alternative.\"",
            marshroot = "\"Comes out of the reach. Yes, I know. Half of what "
                     .. "cures you out here starts as something that would "
                     .. "rather not.\"",
            wisp     = "The apothecary stops arranging drawers. \"You saw it? "
                    .. "Then you were on the stone, or you would not be telling "
                    .. "me.\"",
        },
    },

    -- ─── Guards ──────────────────────────────────────────────────────────────
    -- Two of them, same faction, stationary. Attacking one is how `aggro_d`'s
    -- assist path gets exercised: the other joins in because they share a
    -- faction, which is a field nothing read until now.
    {
        id          = "town_guard",
        name        = "guard",
        short       = "a Thornhollow guard",
        description = "Boiled leather, a spear held like a walking stick, and "
                   .. "the particular boredom of someone who has watched this "
                   .. "gate for six years without incident.",
        stats       = { hp = 70, max_hp_flat = 70, strength = 14, dexterity = 12,
                        constitution = 13, level = 6 },
        damage      = { min = 5, max = 10 },
        xp_award    = 45,
        stationary  = true,
        aggressive  = false,
        faction     = "thornhollow",
        spawn_room  = "thornhollow.west_gate",
        count       = 2,
        respawn_time = 600,
        tags        = { "guard", "townsfolk" },

        dialogue = {
            greeting = "\"Stay on the stone.\"",
            stone    = "\"The causeway. It's laid on piles down to the hard "
                    .. "bottom. Step off it and you're in six feet of water and "
                    .. "eleven feet of everything else.\"",
        },

        loot_table = {
            { item_id = "hard_rations", chance = 0.5 },
        },
    },

    -- ─── Atmosphere ──────────────────────────────────────────────────────────
    {
        id          = "tavern_drunk",
        name        = "drunk",
        short       = "a man asleep in his own coat",
        description = "Face down on the table with one arm hanging. He has been "
                   .. "described as 'about to leave' for several years.",
        stats       = { hp = 20, max_hp_flat = 20, strength = 9, dexterity = 4,
                        constitution = 11, level = 2 },
        xp_award    = 5,
        stationary  = true,
        spawn_room  = "thornhollow.tavern",
        count       = 1,
        respawn_time = 300,
        tags        = { "townsfolk" },

        -- Weighted echoes: the first is common, the last is rare, and the lfun
        -- one changes with the state of the room. Nothing read `echoes` before
        -- this either.
        echo_interval = 45,
        echoes = {
            { text = "The sleeping man says something into the table.", weight = 5 },
            { text = "The sleeping man's hand twitches.", weight = 3 },
            { text = "\"...not the bell,\" says the sleeping man, clearly. "
                  .. "\"It was never the bell.\"", weight = 1 },
        },

        dialogue = {
            greeting = "He does not wake up.",
            bell     = "Without lifting his head: \"Wasn't the bell.\"",
        },
    },

    {
        id          = "forge_apprentice",
        name        = "apprentice",
        short       = "a soot-streaked apprentice",
        description = "Perhaps fifteen, entirely covered in soot except for two "
                   .. "clean circles where goggles were.",
        stats       = { hp = 25, max_hp_flat = 25, strength = 11, dexterity = 13,
                        constitution = 10, level = 2 },
        xp_award    = 8,
        stationary  = true,
        faction     = "thornhollow",
        spawn_room  = "thornhollow.smithy",
        count       = 1,
        respawn_time = 300,
        tags        = { "townsfolk" },

        echo_interval = 60,
        echoes = {
            { text = "The apprentice pumps the bellows in a long, even rhythm.", weight = 4 },
            { text = "The apprentice drops something, and does not look up.", weight = 2 },
            { text = apprentice_at_the_forge, weight = 1 },
        },

        dialogue = {
            greeting = "The apprentice glances at Bellow first, then at you. "
                    .. "\"She's the one you want.\"",
            son      = "\"I'm not.\" Pause. \"Everyone asks.\"",
        },
    },

    -- ─── The watchman ────────────────────────────────────────────────────────
    -- `patrol` and `unique` together: he walks a fixed route, and however many
    -- times `populate()` is called there is exactly one of him.
    {
        id          = "night_watchman",
        name        = "watchman",
        short       = "the night watchman",
        description = "An old man with a staff, a horn he has never blown, and "
                   .. "a route he has walked so long that the town sets its "
                   .. "clocks by where he is.",
        stats       = { hp = 55, max_hp_flat = 55, strength = 12, dexterity = 10,
                        constitution = 12, level = 5 },
        damage      = { min = 3, max = 7 },
        xp_award    = 30,
        unique      = true,
        faction     = "thornhollow",
        spawn_room  = "thornhollow.square",
        count       = 1,
        respawn_time = 900,
        tags        = { "guard", "townsfolk" },

        patrol = {
            "thornhollow.square",
            "thornhollow.market",
            "thornhollow.west_gate",
            "thornhollow.square",
            "thornhollow.undercroft_stair",
        },
        patrol_interval = 30,

        dialogue = {
            greeting = "\"Evening. It's always evening, near enough.\"",
            crypt    = "\"Old part's older than the chapel. Chapel was put on "
                    .. "top of it, which tells you what they thought of it.\"",
        },
    },
}
