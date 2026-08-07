# Messages — One Line, Three Readers

```lua
messaging.announce("$Actor $actor.v(draw) a line of fire at $target.",
                   { actor = caster, target = wisp })
```

```
the caster reads    You draw a line of fire at a pale wisp.
the target reads    Wren draws a line of fire at you.
an onlooker reads   Wren draws a line of fire at a pale wisp.
```

One authored sentence. Before this, every caller wrote two or three strings by
hand and kept them in step by remembering to — and could not say "you" at all.

## The syntax

```
$dealt              a scalar out of the context            -> "7"
$actor              a reference     "you" | "Wren" | "a pale wisp"
$Actor              the same, capitalised
$actor.name         always the display name, even to the actor
$actor.they         subjective      "you" | "she" | "they" | "it"
$actor.them  .their  .theirs  .themself
$actor.v(swing)     a verb agreeing with how `$actor` renders
$actor.vthey(be)    a verb agreeing with the *pronoun*
$weapon.of(actor)   "your longsword" | "her longsword"
$$                  a literal dollar
```

`$` was chosen so this **subsumes** the substitution `ability_d.messages` already
did rather than competing with it: every message already authored is already
valid input and simply gains meaning. There is no second syntax and nothing was
deprecated.

> [!WARNING]
> Not `{actor}`. `lib/color.lua` matches `{(.-)}` across the whole string, so
> `{actor}` and `{red}` would be indistinguishable in the source — nothing in
> the text would say which consumer owns a tag, and it would break the first
> time somebody added a colour alias named `target`. That is the same shape as
> the `cmd.olc`-versus-`olc` collision `CLAUDE.md` already records.

**An unknown token survives verbatim**, form and argument included. `$victim`
stays `$victim`. A message reading "You strike $victim" is a typo somebody can
see and fix; one reading "You strike " is a bug they will stare at.

**Capitalisation is an explicit flag**, not sentence detection: `"{red}$actor
strikes{/}"` has its token at byte six and colour tags are everywhere. Capitals
step over a leading tag, so `{cyan}a wisp` becomes `{cyan}A wisp`.

## The two verb forms, and why there are two

English asks the agreement question twice, and one flag cannot answer both:

```
they swing   /  she swings   /  you swing        the PRONOUN is the subject
Ash swings   /  the rats swarm                   the NAME is the subject
```

A person whose pronouns are they/them takes **"they swing"** and **"Ash
swings"** — a name is third-person singular whatever its owner's pronouns are.
So a pronoun set carries `plural` (agreement with the pronoun) and `collective`
(agreement with the name, true only for something that genuinely is many).

`$actor.v(…)` agrees with how `$actor` renders, which is what almost every line
wants. `$actor.vthey(…)` agrees with the pronoun, for
`"$Actor.They $actor.vthey(be) bleeding."`

## Pronoun sets

| set | | | | | |
|---|---|---|---|---|---|
| `male` | he | him | his | his | himself |
| `female` | she | her | her | hers | herself |
| `neutral` | they | them | their | theirs | **themself** |
| `thing` | it | it | its | its | itself |
| `plural` | they | them | their | theirs | **themselves** |

`neutral` and `plural` differ in exactly one cell — one person whose pronouns
are they/them, against a swarm of rats — and it is the difference people notice.

**An entity with no gender is `it`, unless it is a player, and then it is
`neutral`.** An ungendered creature is a wisp and "it bites you" is right; an
ungendered *player* is a person who has not said, and "it swings" about a person
is offensive where "they swing" is unremarkable. Nothing sets `gender` at
character creation, so every player alive is in that bucket — it is the default
path, not an edge case.

A game gets neopronouns by writing five strings:

```lua
entity.pronouns = { they = "ze", them = "hir", their = "hir",
                    theirs = "hirs", themself = "hirself" }
```

Read before `gender`, so a game can move to a better-named field at its own pace
with no save-format change.

## Which name a creature gets

```toml
[game]
display_name_prefers = "name"   # or "short"
```

A creature carries both and they answer different questions: `name` is what you
type to attack it, `short` is what it reads as in prose. Which belongs in a
sentence is a decision about what kind of game this is — `"short"` for roleplay,
`"name"` for hack-and-slash — so it is configuration. Whichever is preferred,
the other is the fallback.

## The API

| | |
|---|---|
| `messaging.tell(entity, template, ctx)` | render for one reader and send |
| `messaging.broadcast(room_id, template, ctx, opts)` | render per reader, send to a room |
| `messaging.announce(template, ctx, opts)` | the same, room taken from `ctx.actor` |
| `render.render(template, ctx, viewer)` | one string; `viewer = nil` means nobody is "you" |
| `render.render_for(template, ctx, viewers)` | `{ [viewer] = string }` |

`opts = { exclude = entity | char_id | {…}, include = { entities } }`.

**A broadcast parses once and renders once per distinct role set, not per
viewer.** Ten people watching one attacker hit one target is three renders,
because everybody who is neither participant reads the same sentence.

## Where it lives

| | |
|---|---|
| `mudlib/lib/grammar.lua` | pronoun sets, conjugation — facts about *English*, so a translation replaces exactly this file |
| `mudlib/lib/render.lua` | the format, and the parse cache |
| `mudlib/lib/messaging.lua` | sessions, rooms, who is present |

No daemon: nothing outlives a call but derived caches.

## In an ability

```lua
messages = { line = "$Actor $actor.v(draw) a line of fire at $target.",
             result = "It takes $dealt." }
```

`line` is one sentence for everybody. `self`/`room`/`target` remain for the case
that genuinely is three statements — where the actor is told something the room
must not hear — and declaring both is refused at define time.

## See also

- [Abilities](./abilities.md) — where `messages` is authored
- [Creatures & Combat](./combat.md)
