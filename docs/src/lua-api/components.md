# Components

An item that is a container **and** a light source **and** a quest token has no
single class it could be. So items are not a class hierarchy: they are an
`Item` with components bolted on, and every role an item can play is one.

`components/weapon.lua` has cited this page since it was written. This is it.

## The three parts

| | Holds | Rule |
|---|---|---|
| **Archetype** `Weapon{...}` | construction | flat authoring data in, an `Item` out |
| **Component** `item.weapon` | data | no functions, no metatables; **its presence is the has-component test** |
| **System** `components/weapon.lua` | behaviour | module functions taking the item — never installed on the instance |

All three live in one file, deliberately. Splitting them across directories
would mean three files to open to answer one question about weapons.

`item.weapon ~= nil` **is** the has-component test. There is no `is_weapon`
flag to keep in step with anything, and no `applies_to` list to rot — the same
reasoning that makes [traits](./traits.md) decide presence by storage.

## What a component module looks like

```lua
-- mudlib/components/glowing.lua
local M = {}

M.component = "glowing"   -- the field this owns on an item
M.order = 35              -- where its lines sort in `examine`

function M.from_data(data)              -- flat authoring data → component
    return { colour = data.glow_colour or "white" }
end

function M.apply(item, data)            -- the archetype
    item.glowing = M.from_data(data)
    return item
end

function M.is(item)                     -- the predicate
    return type(item) == "table" and type(item.glowing) == "table"
end

function M.describe(item, ctx)          -- what `examine` shows. Unindented.
    if not M.is(item) then return {} end
    return { "It glows " .. item.glowing.colour .. "." }
end

return M
```

Drop that in `mudlib/components/` and you are done. Nothing registers it,
nothing imports it, and `examine` shows its line — which is the whole point of
the index below.

Only `component` and `is` are required. `order` defaults to 50.

### `describe(item, ctx)`

Return an array of **unindented** strings; the caller owns layout. `ctx` carries
what a component may need beyond the item:

| | |
|---|---|
| `ctx.instance_id` | for anything reading per-instance object state — whether *this* chest is open, as opposed to the template |
| `ctx.viewer` | for anything whose answer depends on who is asking — whether you meet a requirement |

Both may be absent. `examine` on a shop's stock is a fair question with no
viewer in it, so a component that wants one must cope without.

### `equip_specs(item, ctx)`

Optional. What wearing the item contributes, as `set_source_effects` specs.
`components/armor.lua` is the worked example: it is what turns `defense` and
`resist` from data nobody reads into a real `damage_taken` handler.

The two effect factories arrive through `ctx` — `ctx.trait_effect(trait_id)`
and `ctx.protection_effect()` — because those definitions belong to
`lib/equipment.lua`. A component does not get to require its own consumer.

## The index

`require('components')` is `mudlib/components/init.lua`, which finds every
component by listing the directory — the same shape `lib/commands.lua` uses for
commands.

| | |
|---|---|
| `all()` | every module, in `order` |
| `get(name)` | one by the field it owns — `get("armour")`, note, not `"armor"` |
| `on(item)` | the components this item actually has |
| `describe(item, ctx)` | every line every component has to say |
| `equip_specs(item, ctx)` | every spec every component contributes |
| `flush_cache()` | after a hot reload |

Order is **declared**, not discovered. The filesystem has no opinion on whether
a sword's damage should print above or below its strength requirement, and the
answer should not change when somebody renames a file.

A component whose `describe` raises is logged and skipped; the others still
print. One bad edit should not take the whole of `examine` with it.

## The components that ship

| | Owns | Archetype |
|---|---|---|
| [`weapon`](./items.md) | `item.weapon` | `Weapon{...}` |
| `armour` | `item.armour` — note the file is `armor.lua` | `Armor{...}` |
| `container` | `item.container` | `Container{...}` |
| `drinkable` | `item.drinkable` | `drinkable.apply(item, {...})` |
| `requires` | `item.requires` | attached by `Weapon`/`Armor` from `required_*` fields |

Two are worth a closer look:

- **`container`** keeps nothing about its contents. Those live in `item_d`'s
  location index under `item:<instance id>`, so a container holds things the
  same way a room does and `put`/`get from` are the same code path as
  `drop`/`get`. Open, closed and locked are per-instance object state for the
  same reason: two backpacks from one template must be able to disagree.
- **`requires`** has no archetype of its own and its `from_data` returns `nil`
  when nothing is required, so `item.requires` stays *absent* rather than
  becoming an empty table indistinguishable from a real constraint. `met()`
  answers `true` for an item with no component, so no caller has to check first.

## The one exception to "no functions"

A component holds data. The exception, stated rather than hidden: `item.weapon`
carries `hit_message`, `miss_message` and `crit_message`, which may be lfuns.
They are authoring text that happens to be conditional, they live on the
*template* and are never copied onto an instance, and the alternative — a
parallel message table keyed by weapon id — is worse. `on_drink`, `on_use` and
the other item hooks are top-level fields for the same reason.

## What is not a component

`lib/light.lua` reads `room.light_level` and item state and answers one
question: can you see. That is a **system with no component of its own** — a
policy module, not a mixin — so it stays in `lib/`. `lib/equipment.lua`,
`lib/carry.lua`, `lib/item.lua` and `lib/object.lua` are consumers, not
components.

The test is not "is it optional" but "does it attach data to an object". If
there is no `item.<something>` it owns, it is not a component.
