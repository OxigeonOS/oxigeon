# Prototypes — Authoring by Inheritance

```lua
-- game/prototypes/beasts.lua — hand-written. OLC never touches this file.
return {
    mobs = {
        ["beast"]       = { race = "beast", aggressive = true, count = 1 },
        ["mine.beast"]  = { prototype = "beast", faction = "mine",
                            tags = { "beast", "mine" } },
        ["mine.crawler"] = { prototype = "mine.beast", name = "crawler" },
    },
}
```

```lua
-- game/areas/collapsed_mine/mobs.lua — generated. Only what differs.
{
    id           = "mine_crawler",
    prototype    = "mine.crawler",
    short        = "a pale mine crawler",
    stats        = { hp = 55, strength = 14, dexterity = 12, level = 7 },
    damage       = { min = 5, max = 11 },
    xp_award     = 90,
    spawn_room   = "collapsed_mine.first_level",
    count        = 2,
},
```

That is the whole interface. The interesting part is what the second file no
longer says.

## Why this exists

`mine_crawler`, `shale_lurker`, `reed_crawler` and `marsh_lurker` were four
copies of one twelve-key skeleton with four numbers changed. There was no way to
say *"this is another one of those"*, so changing what a crawler is meant finding
every crawler, in two areas, and getting all of them.

Evennia solves this with prototypes and we have taken the idea, not the shape.
In Evennia a prototype *is* the definition and `spawn()` instantiates it. Here
every spawnable thing already has a registered template with an id, and templates
are what `mob_d`, `item_d`, `combat_d` and the loot tables all name. So a
prototype is not a replacement for a template — it is what a template inherits.

> **Resolved at load, not at spawn.**

Flattening happens in `areaload`, on the flat authoring data, before anything is
registered. A registered template is therefore exactly what it has always been,
and nothing downstream can tell a prototyped one from a hand-written one. That is
why combat, spawning and every item path needed no changes at all.

The cost is that an edit takes effect on `areas reset` rather than instantly.
`areaload` flushes the prototype index on every load, so that is structural
rather than something each reload path has to remember.

## The four layers

```
schema defaults  ←  prototype chain  ←  the area's data file  ←  custom.lua
```

Each layer is more specific and more hand-written than the last, and the merge
order is exactly that order.

`custom.lua` and prototypes do not compete. `custom.lua` is *this area's* last
word; a prototype is *everyone's* first word.

| | `custom.lua` | a prototype |
|---|---|---|
| scope | one area | everywhere it is named |
| keyed by | record id | prototype id |
| may hold functions | yes | yes |
| may delete a key | **no**, deliberately | yes, with `@none` |
| written by OLC | never | never |

## Where they live

One `prototypes/` directory, discovered across both jail roots exactly the way
`schema/` and `components/` are. The mudlib ships the index and no content;
prototypes go in `game/prototypes/*.lua`.

The file name carries nothing — the id does. An area that wants a private
prototype calls it `collapsed_mine.crawler` and puts it wherever it likes.

> [!NOTE]
> There is deliberately no per-area `prototypes.lua`. The duplication this exists
> to remove is *cross-area*: `reed_crawler` and `mine_crawler` are the same
> creature in two areas. A per-area file cannot express that, so the first thing
> anybody would do is copy the prototype into the second area — reproducing the
> duplication one level up, which is worse than not having the feature.

Keyed by the plural — `mobs`, `items`, `rooms` — matching `custom.lua` and the
generated file names, so a builder learns one shape. Keying by kind first is also
what keeps a prototype *pure authoring data*: there is no `kind` or `doc`
metadata key inside it that could one day collide with a schema field a game
layer invents.

## The inheritance model

**One parent.** A list of parents needs a linearisation, and every one of them
produces a "which parent won?" question weekly. Multiple inheritance for items is
already served by `components`, which is explicit, orthogonal and discovered.

> *If you want two parents, you want a component, or a third prototype that names
> one of them.*

**Dotted names are a naming convention and nothing else.** `beast.crawler` does
not implicitly inherit `beast`; `prototype = "beast"` is always written out.
Implicit hierarchy means renaming `beast.crawler` to `vermin.crawler` silently
reparents it — a change in behaviour with nothing in the diff to show for it. It
would also make a dot mean inheritance here and containment one namespace over,
where a room id is `<area>.<room>`.

Depth is capped at 8 and cycles are detected. Both report the **full path**
(`marsh_lurker -> beast.crawler -> beast -> beast.crawler`), because the path is
the fix.

## What merges and what replaces

The schema decides — never a guess about shape, which gets an *empty* `exits`
wrong the first time and gets it wrong silently.

| Descriptor type | e.g. | Rule |
|---|---|---|
| scalars, `enum`, `id`, `lfun` | `faction`, `short` | child replaces |
| `range` | `damage` | replaced **wholesale** — `{min,max}` is one value |
| `map` | `stats`, `dialogue`, `exits` | **merged key-by-key** |
| `string_array`, `id_array` | `tags`, `patrol` | child replaces |
| `record_array` | `echoes`, `loot_table` | child replaces |
| a field no schema declares | | kept, and replaces |

The `map` rule is the one that pays for the feature: five stat keys in the
prototype and two in the child produce a creature with five.

An `of_record` value — `exits.north` — is replaced **whole**, never deep-merged.
A child that inherited an exit's `target` while supplying only its `hidden` flag
would be a passage whose destination is invisible in the file in front of you.

> [!WARNING]
> **Arrays replace. They do not union.** Union has no removal, so you immediately
> need a sentinel for the case a sentinel handles worst. Order is content — the
> union of two patrol routes is not a route. And an append silently doubles a
> `loot_table` entry that reads as correct in both files.
>
> The consequence is a good one: `tags = {}` in a child overrides the parent's
> list with an empty one, with no sentinel and no magic. In OLC you never type a
> list by hand anyway — `olc add tags mine` operates on the *effective* list and
> writes the whole result.

## Removing an inherited field: `@none`

```lua
{ id = "shale_lurker", prototype = "mine.lurker", patrol = "@none" }
```

`custom.lua` deliberately has no delete sentinel, and its reason is good: there
the generated file is the whole truth, so "take it out in OLC" is always
available, and a deletion visible only in the patch file would not be.

**That argument does not carry across.** A prototyped record is incomplete by
construction — the value to remove lives in the *parent's* file. Without a
sentinel, a child that needs one field fewer must either stop inheriting or make
the prototype worse, and both are the failure the feature exists to prevent.

- Legal at the top level of a field, and at **one** key of a map
  (`stats.fear = "@none"`). Not deeper.
- **Consumed by the resolver.** No `@none` ever reaches a registered template,
  which is the only reason `item_d.resolve` and `mob_d.spawn` need to know
  nothing about it.
- Reported by `verify` as a note, and printed by `olc show` as
  `- patrol  (removed here)` — so the deletion is visible in the child's own
  file, in the diff and in the linter. That is exactly the visibility
  `custom.lua`'s rule was protecting, restored by other means.
- A string rather than a table identity, because codegen has to be able to
  *emit* it. `@` cannot begin a valid map key, a tag, a trait id or a room id.
  The price, stated plainly: a `description` of literally `"@none"` is
  unwritable.

## OLC

> **OLC edits overrides. OLC shows effective values. The file holds overrides.**

```
  mob shale_lurker
    name             lurker
  ~ faction          mine                     [mine.beast]
  ~ aggressive       true                     [beast]
    xp_award         130
  - patrol           (removed here)
  # id               shale_lurker

  prototype mine.lurker -> mine.beast -> beast
  · set here   ~ inherited   - struck   # hand-code only
  6 of 11 values are inherited. `olc thin` drops what only restates them.
```

| Command | |
|---|---|
| `olc new mob <id> from proto:<base>` | the record is two keys, and nothing is copied |
| `olc protos [kind]` | every prototype, with its parent and how many records use it |
| `olc show proto:<id>` | what you are inheriting |
| `olc set prototype <id>` | checked before it lands — a typo would silently inherit nothing |
| `olc unset <field>` | clear it here; an inherited value comes back |
| `olc strike <field>` | remove an inherited field entirely |
| `olc thin` | drop everything that only restates the prototype |

`from` resolves a **component first**, then a prototype, so nothing already typed
can change meaning. `from comp:x` and `from proto:x` are explicit.

> [!IMPORTANT]
> **Never subtract. Never infer intent from value equality.**
>
> The draft a session holds *is* the override set, so `serialize`, `codegen` and
> `olc.merged` learn nothing about prototypes. The alternative — hold merged
> values and subtract at write time — needs the subtraction to be the exact
> inverse of the merge, which is a second algorithm that will diverge from the
> first. And it is *wrong* in a case that occurs: a builder who deliberately sets
> a value equal to the inherited one is saying "this is mine now, and it must not
> move when the prototype moves".
>
> `olc thin` is the safe form: a human asked for it, in a session, with the
> result on screen.

**OLC cannot write prototypes, and will not.** Generating `prototypes/*.lua`
would make them OLC-owned, which means the functions people put in them get eaten
by a regeneration — the exact failure `adopt_d` and `_meta.managed` exist to
prevent, with no `_meta` to hang a gate on and N areas changed per edit.
Prototypes are hand-written, like `custom.lua`, and for the same reason.

## verify

`verify` asks two different questions of the same area, and each check gets the
view that answers its own:

| View | Checks |
|---|---|
| **raw** — what does this file hold | duplicate ids, `lossy`, `unknown`, serializability, `custom.lua` ids, the prototype chain, strikes |
| **resolved** — what will the next reload do | schema validation, exits, reachability, references, traits, components, style |

A child missing a field it inherits is **not** an error. An inherited function is
**not** reported as belonging in `custom.lua` — it already lives in a hand-written
file that nothing regenerates, so the finding would be both wrong and
unactionable.

`verify prototypes` lints the library itself, which is worth asking once rather
than N times as "does not exist" against each child.

## Failure modes

Every one of these logs to the console **and** the journal, leaves the record
with its own unresolved data, and carries on. A broken prototype costs one
creature's stat block, never the area.

| | |
|---|---|
| the parent does not exist | named, and if an id of that name exists under another kind, it says so: *"'reagent' is an item prototype; this is a mob"* |
| a cycle, or depth > 8 | the full path |
| a prototype names a field no schema declares | **kept and merged, and reported.** With no descriptor there is no merge rule, so it replaces |
| a prototype declares an `id` | dropped and reported — it would otherwise overwrite every child's |
| a prototype file does not compile | that file's prototypes are absent; children then fail with "does not exist", which is the correct cascade |
| a template names another **template** | "does not exist". Templates are area-scoped and load in passes; a cross-area template parent would make load order matter again, which `load_all`'s passes exist to eliminate |

## Permissions

```toml
"/game/prototypes" = { write = "dir.write.game.prototypes" }
```

Not granted to the builder role. A prototype holds functions and one edit reaches
every area that names it, so it is a code change wearing content's clothes — the
same reasoning as `/game/lib`. Reads stay open, because `verify prototypes` and
`olc show proto:<id>` are for any builder.

## What it will not do

- **No multiple inheritance.** Use a component, or a third prototype.
- **No `spawn` from a prototype.** That would register a template under an
  invented id that no file declares — a registry entry with no source, which is
  the class of thing area discovery exists to eliminate. `olc new mob scratch
  from proto:beast` applies live immediately and leaves a file trail.
- **No implicit hierarchy from dotted names.**
- **No auto-thinning on save.**
- **No per-field "this restates the prototype" lint.** It would fire forever on
  legitimate content, and a linter people learn to skim catches nothing at all.
  The record-level version — *"adds nothing its prototype does not already say"* —
  cannot false-positive and is reported.

## See also

- [OLC — Building In-Game](./olc.md) — the round trip a prototyped record makes
- [World Building](./world-building.md) — what a room is as data
- [Creatures & Combat](./combat.md) — templates and instances
- [Items, Equipment & Containers](./items.md) — components, which are the other
  axis
- [Spawners](./spawners.md) — the workshop's rat nest, which is a prototype chain
  and a spawner used together and is the shortest worked example of either
