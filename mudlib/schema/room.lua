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
}

--- Fields the class reads that OLC deliberately cannot author.
---
--- `actions` maps a verb to a function. Naming it here is what makes `adopt`
--- report it as "moves to custom.lua" rather than as an unknown field somebody
--- might delete.
M.hand_written = { "actions" }

return M
