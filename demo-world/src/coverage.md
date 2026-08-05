# Coverage Matrix

Every subsystem, the content that exercises it, and the test that pins it. This
is the table the demo world was built from — read backwards, it is a list of
things that had no user until something in the game needed them.

## World & rooms

| Feature | Vehicle | Test |
|---|---|---|
| Data-oriented areas, `load_area` | every area | `world_graph.rs` |
| `ROOM_D.merge`, multi-file areas | Thornhollow's three files | `thornhollow.rs` |
| `_meta` (name, title, level, status) | all four areas | `thornhollow.rs` |
| lfun descriptions | the marsh (weather), the cauldron, the levers | `marsh.rs`, `mine.rs` |
| lfun `sound` | the causeway head in fog | `marsh.rs` |
| `light` 0–3, dark rooms | mine levels 1–3, the crypt | `mine.rs` |
| `smell` / `sound` | every room in the marsh and mine | — |
| Scenery `items` | every room has ≥ 2 | — |
| Room actions, dispatch precedence | `drink` at the well | `thornhollow.rs` |
| Rich exits with a `check` | the mine grille, the sump shaft | `virtual_rooms.rs` |
| Exits, opposites, all ten directions | the whole map | `world_graph.rs` |
| **Virtual rooms** | `reach.X.Y` | `virtual_rooms.rs` |
| **Eviction** | last occupant leaves | `state_retention.rs` |
| Area reset | the mine puzzle clears, the herb gate does not | `mine.rs` |
| Object state | door, levers, seam, cauldron, flagstone | `mine.rs` |
| Room tags + reverse index | `outdoor`, read by the weather | `thornhollow.rs` |

## Items, equipment, economy

| Feature | Vehicle | Test |
|---|---|---|
| Item **instances** with a location | everything on a floor | `items_ground.rs` |
| `on_use` / `on_pickup` / `on_drop` | the lantern, hooked test items | `items_ground.rs` |
| `weapon` component | 5 weapons, incl. two-handed and `magic` | `equipment.rs` |
| `armour` — `defense`, `resist`, `stat_bonus` | jerkin, cloak, circlet, buckler | `equipment.rs`, `combat_mitigation.rs` |
| `requires` — level, strength, dexterity | greatsword, silver dagger, pick | `equipment.rs` |
| Slots, two-handed displacement | greatsword vs buckler | `equipment.rs` |
| `drinkable` | draught, antidote, purple potion | `traits_effects.rs` |
| **Containers** | backpack, vault, corpse | `items_ground.rs`, `mine.rs` |
| Container contents through save/load | the backpack | `lifecycle.rs` |
| Containment cycles refused | `put bag in box; put box in bag` | `items_ground.rs` |
| Shops, prices, the gold sink | three shops, three rates | `shop.rs` |
| `spend_gold` refusing | buying while broke | `shop.rs` |
| Restock on a **task** | `tasks run shop.restock` | `shop.rs` |
| A ledger over `db_*` | every transaction | `shop.rs` |
| Loot tables | every hostile creature | `items_ground.rs` |
| Light sources, per instance | two lanterns disagreeing | `mine.rs` |

## Creatures & combat

| Feature | Vehicle | Test |
|---|---|---|
| Templates, `count`, `populate` idempotency | 21 creatures | `thornhollow.rs` |
| `spawn` / `despawn` / `respawn_time` | all of them | `state_retention.rs` |
| `find_in_room` prefix match | `attack lur` | — |
| **`aggressive`** | lurkers, crawlers, the Wisp, the Delver | `marsh.rs` |
| **`stationary`** | shopkeepers, guards | `thornhollow.rs` |
| **`unique`** | the Wisp, the Delver, the watchman | `thornhollow.rs` |
| **`patrol`** | the night watchman | — |
| **`echoes`**, weighted and lfun | the drunk, the apprentice | `thornhollow.rs` |
| **`dialogue`**, incl. lfun answers | every named NPC | `thornhollow.rs` |
| **`faction`** | the two guards | `thornhollow.rs` |
| **`on_combat`** | the lurker's bite, the Delver's curse | `marsh.rs` |
| **`on_death`** on a template | the Delver's corpse | `mine.rs` |
| `damage_taken` pipeline with real armour | any fight while wearing something | `combat_mitigation.rs` |
| Damage types vs resists | silver dagger / warded cloak | `combat_mitigation.rs` |
| Death, respawn, XP | dying anywhere | `traits_effects.rs` |
| **XP → levels** | `level_d`, and `player.levelup` | `levelling.rs` |

## Traits

| Feature | Vehicle | Test |
|---|---|---|
| `attribute` | 7 of them | `trait_sparsity.rs` |
| `counter` | level, and 5 skills | `trait_sparsity.rs` |
| `derived` | max_hp, max_mp, willpower, carry_capacity | `traits_breadth.rs` |
| **Derived-of-derived** | `spell_power` ← willpower; `max_stamina` ← carry_capacity | `traits_breadth.rs` |
| `min`/`max` as another trait's id | `hp.max = max_hp`, `stamina.max = max_stamina` | `traits_breadth.rs` |
| All four `round` modes | reflex, resolve, presence, attunement | `traits_breadth.rs` |
| `hidden` | `luck_seed` | `traits_breadth.rs` |
| Gauge regeneration, remainder carry | hp, mp, stamina | `traits_effects.rs` |
| `offline` per gauge | stamina yes, hp no | `traits_breadth.rs` |
| **Sparse presence** | a sword has no willpower | `trait_sparsity.rs` |
| `category` as a lens | `score` vs `skills` vs `traits` | `trait_sparsity.rs` |
| Skills as traits | herbalism, mining, swordsmanship… | `trait_sparsity.rs` |
| `seal` reporting a cycle **as a path** | `traits/broken_example.lua` | `traits_breadth.rs` |
| Memoization, `bump_all` on reload | any repeated read | `traits_breadth.rs` |

## Effects

| Feature | Vehicle | Test |
|---|---|---|
| `modifiers` sugar | stoneskin, wardskin | `traits_effects.rs` |
| Explicit phases `mult` / `reduce` | armour vs stoneskin | `combat_mitigation.rs` |
| Multipliers add, not compound | two XP buffs | `traits_effects.rs` |
| `tick` on the shared heartbeat | marsh fever, regeneration | `marsh.rs` |
| `duration`, lazy expiry, `sweep` | any timed buff | `traits_effects.rs` |
| All five stack modes | across the effect files | `traits_effects.rs` |
| `condition` over `lib/checks.lua` | marsh chill, wardskin | `marsh.rs`, `traits_breadth.rs` |
| **`survives_death`** | the Wisp's mark, the Delver's Regard | `marsh.rs`, `mine.rs` |
| `persist = false` | every `equip:` aura | `equipment.rs` |
| Source schemes | `equip:` `spell:` `quest:` `room:` `mob:` `admin:` | `equipment.rs`, `quests.rs` |

## Events, timers, tasks

| Feature | Vehicle | Test |
|---|---|---|
| `emit` / `on` / priority | `mob.died` → loot, XP, quests | `quests.rs` |
| **`room.entered` / `room.left`** | aggro, quest visits | `marsh.rs`, `quests.rs` |
| **`player.login` / `player.logout`** | levelling catch-up | `levelling.rs` |
| **`player.levelup`** | `level_d` | `levelling.rs` |
| `item.*` events | every item verb | `items_ground.rs` |
| Area-scoped events | `area.collapsed_mine.delver_slain` | `mine.rs` |
| `ticker.after`, arming by id | the lever reset, corpse rot | `mine.rs` |
| **`task_d`** — schedule/pause/run_now | shop restock, board sweep | `shop.rs` |

## State tiers

| Tier | Vehicle | Test |
|---|---|---|
| memory | combat targets, weather, aggro | `state_cache.rs` |
| write-behind | quest kill counters | `quests.rs` |
| write-through | the shop ledger | `shop.rs` |
| character (`SAVE_FIELDS`) | traits, gold, quest flags, equipment | `lifecycle.rs` |
| **durable vs memory cooldowns** | herb beds (24 h) vs spells (6 s) | `marsh.rs`, `traits_breadth.rs` |
| The document store, all 12 `db_*` | the notice board + the ledger | `board.rs`, `shop.rs` |

## Interface & protocol

| Feature | Vehicle | Test |
|---|---|---|
| Prompt templates | `prompt %h/%H %m/%M >` | `traits_effects.rs` |
| Colour, and `color off` | everything | — |
| Pager | `help`, `board` | — |
| Channels + name-as-verb | `chat`, `staff` (gated) | — |
| GMCP outbound | Char.Vitals/Status/Effects, Room.Info | `gmcp_inbound.rs` |
| **GMCP inbound** | `Core.Supports.Set`, `Core.Hello`, `Core.Ping` | `gmcp_inbound.rs` |
| **A custom package** | `Game.Quest` | `gmcp_inbound.rs` |
| NAWS-aware wrapping | `Player:get_width` | `gmcp_inbound.rs` |

## Admin, building, observability

| Feature | Vehicle | Test |
|---|---|---|
| RBAC, roles declared in a file | `game/setup_roles.lua` | `staff.rs` |
| `role grant/revoke/allow/deny/refresh` | the `role` command | `staff.rs` |
| A grant reaching an online session | | `permission_refresh.rs` |
| `permissions.toml` `[directories]` | `/areas`, gated | `staff.rs` |
| `get_account` | `finger` | `staff.rs` |
| Superuser bypass | account 1 | `permission_refresh.rs` |
| OLC round trip | `olc` / `dig` / `codegen_d` | — |
| Journal + audit | every admin action | `observability.rs` |
| **Heap and GC counters** | `mudstatus`, `mudstatus gc` | `state_retention.rs` |
| Hot reload | `reload` any daemon | `hot_reload.rs` |

## Compute & sandbox

| Feature | Vehicle | Test |
|---|---|---|
| `compute()` off-thread | `navigate` | `virtual_rooms.rs` |
| **Revalidation of a stale result** | `still_connected` before walking | `virtual_rooms.rs` |
| Worker VMs have no efuns | the pathfinder | `virtual_rooms.rs` |
| Sandbox — io, os.execute, debug, jit | — | `sandbox_reality_check.rs` |
| Path traversal | `list_dir("../..")` | `list_dir_jail.rs` |
| `lua_instruction_limit` | — | `instruction_limit.rs` |
| PRNG seeded per VM | combat, loot, echoes | `sandbox.rs` |

## Deliberately not covered

Called out so their absence is a decision rather than an oversight:

- **Initiative, groups, positioning, ranged combat, pursuit.** `combat.md` puts
  these out of scope; the round loop is one attacker, one target.
- **MCCP2 compression.** Negotiated and not performed — see
  `docs/src/protocols/mccp.md` for why.
- **PostgreSQL.** The enum value parses and has no runtime path.
- **Stacking item instances.** Two instances are two objects.
- **`while true do pcall(...) end`** still wedges a worker; documented in
  `sandboxing.md`.
