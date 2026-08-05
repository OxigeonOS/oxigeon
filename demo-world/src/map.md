# The Whole Map

Twenty-seven authored rooms in four areas, plus a generated grid. Every exit
below is real — this is dumped from `DAEMON.world.exit_graph()` rather than
drawn by hand, and `tests/world_graph.rs` asserts it stays connected.

```
                          ┌─────────────────────┐
                          │  WIZARD'S WORKSHOP  │   you start here
                          └─────────────────────┘

                                  archive
                                     │ s/n
        pantry ─── e/w ───────── laboratory ───── e/w ───── scrying chamber
                                     │ s/n
                                  entrance
                                     │ e
                                     │            (treasure vault: no exits;
                                     │             leave by touching the orb)
   ═══════════════════════════════════╪═══════════════════════════════════════
                                     │ w
                                 undercroft ── e/w ── crypt        THORNHOLLOW
                                     │ up
                              undercroft stair
                                     │ up
   marsh ◄── w ── west gate ── e/w ── SQUARE ── e/w ── market ── n/s ── Hobb's
                                     │  │                 │
                                     │  └─ n/s ─ tavern   └─ s/n ─ apothecary
                                     │ n
                                  smithy
                                     │ down
   ═══════════════════════════════════╪═══════════════════════════════════════
                                     │                            COLLAPSED MINE
                                   adit
                                     │ down
                              first level  ── mine the seam
                                     │ down
                             second level ── e/w ── pump house  ── the levers
                                     │ w  (locked grille)
                              deep workings
                                     │ down  (needs the pump)
                                 the sump  ── the Delver
```

```
   ═════════════════════════════════════════════════════════════════════════
                                                              GREYWATER MARSH
   west gate ── w ── causeway head ── w ── causeway mid ── w ── stilt village
                                              │ n                    │ w
                                          herb beds              deep water
                                                                     │ w
   ═══════════════════════════════════════════════════════════════════╪═════
                                                            THE DROWNED REACH
                                                                 reach.0.0
                                                            (81 × 81, generated)
```

## Every exit, exactly

| From | Exits |
|---|---|
| `wizard_workshop.entrance` | north → laboratory · **east → thornhollow.undercroft** |
| `wizard_workshop.laboratory` | south → entrance · north → archive · west → pantry · east → scrying_chamber |
| `wizard_workshop.pantry` | east → laboratory |
| `wizard_workshop.archive` | south → laboratory |
| `wizard_workshop.scrying_chamber` | west → laboratory |
| `wizard_workshop.treasure_vault` | *(none — the orb returns you)* |
| `thornhollow.undercroft` | up → stair · east → crypt · **west → wizard_workshop.entrance** |
| `thornhollow.crypt` | west → undercroft |
| `thornhollow.undercroft_stair` | up → square · down → undercroft |
| `thornhollow.square` | north → smithy · east → market · south → tavern · west → west_gate · down → stair |
| `thornhollow.smithy` | south → square · **down → collapsed_mine.adit** |
| `thornhollow.tavern` | north → square |
| `thornhollow.market` | west → square · north → general_store · south → apothecary |
| `thornhollow.general_store` | south → market |
| `thornhollow.apothecary` | north → market |
| `thornhollow.west_gate` | east → square · **west → greywater_marsh.causeway_head** |
| `greywater_marsh.causeway_head` | east → thornhollow.west_gate · west → causeway_mid |
| `greywater_marsh.causeway_mid` | east → causeway_head · west → stilt_village · north → herb_beds |
| `greywater_marsh.herb_beds` | south → causeway_mid |
| `greywater_marsh.stilt_village` | east → causeway_mid · west → deep_water |
| `greywater_marsh.deep_water` | east → stilt_village *(west → `reach.0.0` when you walk it)* |
| `collapsed_mine.adit` | up → thornhollow.smithy · down → first_level |
| `collapsed_mine.first_level` | up → adit · down → second_level |
| `collapsed_mine.second_level` | up → first_level · east → pump_house · **west → deep_workings** *(locked)* |
| `collapsed_mine.pump_house` | west → second_level |
| `collapsed_mine.deep_workings` | east → second_level · **down → the_sump** *(needs the pump)* |
| `collapsed_mine.the_sump` | up → deep_workings |

The bold ones are the four **inter-area** links. Three of them did not exist
until this guide was written.

## Where the creatures are

| Room | Who | Level | Aggressive |
|---|---|---:|---|
| `wizard_workshop.pantry` | 2 × grey rat | 1 | no |
| `wizard_workshop.laboratory` | dust mephit | 3 | no |
| `thornhollow.smithy` | Bellow, apprentice | 8, 2 | no |
| `thornhollow.general_store` | Hobb | 3 | no |
| `thornhollow.apothecary` | apothecary | 4 | no |
| `thornhollow.tavern` | the drunk | 2 | no |
| `thornhollow.square` | the watchman *(patrols)* | 5 | no |
| `thornhollow.west_gate` | 2 × guard | 6 | no |
| `greywater_marsh.herb_beds` | 3 × reed crawler | 3 | **yes** |
| `greywater_marsh.stilt_village` | 2 × marsh lurker | 5 | **yes** |
| `greywater_marsh.deep_water` | the Wisp *(unique)* | 10 | **yes** |
| `collapsed_mine.first_level` | 2 × mine crawler | 7 | **yes** |
| `collapsed_mine.deep_workings` | shale lurker | 9 | **yes** |
| `collapsed_mine.the_sump` | the Delver *(unique)* | 15 | **yes** |

Nothing in town is aggressive. Everything past the gate is.

## Levels

Experience becomes levels through `game/daemons/level_d.lua`, which listens to
`player.xp_gained`. The curve is a table rather than a formula, so a designer can
see that level 4 costs 450 and decide whether that is the right moment for the
mine:

| Level | Total XP | Roughly |
|---:|---:|---|
| 2 | 100 | the apothecary's quest |
| 3 | 250 | that plus a few crawlers |
| 4 | 450 | the guard's quest |
| 5 | 700 | the delivery quest |
| 7 | 1,400 | the mine's first level |
| 10 | 3,200 | the Wisp, and the chain quest unlocks |
| 15 | 8,700 | the Delver is a fair fight |

Levelling refills your gauges, because `max_hp` and `max_mp` are derived from
level and the ceilings have just moved. That is one line, because a gauge's
bound is an ordinary trait.
