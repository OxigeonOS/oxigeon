-- mudlib/schema/mob.lua — What a creature template is, as data.
--
-- `mob_d` already stores raw templates and builds at spawn time, so this kind
-- was always the closest to authorable. What it lacked was anything that knew
-- which of its twenty-odd fields a builder may touch.

local M = {}

M.kind  = "mob"
M.order = 30

M.fields = {
    { name = "id", type = "id", target = "mob", required = true, editable = false,
      help = "Template id. `spawn` and loot tables name this." },

    -- Second, because `schema.orderer` emits in schema order and this belongs on
    -- line two of every generated record: it is the first thing a reader of the
    -- file needs, since the rest of the record is only what *differs* from it.
    { name = "prototype", type = "id", target = "prototype", editable = true,
      help = "A prototype this inherits from, resolved at area load. This record "
          .. "holds only what differs from it. See docs/src/lua-api/prototypes.md." },

    { name = "name", type = "string", editable = true,
      help = "The noun combat and `talk` use: 'bone picker'." },

    { name = "short", type = "string", default = "Something", editable = true,
      lfun = true, help = "How it reads in a room: 'a stooped bone picker'." },

    { name = "description", type = "text", editable = true, lfun = true,
      help = "What `examine` shows." },

    { name = "race", type = "string", editable = true },
    { name = "faction", type = "string", editable = true },
    { name = "gender", type = "enum", values = { "male", "female", "neutral" }, editable = true },
    { name = "title", type = "string", editable = true },

    -- Every authored key lands in `stats` — `Mobile:new` merges without a
    -- whitelist — so a creature's skills arrive here with its attributes and
    -- get clamping and derived traits for free. See docs/src/lua-api/traits.md.
    { name = "stats", type = "map", of = "number", key_source = "trait",
      default = {}, editable = true,
      help = "Trait id -> starting value. Checked against trait_d's definitions." },

    { name = "damage", type = "range", default = { min = 1, max = 2 }, editable = true,
      help = "Unarmed spread, used when nothing is wielded." },
    { name = "damage_type", type = "string", default = "physical", editable = true },
    { name = "xp_award", type = "integer", default = 0, min = 0, editable = true },

    { name = "aggressive", type = "boolean", default = false, editable = true },
    { name = "stationary", type = "boolean", default = false, editable = true },
    { name = "unique", type = "boolean", default = false, editable = true },

    { name = "spawn_room", type = "id", target = "room", editable = true,
      help = "Where `populate` puts it. A forward reference here is normal." },
    { name = "count", type = "integer", default = 1, min = 0, editable = true },
    { name = "respawn_time", type = "integer", min = 0, editable = true },

    -- Both forms `Mobile:roll_echo` accepts: a bare string is an echo of
    -- weight one, and the table form is the same echo with a weight. The
    -- text may be an lfun, which is how thornhollow's drunk says something
    -- different depending on the room.
    { name = "echoes", type = "record_array", default = {}, editable = true,
      bare = "text",
      record = {
          { name = "text", type = "text", lfun = true, required = true },
          { name = "weight", type = "number", min = 0, default = 1 },
      },
      help = "Idle lines. One is picked every echo_interval." },
    { name = "echo_interval", type = "integer", default = 30, min = 1, editable = true },

    { name = "patrol", type = "id_array", target = "room", editable = true,
      help = "Room ids to walk between, in order." },
    { name = "patrol_interval", type = "integer", default = 15, min = 1, editable = true },

    -- Usually a map of strings, occasionally holding a function. The generic
    -- lossy detector moves the WHOLE field to custom.lua if any value is one:
    -- splitting a field half-and-half between the two files would be worse than
    -- either, because neither file would then be readable on its own.
    { name = "dialogue", type = "map", of = "text", editable = true, lfun = true,
      help = "topic -> what it says. A function is legal and moves to custom.lua." },

    { name = "loot_table", type = "record_array", editable = true,
      record = {
          { name = "item_id", type = "id", target = "item", required = true },
          { name = "chance", type = "number", min = 0, max = 1, default = 1 },
      },
      help = "{ { item_id = 'x', chance = 0.4 }, ... }" },

    { name = "inventory", type = "id_array", target = "item", editable = true },
    { name = "equipment", type = "map", of = "id", target = "item", editable = true,
      help = "slot -> item template id." },

    { name = "tags", type = "string_array", default = {}, editable = true },
}

--- Creature hooks. Functions, so `custom.lua`'s business — and the reason a
--- creature's *behaviour* is explicitly outside what OLC can author.
M.hand_written = { "on_death", "on_combat", "on_interact", "on_spawn" }

return M
