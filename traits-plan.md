# Sparse Traits — Any Numeric Data, On Any Entity

*(Supersedes the components plan, whose remaining steps live in tasks #3–#7.)*

## Context

`trait_d` was built for character stats and it does that well: four kinds, a
declared-and-enforced dependency graph, cycle detection reported as a path,
lazy timestamp regeneration, memoised values invalidated by a generation
counter. The intent is broader — a trait is meant to be *any* numeric datum any
entity can hold: stats, skills, masteries, item durability, charges.

Two lines stop it being that.

```lua
-- attach, trait_d.lua:564-570 — materialises every defined trait on the entity
for id, def in pairs(r.defs) do ... stats[id] = def.default ... end

-- recompute, trait_d.lua:307 — walks every defined trait, for every entity
for _, id in ipairs(order_of(r)) do
```

Storage bloat is the visible cost. The real one is that **recompute is O(traits
the game defines), not O(traits this entity holds)**. A full recompute of 11
traits measures 11 µs (`traits.md`). Add a trait per weapon type, spell, craft
and mastery — hundreds — and every player pays for all of them on every
invalidation, to compute skills they have never heard of. A sword pays too, and
gets a `willpower` in its save data.

The fix is to let **storage decide what an entity has**, rather than the
definition table deciding that everything has everything.

## What already works — do not rebuild it

- **Runtime definition.** `M.define` (`trait_d.lua:139-141`) already ends with
  `r.order = nil` and `r.gen = r.gen + 1`, so a trait defined mid-session
  invalidates the topological order, re-seals lazily, and invalidates every
  memo. Defining a skill at runtime works today.
- **Cycle detection and `seal()`** are properties of the definitions, not of any
  entity. Untouched.
- **The memo** (`_memo`/`_entity_gen`, weak-keyed, `trait_d.lua:88-89`) and the
  generation counter. `set_base` already calls `M.bump(entity)` after writing
  (`trait_d.lua:436-437`), so a *new* key on an entity already invalidates —
  which is what makes a per-entity present-set cache safe.
- **`stats_of`** (`trait_d.lua:250-254`) auto-creates `entity.stats` on any
  table, so any object is already a candidate entity.

---

## Part 1 — The presence rule

| Kind | Present when |
|---|---|
| `attribute`, `gauge`, `counter` | `entity.stats[id]` is a number |
| `derived` | every id in `depends` is present for that entity |
| any, with `always = true` | the entity has been attached |

Applicability is **derived from the data, never declared**. A sword has `dps`
because it has `damage` and `speed`; it has no `willpower` because it has no
`wisdom`. There is no `applies_to` list to maintain and therefore none to rot —
the same reasoning that made `depends` enforced rather than advisory.

Two consequences to make explicit rather than let emerge:

- **Bounds count as dependencies.** `seal()` already folds a `max` that names a
  trait into the graph, so a gauge whose bound trait is absent is itself absent.
- **`always = true`** is the escape hatch for a formula where absent-means-zero
  is a legitimate answer and the trait should exist on every seeded entity.
  Expect it to be rare; if it is common, the presence rule is wrong.

**Values stored under an undefined trait id are inert, not an error.** A save
holding `swordsmanship` before that trait is defined reads as absent and starts
answering once the definition lands. A broken or not-yet-loaded trait file must
not take a character down, the same way a broken area file does not.

---

## Part 2 — API

### Split `attach` into lifecycle and seeding

`attach` currently does both, which is why it materialises everything.

| Call | Does |
|---|---|
| `attach(entity)` | lifecycle only — ensure `stats` and `_at`, clamp gauges already present, `bump`. Cheap enough for every item instance. |
| `seed(entity, set)` | write the defaults for one named set. Called at character creation and mob spawn. |

Sets are declared on the spec — `sets = "character"` or `sets = {"character",
"mob"}`, defaulting to `{"character"}`. That default means players and mobs
behave exactly as they do today; sets are a *seeding* convenience, and after
seeding, storage is the truth.

### New

| Function | |
|---|---|
| `has(entity, id)` | `-> boolean`. "Do they know it at all", which is a different question from its value. |
| `forget(entity, id)` | remove a trait from an entity — unlearning, an enchantment stripped. Bumps. |
| `present(entity)` | the entity's trait ids, in dependency order. What `all` iterates. |

### Changed

| Function | Change |
|---|---|
| `value(entity, id)` | absent → `def.default` (0 for most), **without materialising it**. Arithmetic stays safe; `has` answers the other question. |
| `all(entity)` | only present traits — so `score` on a sword stops listing willpower |
| `set_base(entity, id, n)` | on an absent trait, **creates** it. This is how a skill is learned. |
| `recompute` | walks the entity's present set, not the global order |

### What a trait *is* — `category`

`group` cannot carry this. It exists today (`trait_d.lua:130`) and is already
plumbed through `all()` (`:409`), but it is being used as a *display heading* —
"Attributes", "Vitals", "Derived" — which tracks `kind` more than meaning. Three
different questions are hiding in one field:

| Axis | Answers | Status |
|---|---|---|
| `kind` | what is stored and how it is computed — attribute / derived / gauge / counter | exists, load-bearing for the engine |
| **`category`** | **what this number *is* in the game's vocabulary** — stat, skill, resource, condition, reputation | **new** |
| `group` | which heading it sorts under *within* a command | exists, stays presentational |

They genuinely separate. `swordsmanship` is a counter; `sword_mastery` is a
derived percentage over it. Different `kind`, same `category = "skill"`, and
both belong in `skills` under `group = "weapon"`. No single field expresses that.

```lua
{ id = "swordsmanship", kind = "counter", category = "skill", group = "weapon",
  min = 0, max = 100 }
{ id = "strength", kind = "attribute", category = "stat", group = "attributes" }
{ id = "durability", kind = "gauge", category = "condition", sets = {"item"} }
```

Three rules keep it from becoming a second `kind`:

- **`category` is freeform**, like a permission string or an area name. The
  mudlib defines no closed list; a game invents `reputation` or `mastery`
  without touching the driver.
- **It defaults to `"stat"`**, so every trait defined today keeps appearing in
  `score` with no edit — the same migration property `sets` gets.
- **It never changes behaviour.** It is a lens for commands, nothing more. The
  moment a category is tempted to *mean* something — "skills advance by use" —
  that belongs on the spec as its own declared field (`advances = "use"`), not
  implied by a string. Adding a category must not be able to break anything.

**Commands name what they show**, rather than the trait naming where it goes:

| Command | Shows |
|---|---|
| `score` | `category == "stat"`, grouped by `group` |
| `skills` | `category == "skill"`, grouped by `group` |
| `traits` (admin) | everything, with `kind`, `category`, `set` and presence — the discoverability answer, and where a mis-categorised trait shows up |
| item `examine` | `category == "condition"` on the item |

A trait in a category no command names appears nowhere until someone writes the
command. That is the correct default: new categories should not silently leak
into `score`, and `traits` is always there to find them.

> Not folded into `sets`, though the question is fair — they correlate. `sets`
> decides what gets *seeded*; `category` decides what gets *shown*. A skill is
> explicitly never seeded (not having swordsmanship is the point of sparseness),
> so a `skill` set would be an empty seed list used only as a tag. Overloading
> one field to mean both would make that contradiction permanent.

### `Mobile:stat` → `Object:trait`

`stat` is a narrower word than the concept, and once one call answers for
swordsmanship and durability it is the wrong one. Rename to `trait(id)`, and
move it from `Mobile` (`mobile.lua:114`) **up to `Object`**, so rooms, items,
mobs and players all have it — which is the point.

23 call sites in `mudlib/` and `game/` (`combat_d`, `death_d`, `gmcp_d`,
`affect`, `score`, …). A hard rename, no alias: this codebase deleted
`create_sandboxed_env` to have one boundary rather than two, and a compatibility
alias is the same debt.

---

## Part 3 — Keeping recompute O(entity)

The trap: filtering the global order by presence on every recompute is still
O(all defined traits) and buys nothing.

**Per-entity present-set cache**, living beside the memo and validated by the
same two counters (`r.gen`, `_entity_gen[entity]`) that already guard it. Both
already move when they need to — `set_base` bumps the entity, `define` bumps the
generation.

Building the set on a miss:

1. Stored kinds — iterate `entity.stats` keys. O(what the entity holds).
2. Derived — test each derived def's `depends` against the set, to fixpoint.
   O(derived defs), and derived traits are the small population; skills and
   masteries are counters.
3. Order it. `seal()` assigns `def.rank = i` from the global topological order;
   sort the present list by rank. O(k log k) for small k, and deterministic —
   which the effects system's ordering guarantee depends on.

For a player with 40 skills out of 400 defined traits and 10 derived: ~50 on a
miss, ~50 on a hit, against 400 today.

*Optional later:* index derived traits by dependency at seal time so step 2 only
considers those whose deps intersect the entity's stored keys. Worth it only if
derived traits get numerous; note it, don't build it.

---

## Part 4 — Migration

**Existing characters need none.** They already have all eleven traits
materialised, so every one is "present" and behaviour is identical. The golden
test below pins that.

**`Mobile.skills` is deleted.** It exists as a parallel `skill -> level` map
(`mobile.lua:96, 331-339`) precisely because traits could not be sparse. A skill
becomes a counter the entity happens to hold, gaining clamping, bounds and a
derived mastery percentage for free.

- `get_skill`/`set_skill` become `trait`/`set_base`, or thin deprecated wrappers
  deleted in the same pass.
- `Player:from_save` (`player.lua:123-125`) migrates a saved `skills` table into
  `stats` on load. `skills` leaves `SAVE_FIELDS`; storage moves, nothing is lost.
- `objdump.lua:119` drops its Skills line — they appear under traits.

**`score` will flood** once a player knows forty skills. `group` already exists
on every spec and is already carried through `all()`; `score` should render the
non-skill groups and a separate `skills` command take the skill group. Follow-on
work, not part of this change, but decide it before authoring skills.

---

## Part 5 — Build order

| Step | Work |
|---|---|
| **1** | `rank` in `seal()`; `present(entity)` and its cache; `recompute` over the present set. No API change yet — existing entities have everything present, so all current tests must stay green untouched. This is the whole performance change, isolated. |
| **2** | Presence rules proper: `value` stops materialising, `all` filters, `has` / `forget` / `always`, undefined-id values inert. |
| **3** | Split `attach` into `attach` + `seed`; `sets` on the spec defaulting to `{"character"}`. |
| **4** | `Mobile:stat` → `Object:trait` across 23 sites. Mechanical, do it in one pass. |
| **5** | `category` on the spec, defaulting to `"stat"`. `score` filters to it — no visible change, because everything defaults there. Add the `traits` admin command, which is how the next step gets checked. |
| **6** | Delete `Mobile.skills`; migrate `Player:from_save`; drop the objdump line; add the `skills` command over `category == "skill"`. |

Step 1 is the one worth landing on its own — it is the performance fix, it
changes no behaviour, and if anything in it is wrong the existing suite says so
immediately.

---

## Part 6 — Verification

`tests/trait_sparsity.rs`, through the real VM per CLAUDE.md:

- **Presence** — an item with `stats = { durability = 100 }` has `durability`
  and not `wisdom`; `all()` returns one trait; `score` on it lists one trait.
- **Derived presence follows deps** — define `dps` over `damage`/`speed`; an
  item with both has it, an item with one does not.
- **Bounds** — a gauge whose `max` names an absent trait is itself absent.
- **Learning** — `set_base(player, "swordsmanship", 1)` on a previously absent
  trait creates it; `has` flips false → true; `forget` reverses it.
- **Absent reads** — `value` returns the default and **writes nothing**; assert
  `stats.swordsmanship` is still nil afterwards.
- **Inert unknowns** — a stats table holding an undefined id reads as absent,
  then becomes present when the trait is defined at runtime.
- **Seeding** — `seed(entity, "character")` materialises the character set and
  nothing else.
- **Categories are a lens, not behaviour** — the same trait defined under two
  different categories computes an identical value, settles identically, and
  differs only in which command lists it. This is the test that stops `category`
  quietly becoming a second `kind`.
- **Command routing** — a `category = "skill"` trait appears in `skills` and not
  in `score`; a `category = "stat"` trait the reverse; a trait in a category no
  command names appears in `traits` only.

**The performance property, tested behaviourally not by timing.** Count formula
evaluations: define 200 derived traits, recompute an entity holding 2, assert
the evaluation count is proportional to the entity and not to the registry. A
timing assertion would be flaky; a call count is exact.

**Regression:**

- `tests/traits_effects.rs` and `tests/combat.rs` green with no edits — that is
  the migration proof.
- **Golden `score`**: capture a seeded character's full `score` output before
  and after the whole change and diff. Identical, or the migration is wrong.
- A skill round-trips through save/load after the `Mobile.skills` deletion.

Docs: `docs/src/lua-api/traits.md` needs the presence rule, `has`/`forget`/
`seed`, the `sets` and `category` fields, the three-axis table
(`kind`/`category`/`group`), and the `Object:trait` rename; the "What it will
not do" section loses "no per-trait invalidation" as the reason it gives, since
the present-set cache is exactly that bookkeeping arriving. `object-hierarchy.md`
gains `trait()` on `Object`. `CLAUDE.md`'s "Stats are read through
`player:stat(id)`" line needs updating.
