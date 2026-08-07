-- mudlib/schema/room.lua — What a room is, as data.
--
-- The mapping `room_d.from_data` performs, written down. It had only ever
-- existed as a docstring above that function and as a hardcoded list of five
-- keys in `codegen_d`, and the two disagreed — which is why a `dig` destroyed
-- `light`, `smell`, `sound` and `tags` on every room it touched.

local movement = require('lib.movement')

local M = {}

M.kind  = "room"
M.order = 10

M.fields = {
    { name = "id", type = "id", target = "room", required = true, editable = false,
      help = "Dotted room id: <area>.<room>. Renaming one breaks every exit into it." },

    -- Second, because `schema.orderer` emits in schema order and this belongs on
    -- line two of every generated record: it is the first thing a reader of the
    -- file needs, since the rest of the record is only what *differs* from it.
    { name = "prototype", type = "id", target = "prototype", editable = true,
      help = "A prototype this inherits from, resolved at area load. This record "
          .. "holds only what differs from it. See docs/src/lua-api/prototypes.md." },

    -- `lfun = true` on the four the class resolves through `Object.resolve`:
    -- each is a string OR a function returning one, and both are ordinary
    -- content. OLC authors the string; a function is `lossy` and moves to
    -- custom.lua on adoption. Without the flag the linter calls every
    -- weather-keyed description in greywater_marsh an error.
    { name = "short", type = "string", default = "A Room", editable = true, max_len = 60,
      lfun = true, help = "The title shown at the top of look." },

    { name = "description", type = "text", default = "You are in a room.", editable = true,
      lfun = true, help = "The prose body." },

    -- Note the collision worth naming: `light` on a ROOM is ambient brightness
    -- and maps to `light_level`; `light` on an ITEM is how brightly it burns.
    -- Per-kind schemas keep them apart, but anyone assuming one descriptor
    -- covers both will be wrong.
    { name = "light", type = "integer", default = 2, min = 0, max = 3, editable = true,
      help = "0 pitch dark, 1 dim, 2 normal, 3 bright." },

    { name = "smell", type = "string", editable = true, lfun = true,
      help = "What it smells of, for `smell`." },

    { name = "sound", type = "string", editable = true, lfun = true,
      help = "What you can hear, for `listen`." },

    { name = "tags", type = "string_array", default = {}, editable = true,
      help = "Indexed by tag_d: outdoor, town, safe, damp. weather_d reads 'outdoor'." },

    -- Ordered by `movement.ORDER`, so a generated file reads north-south-east-
    -- west rather than alphabetically. That table is now the only copy; `dig`
    -- and `cmds/directions.lua` read the same one.
    -- An exit is a room id, or a table describing the passage — a locked door,
    -- a check, something that happens when you walk it. `room_d.from_data`
    -- unwraps either, so both are legal and `of_record` is what says so.
    -- `check` and `on_traverse` are functions and so are hand-written: a room
    -- carrying one is reported as lossy, which is correct.
    { name = "exits", type = "map", of = "id", target = "room",
      key_values = movement.ORDER, default = {}, editable = true,
      of_record = {
          { name = "target", type = "id", target = "room", required = true },
          { name = "hidden", type = "boolean" },
          { name = "locked_desc", type = "string" },
      },
      help = "direction -> target room id, or a table with `target`." },

    { name = "items", type = "map", of = "text", default = {}, editable = true,
      lfun = true,
      help = "keyword -> what `examine <keyword>` shows. Scenery, not objects." },

    { name = "stats", type = "map", of = "number", key_source = "trait", editable = true,
      help = "Trait id -> value. A room's corruption is a trait like any other." },

    -- ─── The spawner ─────────────────────────────────────────────────────────
    --
    -- A place that *produces* creatures, which is a different statement from a
    -- creature saying where it lives. Both exist and they are not
    -- interchangeable:
    --
    --   mob.spawn_room + mob.count     a fixed population — the smith is in the
    --                                  smithy, and there is one of her
    --   room.spawn_*                   a source — this nest makes rats, of
    --                                  these kinds, up to this many, over time
    --
    -- Authoring a spawner on the *room* rather than as a fourth generated kind
    -- means OLC can already edit it: these are three ordinary fields in types
    -- the schema has, so `olc set spawn_max 4` needs no new machinery. The cost
    -- is that a room has one spawner and it seeds itself, which is what a nest
    -- is. See docs/src/lua-api/spawners.md.
    { name = "spawn_max", type = "integer", min = 0, max = 50, editable = true,
      help = "Most creatures from this spawner alive here at once. Absent or 0 "
          .. "means the room has no spawner." },

    { name = "spawn_interval", type = "number", min = 1, default = 60, editable = true,
      help = "Seconds between top-ups. One creature per tick, so this is the "
          .. "rate the room refills at, not how long it takes to fill." },

    { name = "spawn_table", type = "record_array", editable = true,
      record = {
          { name = "template", type = "id", target = "mob", required = true,
            key = true },
          { name = "weight", type = "number", min = 0, default = 1 },
      },
      help = "{ { template = 'black_rat', weight = 5 }, ... } — weighted, and "
          .. "the weights are relative to each other rather than to anything." },
}

--- Fields the class reads that OLC deliberately cannot author.
---
--- `actions` maps a verb to a function. Naming it here is what makes `adopt`
--- report it as "moves to custom.lua" rather than as an unknown field somebody
--- might delete.
M.hand_written = { "actions" }

return M
