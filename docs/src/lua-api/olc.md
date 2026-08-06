# OLC — building without leaving the game

`olc` makes areas, rooms, items and creatures; `dig` makes passages; `verify`
checks the result; and `ls`/`cd`/`cat` let you look around the files. Everything
it writes is plain Lua data you could have typed by hand, and everything it
cannot express lives in a file it never touches.

```
> olc new area crypt The Sunken Crypt
[OLC] Created area 'crypt'. You are in crypt.entrance.
      `olc save` writes it. `custom.lua` is yours and is never regenerated.

[OLC crypt] > dig n hall
[OLC] Created room crypt.hall
  crypt.entrance  north → crypt.hall
  crypt.hall      south → crypt.entrance  (the way back)
  Cursor: crypt.hall. Unsaved — `olc save` to write.

[OLC crypt] > olc set short The Great Hall
[OLC] crypt.hall.short = The Great Hall  (was Hall)
```

---

## The two ideas everything rests on

**1. A schema says what an authorable thing is.** `mudlib/schema/room.lua`,
`item.lua` and `mob.lua` list every field with its type, its default, whether OLC
may set it, and one line of help. Codegen emits from it, `olc set` validates
through it, `verify` checks against it, and `objdump -s` annotates with it. One
description, four consumers.

Component fields live in the component file beside the `from_data` that reads
them — `mudlib/components/weapon.lua` carries its own `M.fields` — and are
discovered rather than listed. A new component becomes authorable by existing.

**2. The flat authoring form is what gets written, not the built object.**
`Weapon{…}` → `Item:new` + `weapon.from_data` is a one-way function: an Item
cannot be turned back into a file. So OLC reads and writes the *input* to that —
the same table `Weapon{…}` takes — and the loader builds from it.

---

## What an area is made of

```
game/areas/crypt/
  _meta.lua     OLC-owned. Carries `managed`, which gates every write.
  rooms.lua     OLC-owned. An array of flat room data.
  items.lua     OLC-owned. An array of flat item authoring data.
  mobs.lua      OLC-owned. An array of creature templates.
  custom.lua    YOURS. Never read or written by OLC.
```

The four generated files are rewritten **wholesale** on every save. That is only
safe because the fifth exists.

### `custom.lua` is the half that is code

```lua
-- game/areas/crypt/custom.lua — hand-written. OLC never touches this.
local function pull_chain(session_id, args_str, args) … end

return {
    rooms = {
        ["crypt.cistern"] = {
            actions     = { pull = { func = pull_chain, hint = "pull the chain" } },
            description = function(room)
                if get_object_state(room.id, "drained") then return "…empty…" end
                return "…half full of black water…"
            end,
        },
    },
    items = { ["crypt_lantern"] = { on_use = function(item, user_id) … end } },
    mobs  = { ["crypt_eel"]     = { on_death = function(mob) … end } },

    --- Called last, and again on every area reset — so it must be idempotent.
    on_load = function(area_name) … end,
}
```

Patches merge over the generated data **before** it is constructed, so a patched
`damage` reaches `weapon.from_data` rather than an already-built component.

- A `map` field (`exits`, `items`, `stats`, `dialogue`) merges key by key.
  Everything else is replaced. The schema decides which, not a guess about shape
  — that guess is wrong the first time somebody patches an empty `exits`.
- A patch naming an id no generated file declares is a **warning**, not a
  silence. It is how a room you renamed gets noticed: the patch carrying its
  actions now points at nothing.
- There is deliberately **no way to delete a generated key from a patch**. If a
  field should not be there, take it out in OLC where it shows in the diff.

---

## Areas are discovered, not listed

`game/init.lua` used to name every area: a `pcall` block each, requiring rooms,
items, mobs and shops by hand. An area OLC created was invisible until somebody
edited that file — and OLC never registered a reset source, so
`areas reset <new_area>` answered "No registered source" for every area it had
ever made.

`lib/areaload.lua` walks `areas/` and loads what it finds, in **passes across all
areas** — items, then rooms, then mobs, then shops. Passes rather than
area-by-area removes every cross-area ordering hazard at once, including one
already in the tree: `thornhollow.smithy` has a `down` exit into
`collapsed_mine.adit`, which worked only because the two happened to be listed in
the right order.

Five entry file names — `rooms.lua`/`init.lua`, `items.lua`, `mobs.lua`,
`shops.lua`, `custom.lua`. Anything else in the directory is included by one of
those. That single convention is why `legacy_rooms.lua` needs no special case.

Registering the reset spec is the *last act* of loading an area, so a working
`areas reset` is not something anyone has to remember.

---

## The grammar

`olc` is one verb. Nothing is swallowed: `look`, `who` and a tell all still work
while you build, and the dispatcher's room-action layer is untouched.

```
Session   olc <area>                enter (refuses one OLC does not manage)
          olc new area <name> [title…]
          olc adopt <area>          what adopting an existing area would change
          olc done
Cursor    olc edit <target>         what `set` acts on; `olc edit` = this room
          olc where                 cursor, versus where you are standing
Create    olc new room|item|mob <id> [from <base>]
          olc bases [item|mob|room]
Inspect   olc show [<target>] · olc fields [<kind>] · olc help <field>
          olc list rooms|items|mobs · olc diff
Change    olc set <field> <value>   … or bare, to open the editor
          olc set on <target> <field> <value>
          olc unset <field> · olc add|remove <field> <value>
          olc tag|untag <tag>… · olc comp add|remove|list <component>
Persist   olc save · olc revert [<target>]
```

### The cursor does not follow movement

Walking next door to see what an exit looks like from the other side and walking
back must not make it fifty-fifty which room the next `set` writes to. `dig`
*does* move the cursor, because you just explicitly created that room, and
`olc where` says so when the two differ.

`olc set on <target> …` writes somewhere else without moving the cursor. `on` is
a reserved word — `schema.RESERVED`, asserted by a test — rather than a guess
about whether the next token resolves as a field. That guess is DWIM on a command
that writes files, and it goes wrong the day somebody names an item `damage`.

### Buffered, not write-through

`set` changes the draft **and** the live object, so the room changes under you as
you type. Disk is touched only by `olc save`, which runs `verify` first and
refuses on an error.

The old OLC wrote on every `dig`. That is what makes a lint pointless: you cannot
gate a write on a check that runs after the write.

Leaving with unsaved work is refused. On a dropped connection the count is
journalled rather than silently discarded.

---

## What is refused, and when

> OLC refuses at input time anything the serializer cannot round-trip. Anything
> merely *probably wrong* is accepted, recorded, and reported by `verify`.

That split matters because **forward references are the normal case**: you set
`spawn_room` before digging it, and an OLC that refuses them is one you have to
build in the right order.

| Refused as you type | Reported by `verify` |
|---|---|
| a non-finite number | a reference to something that does not exist |
| a boolean that is not one of `true false yes no on off 1 0` | a one-way passage |
| an enum non-member (the error lists them) | a room nothing leads to |
| a map key that is not a keyword | a trait `trait_d` does not define |
| an `lfun` field, always — with the `custom.lua` recipe | a field no schema names |

Nothing is truthy-by-default: `olc set aggressive maybe` is an error, not a
`true`.

Strings are not narrowed at all. `lib/serialize.lua` handles quotes, `]]`,
backslashes, control bytes and UTF-8, so there is nothing to protect the file
from.

### Prose

`olc set description Some short text` sets it directly. `olc set description`
with nothing after it opens `editor_d`, pre-loaded with what is there:

```
[EDITOR] crypt.ossuary.description — 1 line.  .s save  .q abort  .h help
  1] A bare room awaiting description.
] .c
[EDITOR] Buffer cleared.
] Bone stacked to the vault, femur on femur, four hundred years of
] Thornhollow's dead sorted by part rather than by person.
] .s
[EDITOR] Saved 2 lines, 118 characters.
```

Commands are dot-prefixed so a line of prose is never eaten, and `..` escapes a
literal leading dot. `quit` typed inside is *text* — a description containing the
word is ordinary — so `.q` is the only way out.

---

## `verify`

```
verify <path>          does this file compile?  (unchanged)
verify area <name>     lint an area, read from disk
verify                 lint the area you are building
verify all             every discovered area
```

**It reads disk, not the registry.** By the time content is registered it has had
its duplicate ids collapsed, its unknown fields dropped by the loader, and
`custom.lua` applied over the top. What a builder about to save needs to know is
what the *next reload* will do, and only the files can answer that.

| Severity | Meaning | Gate |
|---|---|---|
| `error` | Won't load, or a player can reach a broken state | `olc save` refuses |
| `warn` | Probably wrong, sometimes deliberate | printed |
| `note` | Incomplete or unstyled | printed |
| `lossy` | On disk, not owned by the schema — **the next save would drop it** | `olc adopt` refuses |

`lossy` is its own level rather than an error because it is a different kind of
statement. The others are about whether the area *works*; this one is about
whether saving it would destroy something, and buried among the warnings it gets
skimmed past.

`verify` reports and never changes anything. There is no `--fix`: auto-editing
generated content with no undo is how somebody loses a description to a linter's
idea of tidy.

Orphan detection walks from `_meta.entrance`, defaulting to `<area>.entrance`.
Guessing one — "the room with no inbound edges" — would pick a different room
after every edit and make the list flap between runs.

---

## Adopting an existing area

OLC refuses any area whose `_meta.lua` lacks `managed = "olc.v1"`. Every area
this repository ships is refused, and deliberately: thornhollow's square carries
two inline room actions, greywater_marsh's descriptions are lfuns keyed on the
weather, and regenerating either would delete them while leaving a file that
still compiles.

```
> olc adopt oldtown
oldtown is not OLC-managed. Adopting it would rewrite its data files.

  rooms.lua    12 rooms

Moves to custom.lua — OLC cannot author these:
  room   oldtown.square    actions      hand-written
  room   oldtown.well      description  function

Named by no schema — kept verbatim, not editable in OLC:
  room   oldtown.square    puzzle_seed

Nothing has been written. Re-run with --confirm to adopt.
Originals are copied to legacy_*.lua; nothing is deleted.
```

`--confirm`, in this order:

1. copy each entry file to `legacy_<name>.lua` — **refusing if one exists**;
2. compile-check each copy;
3. write `custom.lua` — **refusing if it exists**, because hand-written code is
   the one thing here that cannot be regenerated;
4. write the data files;
5. write `_meta.lua` **last**, so a failure part-way leaves the area unmanaged
   and OLC still refuses to touch it.

**No Lua source is ever parsed.** The obvious implementation lifts each function
body into the new `custom.lua`; that is a source transformation, it would fail
subtly rather than loudly, and the failure would be somebody's room action
quietly not working. The generated `custom.lua` *references* the copy instead:

```lua
local legacy_rooms = require('areas.oldtown.legacy_rooms')
local function by_id(list, id) … end

return {
    rooms = {
        ["oldtown.square"] = { actions = by_id(legacy_rooms, "oldtown.square").actions },
    },
}
```

Mechanical, lossless, and it leaves an obvious tidying job with a stated end
condition. Nothing is deleted, ever.

---

## Three fields to know about

**A field no schema names is kept, not dropped.** It round-trips verbatim and is
reported as a `note`. Silently losing a field nobody had got round to declaring
is the bug class this whole design exists to end, and it is indistinguishable
from a typo unless it is said out loud.

**`lfun = true` is a flag, not a type.** A room's `description` is prose whether
it is written out or computed; OLC authors the string, a function is legal
content, and a function makes the field `lossy`. That is different from
`type = "lfun"` — a weapon's `hit_message` — which OLC may never set at all.

**`light` means two things.** On a room it is ambient brightness and maps to
`light_level`; on an item it is emission. Per-kind schemas keep them apart, but
anyone assuming one descriptor covers both will be wrong.

---

## The file shell

```
> pwd
/game/areas/crypt

> ls
/game/areas/crypt
  _meta.lua
  custom.lua
  items.lua
  rooms.lua
  4 files, 0 directories.

> ls /
/
  game/
  mudlib/
  0 files, 2 directories.
```

Two mount points, not `list_dir`'s merged view. Merged,
`game/cmds/verify.lua` shadowing `mudlib/cmds/admin/verify.lua` shows as one
entry — so you edit the copy that is not loaded and nothing happens. The virtual
path is also the *permission* path: `permissions.toml` keys on `/game/areas`,
and that is what `ls` prints and `cd` accepts.

`~` is the area you are building. `cd` state lives on `fs_d` rather than `olc_d`,
so `olc done` does not throw your working directory away.

`cat -n` numbers lines, and shows a file's own `{colour}` tags as source rather
than rendering them.

**There is no `rm`, `mv`, `mkdir` or whole-file editor**, and a test asserts they
stay absent. An in-game `rm` is how areas vanish; an `edit` would invite hand
edits to the very files OLC regenerates.

---

## Seeing what you have

`objdump -s` marks every field against the schema:

```
> objdump -s -r item:bone_saw
  Raw fields:
  ·     description = A saw for bone, not for wood.
  ·     id = bone_saw
  !     sharpness = 3
  ·     slot = weapon
  ·     weapon:
  #       hit_message = <function> -> "You saw into the bone picker."
  ·       max = 7
  ·       min = 3
  · OLC-editable   # hand-code only   ^ inherited   ! not in the schema
  1 field no schema names, which `olc save` would drop: sharpness
```

`!` is the point: it is the only thing in the system that answers *what am I
about to lose?* before the loss.

`olc fields <kind>` is the other half — everything that *could* exist, with types
and help — while `objdump -s` is this object, annotated.

---

## Permissions

| Grant | For |
|---|---|
| `cmd.olc` | entering an area and editing it |
| `cmd.olc.areas` | creating a new one |
| `cmd.dig`, `cmd.verify`, `cmd.reload`, `cmd.objdump` | the rest of the toolchain |
| `cmd.ls`, `cmd.cd`, `cmd.pwd`, `cmd.cat` | reading around the tree |
| `efun.write_file`, `efun.verify_file` | calling the efuns at all |
| `dir.write.areas` | writing under the area tree |

The command gate and the efun gate are separate and both apply: `cmd.verify`
lets you type the verb, `efun.verify_file` lets mudlib code call the efun. The
`builder` role in `game/setup_roles.lua` carries all of them.
