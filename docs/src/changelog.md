# Changelog

## Phase 4: Two Lua Runtimes

### Which one did they mean

- **Two identical creatures in a room were unreachable past the first.**
  `attack rat` took the first match and there was no way to name the second;
  same for two identical swords on the floor. `lib/matching.lua` understands
  `2.rat`, and `mob_d`, `item_d` and `lib/carry.lua` all go through it — two
  matchers disagreeing about which sword you meant is the same failure as two
  string-to-value converters.
- **A bare keyword matching several things refuses, with the list**, rather than
  taking the first and hoping. Guessing wrong on `attack` starts the wrong
  fight, and the player who has no time to pick is served without a name at all:
  `attack` and every hostile ability aim at what you are already fighting.
- The ordinal is a **position, recomputed per command**. A number stored on the
  thing would leave gaps — `2.rat` gone, `3.rat` still there — and a gap is more
  disorienting than a shift. `mob_d.in_room` now sorts the instance sequence
  *numerically*; it compared the whole id as a string, so `mob:10` came before
  `mob:2` and the order stopped being spawn order after nine of anything. With
  that fixed, `1.rat` is the oldest rat present.
- **A `stackable` item is never ambiguous**, because interchangeable is what
  stackable means. Read off the declared property rather than from a list of
  commands allowed to skip the question.
- `Carry.find{ any = true }` is code choosing rather than a player choosing — a
  quest taking three marshroots named them itself and has nobody to ask.
- The disambiguation list prints **shorts regardless of
  `game.display_name_prefers`**. That key is about prose voice; a list whose job
  is to tell three creatures apart must not print `rat` three times.

### Two smaller ones from the same session

- **A hostile ability aims at your current fight without being told.**
  `perf emberlance` answered "At what?" with a rat biting you. `cleave` declared
  `default_target = "combat"` and nothing else did, so it is defaulted from the
  declared *outcome* now — an ability that attacks or damages a creature aims at
  your fight; one that heals does not. A list of ability ids would rot.
- **`display_name_prefers` is `"short"`.** Three creatures all reporting as
  `rat` is unreadable the moment a nest puts three of them in one room.

### Weapon speed, encumbrance, and a clock fine enough to tell them apart

- **`speed` was authored on every weapon in the game and read by nothing but
  `examine`.** It is a *rate* — the dead `weapon.dps` helper is
  `avg_damage * speed`, which only type-checks that way — so the time a swing
  costs is `round_length / speed`, and every authored number already meant the
  right thing. `auto_round` marks `{ rounds = 1 / speed }`, which is one line
  and no new concept: `{ rounds = n }` was already multiplicative.
- **Two layers, and keeping them apart is the design.** `round_length` is the
  entity's clock and gates every action on the track; `speed` is the action's
  cost in rounds and only affects swings. So armour belongs on `round_length`
  and counts for more than a weapon without needing a bigger coefficient — it
  taxes everything you do.
- **`encumbrance` is paid for out of strength.** An attribute fed by
  `stat_bonus` on worn pieces, with `round_length` charging only what exceeds
  `strength * 1.5`. A flat penalty per armour class would cost a wizard and a
  knight the same, which is the opposite of true.
- **A weapon can carry a `stat_bonus`.** `lib/equipment.lua` had a top-level
  `item.stat_bonus` branch commented "a weapon may carry stat bonuses too" since
  it was written, and it was unreachable: `Item:new` copies a fixed list of
  fields and `stat_bonus` was not among them. The weapon component contributes
  its own equip specs now, the way armour does. **Fourth instance** of that
  hazard — `on_drink`, the room spawner fields and `mob.speed` were the others —
  and every constructor involved now says so in a comment.
- **A creature has its own attack rate.** Without it a creature's rate came from
  `round_length` alone, which moves 0.05s per point of dexterity, so a rat and a
  bear swung within a tenth of a second of each other. The workshop's rats bite
  at 2.5 to a swordsman's 1.
- **`game.queue_tick_seconds` was 1 second and quantised all of it.** A player
  at 3.0s and a rat at 2.9s came free on the same tick and traded blows in
  perfect lockstep — nothing wrong anywhere, a clock too coarse to tell them
  apart. It is 0.25 now.

### Waiting enqueues; being unable refuses

- **A cooldown queues rather than refusing.** The rule was that only roundtime
  enqueued, on the argument that a cooldown says "not this, for a while" where
  roundtime says "not yet, but soon and certainly". True, and not a distinction
  the player is in a position to make: from the seat `Not yet. (1s)` and `You
  will emberlance next` are the same situation, and two behaviours out of one
  intent reads as arbitrary. A cost you cannot pay and a mistyped target are
  *unable* rather than waiting, and still refuse.
- `queue_d`'s resolvers may return `"retry"` to be put back at the head rather
  than dropped. `advance` pops before resolving, so without it a queued ability
  whose cooldown had not cleared was popped, refused and silently lost.
- **`emberlance` costs a round.** It declared no `roundtime`, so it never marked
  the combat track and `auto_round` went on swinging your fist on its own clock
  — you cast *and* punched, in the same second.

### Spawners

- **A room can produce creatures**, which is a different statement from a
  creature saying where it lives, and both now exist. `spawn_max`,
  `spawn_interval` and `spawn_table` are room schema fields, so a spawner is
  authored and edited in OLC with no new generated file kind. The thing that
  could not be said before is the **cap across kinds**: `mob_d.populate()`
  counts per template, so three rat templates at `count = 2` is six rats and
  there was no way to write "six rats of any kind is too many for one pantry".
- Filled to `spawn_max` at load and a trickle of one per interval afterwards. A
  server that has just started should not have empty rooms for
  `max × interval` seconds, and a cleared room should refill at a rate the
  player can outrun; those are different needs and one rule cannot serve both.
- The cap counts **the spawner's own kinds**, not every occupant — otherwise a
  player switches a nest off by luring something unrelated into the room.
- One `ticker_d` heartbeat for every spawner in the world, each keeping its own
  `due`. The index is fed from `world_d.register_room`, beside the `tag_d` call
  and for the same reason: a room entering the world is the one moment every
  path goes through, so an index fed there cannot drift.
- `verify` reports a half-spawner, a table naming a creature that does not
  exist, a zero weight, and the one that would have been silent — a template in
  a spawn table that *also* carries `respawn_time` or `spawn_room`, and is
  therefore fed by two sources and drifts past its cap one kill at a time.
- The workshop's `workshop_rat` is gone, replaced by a nest in the pantry and a
  `vermin.rat` prototype with three children. Each rat is `{ id, prototype }` in
  `mobs.lua` and inherits everything else — which is the shortest worked example
  of either feature in the demo world. See [Spawners](./lua-api/spawners.md).

### A keyed `record_array` is settable in OLC

- `loot_table`, `echoes` and `spawn_table` are all `record_array`, and **none of
  them could be set in OLC at all**: `schema.set` refused the type outright and
  pointed at a `<field>.<key>` syntax that did not work. A builder could make a
  room with a spawner and never fill in what it spawned.
- A record now declares which field is its address with `key = true`, and
  `olc set spawn_table.black_rat 5` finds or appends that entry. Declared rather
  than inferred: "the first field, if it looks like an id" reads the wrong one
  the first time somebody writes a record in a different order, and reads it
  silently. `echoes` declares no key — its address would be a whole sentence —
  so it is refused and told which file to write it in.
- **`olc set` printed `= (unset) (was (unset))` for every dotted set**, because
  it read `draft[descriptor.name]` and a path descriptor's name *is the path* —
  so it was reading `draft["exits.north"]`, which nothing ever writes. Wrong for
  maps since they existed; `schema.at` reads a path properly now.
- `Room:new` copies a fixed list of fields, so the three spawner fields had to be
  added to it — the same hazard that lost `drinkable`'s `on_drink`, noted in a
  comment on both constructors. A field the schema knows, `verify` checks and the
  generated file round-trips can still be silently absent from the object the
  game uses.

### Layer boundaries, and the areas becoming editable

- **`aggro_d`, `board_d` and `quest_d` moved to the mudlib.** The test that
  decided it: a correctly game-specific daemon **names things**. `reach_d` names
  a room id and an area, `weather_d` names reeds and shutters, `gmcp_game_d`
  names a package only this game has. None of the three movers named anything.
  A mudlib shipping `Mobile.aggressive`, `Mobile:is_aggressive()` and a
  `room.entered` event with nothing that reads them is a mudlib with a hole in
  it. `aggro_d`'s two constants became `game.aggro_delay_seconds` and
  `game.aggro_ignore_level_gap`; `board_d`'s categories and lifetime became a
  `DAEMON.board.configure{…}` call from `game/init.lua`. **`spell_d` stayed** —
  it is a "spell" vocabulary over an engine that deliberately calls the concept
  `ability`, and a compatibility shim belongs beside the thing it keeps
  compatible.

- **Every shipped area is OLC-managed.** Four files each: `rooms.lua`,
  `items.lua` and `mobs.lua` are OLC-owned and rewritten wholesale, and a
  hand-written `custom.lua` holds everything that is a function. Thornhollow
  lost its three-way `ROOM_D.merge` split, because `areaload.inspect` prefers
  `init.lua` over `rooms.lua` unconditionally and a generated `rooms.lua` beside
  a surviving `init.lua` would never have been read. `wizard_workshop/gear.lua`
  folded into `items.lua` — ten items OLC could not list, lint or save.

- **`olc adopt` was baking prototype output into the files it wrote.**
  `read_current` used `require`, which reads the module cache, and
  `prototype.resolve_list` flattens each record's prototype chain *in place* by
  design. So adopting an area copied the prototype's output in beside the
  `prototype` field that produced it, pinning the record: area data outranks a
  prototype, so later edits to the prototype would have done nothing and nothing
  would have said so. It reads from disk now. Two smaller ones with it: an
  `init.lua` area is refused with a reason instead of failing at "Could not read
  rooms.lua", and the authored `entrance` is carried through rather than
  defaulted to `<area>.entrance` — which would have made `verify` report every
  room in an area as an orphan.

- **A component's hand-written field survives being authored as data.**
  `drinkable` declares `on_drink` as `hand_written`, and `Item:new` copies a
  *fixed list* of hooks that does not include it — so `on_drink` reached an item
  only through the archetype path, where `drinkable.apply` assigns it to an
  already-built object. The moment a potion was authored as flat data plus a
  `custom.lua` patch, the hook was merged onto the data correctly and then
  silently dropped during construction. The potion was drinkable and did
  nothing. `components.build` now carries every component's `hand_written`
  names across, driven off the declaration rather than a second list.

- **An integral float in authored content is a cross-runtime hazard.**
  `speed = 1.0` serialises as `1.0` on Lua 5.5, which has an integer subtype and
  keeps the point so the value does not change type on the way back, and as `1`
  on LuaJIT, which has no such subtype. Same value, different file — so
  `olc save` produced a different diff depending on which Lua the server was
  built against. `demo_world` now asserts that regenerating every shipped area
  file from its own contents reproduces it byte for byte, on both runtimes.

### The combat systems are fed

Everything added in the previous phase was wired and inert. `game/` now feeds it:

- **Defence channels.** `combat_d` decides what a fighter can do by which traits
  they store, and nobody stored any — so every fight took the no-configuration
  path and parry and block could not occur to anyone. `defense`, `defense_dodge`,
  `defense_parry` and `defense_block` are derived traits now. The shape is a
  *pool and three weights*, not four ratings: the weights normalise into shares
  of the pool across the channels you can actually use, and the contest takes
  your best. So more channels is not more defence, and a naive table would have
  made picking up a shield make you easier to hit. The numbers are chosen so an
  ordinary level-1 defender is worth exactly `dexterity` — the value `rating()`
  fell back to — and the buckler's `stat_bonus` raises both the pool and the
  block weight so a shield is worth carrying.

- **Degrees of success.** `margin` was computed on every swing and discarded,
  because the mudlib ships one band at power 1.0. Four bands now — graze, hit,
  solid, decisive — with the top one rerolling the hit location.

- **Body layouts.** `Body.locate` ran on every swing and returned nil, because
  no creature named a layout and there was no layout to name.
  `game/body/creatures.lua` ships humanoid, beast, insectile and amorphous, and
  every shipped creature resolves to one — through `race` where the prototypes
  already set it, and through `body` where the shape genuinely differs.

- **Roundtime answers to the fighter.** `queue_d` was falling back to a flat
  three seconds and warning about it once per track. `round_length` is a derived
  trait, exactly 3.0 at dexterity 10 so nothing moved, and reachable by
  encumbrance and equipment through the ordinary `stat_bonus` path.

- **`emberlance` and `cleave` go through `resolve_attack`.** Both used `damage`,
  which is a number applied and cannot miss. `emberlance`'s own header claimed
  its damage met armour "exactly as they meet a sword", and that had stopped
  being true. `cleave` gained `roundtime = { rounds = 1.5 }`, the queue half it
  existed to demonstrate and did not.

- **`oak_buckler` is a shield.** It was tagged one and `armour.shield` was
  false, so the game's only shield could not be blocked with.

### Testing

- **Three test binaries** — `tests/driver/`, `tests/mudlib/`, `tests/demo_world/`
  — where there were about sixty files. The line between the first two is one
  question: if you deleted `mudlib/` and wrote your own, would you keep this test
  or rewrite it? The check that matters is that
  `mv game tests/demo_world ../away && cargo test --test driver --test mudlib`
  is green; it had thirteen failing binaries before.
- `tests/compute_wedge.rs` stays its own binary, and now says why in its header:
  every test in it spins a core for its whole deadline, so as a neighbour it
  starved the pool-recovery test past a forty-second deadline.
- `game/traits/broken_example.lua` is gone. Deliberately broken code in a content
  directory is bad form, and `make_test_lua()` put `game/` on `package.path`, so
  it sat one `require` away from every Lua unit test. The five broken traits are
  defined inline in `tests/mudlib/broken_traits.rs`.

### Output

- **`pager_d` terminates its short-text path.** Text under one page went out
  without a trailing newline while the paged path appended one, so `send_prompt`
  — which adds no leading newline — landed *on* the last line of output. It read
  worst in `olc`, which pages more than anything else and whose output is almost
  always shorter than a screen, so the path that did terminate was almost never
  the one taken.

### The development cockpit

- **`oxigeon-tui`**, a second binary in the same crate: telnet and the debug
  adapter in one window, so you can see what a breakpoint costs the game rather
  than inferring it from an editor. Play pane, source with a breakpoint gutter,
  call stack, variables tree, a REPL over `evaluate`, an Inspect tab that reads
  traits and effects through the daemons rather than the raw table, and a trace
  view. See [oxigeon-tui](./tui.md).
- The file pane is a collapsed tree, the source pane takes the vi motions
  (`:`, `/`, `//`, `n`/`N`, `:noh`, `g`/`G`) and is syntax-highlighted, and every
  step has a `Ctrl`+arrow alias because `F11` is full-screen in most terminals
  and never reaches the application.

### GMCP was never sent to anybody

- **The negotiated capabilities never reached the Session.** Telnet negotiation
  wrote `TelnetConnection.capabilities`; the mudlib reads
  `Session.capabilities`, through `get_session`. Two structs on two objects, and
  nothing joined them — so `Session.capabilities` sat at `Default::default()`
  for the life of every session that has ever connected. Every one of `gmcp_d`'s
  four senders guards on `sess.gmcp_supported`, so **no GMCP was ever pushed to
  any client**: the TUI's Room.Info, Char.Vitals and Effects panes could not
  populate. The `Core.Hello` a client does receive comes straight from the
  driver and never touches Lua, which is what made the link look healthy. The
  same gap left `window_width` nil, so output was wrapped to a default
  regardless of the terminal's real size, and `terminal_type` was never known.
- **Nothing pushed after login either.** `send_vitals`, `send_status` and
  `send_effects` had no callers at all; `send_room` had one, in `movement.lua`,
  so `goto`, `teleport` and a respawn all moved a player without telling their
  client. The only `send_all` ran from the `Core.Supports.Set` handler — which a
  client sends during telnet negotiation, *before* login, when there is no
  character to describe. `gmcp_d`'s own header claimed all four were "pushed on
  the events that change them".
- **`gmcp_d.refresh` pushes what changed**, once per dispatch, from
  `prompt_d.render` — the one place that already runs after every command and has
  already settled the regenerating gauges. Diffed against the last payload, so a
  command that changed nothing sends nothing. Diffing rather than emitting from
  each subsystem is a coverage decision: an event per change would need one in
  `take_damage`, `heal`, `award_xp`, the effect apply and expire paths and the
  regeneration settle, and would still miss regeneration between commands and an
  effect that expired on a tick.
- Plus `player.login` for the opening state and `room.entered` for every way a
  room can change.
- **The test harness discarded outbound GMCP** — `Ok(_) => continue` — which is
  why nothing ever caught this. It keeps it now (`RealVm::take_gmcp`), and
  `tests/gmcp_outbound.rs` asks what a client actually receives.

### OLC builds things now

- **A declarative schema is the single source of truth.** `mudlib/schema/{room,
  item,mob}.lua` list every authorable field with its type, default, editability
  and one line of help; codegen emits from it, `olc set` validates through it,
  `verify` checks against it and `objdump -s` annotates with it. Every OLC defect
  traced to its absence: `generate_room` hardcoded five fields, `olc set` could
  not exist because there was nothing to enumerate, and `adopt` could not report
  what it would lose because nothing knew what "lose" meant. Component fields
  live in the component file beside the `from_data` that reads them, discovered
  rather than listed.
- **`lib/serialize.lua`**, a correct Lua emitter. The old codegen concatenated
  strings, so a room title containing a quote produced a file that would not
  compile and nothing said so until the next reload; its multi-line branch
  indented the closing `]]`, which put four spaces *inside* the string and grew
  them on every read-and-rewrite for ever; and `%.17g` would have rendered every
  authored `speed = 1.2` as `1.1999999999999999`. Emission is idempotent, so a
  file that has not changed produces no diff.
- **A `dig` no longer destroys the room it touches.** The round trip re-emitted
  only the five fields it knew, so `light`, `smell`, `sound` and `tags` were
  deleted from every room a second exit was dug out of. A field *no schema names*
  now round-trips verbatim and is reported — dropping it silently is the bug
  class this work exists to end.
- **`olc` grew a grammar**: create an area, a room, an item or a creature; add a
  component; tag anything; set any field; see what is settable; save. One verb,
  no sub-shell — `look` and `who` still work while you build. The cursor is a
  default argument and deliberately does **not** follow movement; `olc set on
  <target>` writes elsewhere without moving it, and `on` is a reserved word
  rather than a guess about whether the next token resolves as a field.
- **Buffered drafts.** `set` changes the draft and the live object; only
  `olc save` touches disk, after `verify`. The old OLC wrote on every dig, which
  is what makes a lint pointless — you cannot gate a write on a check that runs
  after the write.
- **`editor_d`**, a line editor shaped like `pager_d`. Nothing in the repository
  could accept multi-line input, so a room description — six lines of prose — had
  no way in. Dot-commands, so `quit` typed into a description is text.
- **`verify` is a content linter.** It was "does this file parse", which is worth
  knowing and is not the question a builder has: a file can compile perfectly and
  still describe an exit into nothing. Reads **disk**, not the registry, because
  the registry has already collapsed duplicate ids and applied `custom.lua` and
  so cannot answer "what will the next reload do". Reports and never fixes.
- **`olc adopt`** brings a hand-authored area under OLC, in two steps, and never
  parses Lua source: the original is copied to `legacy_*.lua` and the generated
  `custom.lua` *references* it. `_meta.lua` is written last, so a failure part-way
  leaves the area unmanaged and OLC still refuses it. Nothing is ever deleted.
- **Areas are discovered.** `game/init.lua` named every one of them by hand, so
  an area OLC created was invisible until somebody edited that file — and OLC
  never registered a reset source, so `areas reset <new_area>` answered "No
  registered source" for every area it ever made. `lib/areaload.lua` loads in
  passes across all areas, which also removes an ordering hazard already in the
  tree (`thornhollow.smithy` has a `down` exit into `collapsed_mine.adit`).
- **One direction table.** `movement.OPPOSITES`, `cmds/directions.lua` and
  `dig.lua` each had their own; `dig`'s had no entry for `in` or `out`, so
  digging either made a one-way passage and reported success — while
  `olc.md` claimed all along that it used `movement.lua`'s.
- **`objdump` gained `-d`/`-r`/`-i`/`-s`.** Defaults print byte-identically, so a
  dump stays diffable. `-s` marks each field against the schema, and `!` means no
  schema names it — the only thing in the system that answers *what am I about to
  lose?* before the loss.
- `item_d` feeds the tag index, which it never did: `DAEMON.tag.find("item", …)`
  came back empty for every item in the game while `Item.tags` was widely
  authored and `Item:has_tag` worked.

### A shell for the file tree

- **`ls`, `cd`, `pwd`, `cat`** over a virtual root with two mount points,
  `/game` and `/mudlib`. Not `list_dir`'s merged view: merged,
  `game/cmds/verify.lua` shadowing `mudlib/cmds/admin/verify.lua` shows as one
  entry, so you edit the copy that is not loaded and nothing happens. The
  virtual path is also the *permission* path — `permissions.toml` keys on
  `/game/areas`, and that is what `ls` prints and `cd` accepts.
- A directory you may not read is **named and counted**, with the missing
  permission spelled out. Omitting it silently makes `ls` look broken; showing
  its contents would be the leak.
- `~` is the area you are building; `cd` state lives on `fs_d` rather than
  `olc_d`, so `olc done` does not throw your working directory away.
- **No `rm`, `mv`, `mkdir` or whole-file editor**, and a test asserts they stay
  absent. An in-game `rm` is how areas vanish, and an `edit` would invite hand
  edits to the very files OLC regenerates.
- **`Player:send_paged`**, which colours to the player's preference and does not
  word-wrap. `DAEMON.pager.page` writes through the raw `send` efun, so a paged
  body reached the client with its `{colour}` tags unrendered — `trace.lua`
  carried a comment telling callers not to use colour, which is the wrong end to
  fix it. `cat` uses its `literal` mode: a mudlib file is full of `{red}`, and
  rendering it would paint the listing in the colours of the code you were
  reading while stripping it would silently delete tags from the source.

### The file jail has two roots

- **`write_file` could not reach the tree the world loads from.** It was jailed
  to the mudlib alone, so every file OLC ever generated landed in
  `mudlib/areas/` — a directory that does not exist in this repository — while
  `dig` reported `File written: game/areas/…`. Nothing OLC created could load,
  because nothing was where the loader looks. `read_file`, `write_file`,
  `append_file`, `delete_file`, `file_exists` and `verify_file` now reach both
  roots, as `list_dir` always did.
- **A path may name its root**: `write_file("game:areas/crypt/rooms.lua", …)`.
  Unprefixed, a read searches game-then-mudlib the way `require` does, and a
  write stays in the mudlib — so every existing caller is unmoved. A write names
  a file that may not exist yet, so there is nothing to search and the root has
  to be *chosen*; `audit_d`'s `logs/audit_watch.json` would have silently
  relocated under any rule that guessed.
- **The file efuns return failure rather than raising it**, and always did — so
  `pcall(write_file, …)` gives `ok = true, err = false` and every guard written
  that way was dead. `codegen_d` reported success for refused writes for as long
  as it existed. There is a second return value now, naming the permission that
  would have allowed it: `permission denied: /game/areas/crypt/rooms.lua needs
  'dir.write.game.areas' to write`.
- **Every `[directories]` key names its root**, and one that does not is dropped
  and logged rather than guessed at. `/areas` had no answer once there were two
  trees to mean, and applying it to the wrong one would be the "rule that was a
  no-op" this config file already documents once.
- **Directory rules match whole path segments.** `/game/areas` no longer covers
  `/game/areas_backup` by string prefix.
- `verify_file` had a second jail of its own — mudlib-only, and refusing any path
  containing `..` — so it disagreed with `read_file` about which files existed.
  One jail now.
- New `file_root(path)` and `dir_permission(path, op)`: which layer a read lands
  in, and what a directory rule demands there. Shadowing between the layers was
  previously invisible, and a shell had no way to ask what it may show.

### The builder role was a decoration

- **Every permission string now has one shape, and two tests hold it.**
  `game/setup_roles.lua` granted `cmd.olc`, `cmd.verify` and `efun.write_file`;
  the code required `olc`, `efun.verify` and `efun.file.write`. Not one of the
  builder role's eight grants matched anything it was meant to unlock — the role
  existed, the database held it, `role list` printed it, and it did nothing. The
  only account that could build was account 1, through the `is_admin` superuser
  bypass, which is exactly why nobody noticed. Commands are `cmd.<own verb>`,
  efuns are `efun.<name>` spelled as the global is, directories are
  `dir.<op>.<top>`, and anything else is `<thing>.<capability>`.
- **Raising an alert and hearing one are different powers.** They were one
  string, so the only way to be told about an incident was to be able to page
  everyone about it. `cmd.alert` sends; `alert.receive` receives.
- `cmd.audit.manage` replaces `daemon.audit_d.manage`, and the `journal`/`audit`
  efuns are gated as `efun.journal_read` / `efun.audit_read` — the command gate
  stops the verb, the efun gate stops mudlib code reaching past it.

### Layout

- Components live in `mudlib/components/`; commands are grouped into `admin/` and
  `building/` with the core verbs left at the top level, and the eight direction
  commands are one `directions.lua`.
- **Tests that assert shipped content moved to `tests/demo_world/`**, which is
  deleted along with `game/`. Everything else uses
  `RealVm::boot_with_fixture_world` or `boot_fixture_with_probe` and does not
  name Thornhollow, so somebody who deletes `game/` to build their own world does
  not inherit a broken suite. See [Testing](./testing.md).
- Inline lfuns in area files were hoisted to named functions above the data
  tables. A room's data should read as data.

### Gameplay

- **An authored `max_hp` is honoured.** `max_hp` is derived from constitution and
  level, so a mob declaring 24 hit points got 90 — the declaration was computed
  over rather than used. A hidden `max_hp_flat` attribute now feeds the formula,
  which keeps `max_hp` derived (effects modify it, nothing stores it) while
  letting content say what it means.
- **`_killed_by` is set by the blow that kills, not by every hit**, so a rat you
  are still fighting is no longer recorded as having been killed by you. It also
  stores the killer's *identity* rather than the entity: two fighters pointing at
  each other was a reference cycle that kept a whole `Player` alive past the
  mob's despawn.

### Debugging: freeze by choice

- **`[servers.debug] stop_the_world`**, default `true`. A breakpoint freezes the
  whole game, which is what every debugger does and all LuaJIT can do. Set it
  `false` on a `lua55` build — or run `trace freeze off` — and a stop suspends
  only the dispatch that hit it. The mechanism landed first; the *choice* did
  not, so a `lua55` server had no way to get the ordinary behaviour back.
- **A suspended dispatch is no longer taken apart by the collector.** A stop that
  held one dispatch and let the game carry on came back to a frame whose
  parameters had gone nil — `mobile.lua:185: attempt to index a nil value (local
  'self')`, on the line after one that had used `self` quite happily, and only
  ever while debugging. `luaD_hook` raises `L->top` over the whole activation
  register for the duration of a hook and puts the low value back before
  returning, and a hook that *yields* is no exception — so a dispatch parked at a
  breakpoint sits there with `top` below its own live registers, and the first
  atomic phase to run nils everything above it as a dead stack slice. The
  collector is now held off for as long as anything is parked. Nothing outside
  the VM can raise a suspended thread's `top`, so that is the fix rather than the
  workaround. Cost: a stop stops collecting, bounded by `auto_continue_secs` and
  only while a client is attached.
- **Several dispatches can be stopped at once, and are separately inspectable.**
  `DebugState::parked` was one slot for an unbounded number of stops: a
  breakpoint on a line a ticker reaches every round parked a new dispatch every
  round, each silently replacing the last. The older stop's captured frames were
  dropped without being released — a permanent leak in `introspect.lua` — and
  every question about it came back empty, which is what "the debugger kept
  resetting itself" was. Each stop is now its own DAP thread, named for what it
  is (`sheridan: hit`, `timer:combat.round`).
- **`stopped` now means "the world is frozen"** and nothing else. It used to mean
  "something is stopped", which stopped being the same thing the moment a stop
  could hold one dispatch — so the cockpit drew its freeze banner over a game
  another player was still playing, and `allThreadsStopped` was hard-coded true.
- The evaluator names the `.`-versus-`:` slip. `player.is_alive()` passes no
  `self` and fails inside the callee, at a file and line unrelated to what was
  typed; the error now carries "did you mean :is_alive()?" when the expression
  plainly contains a `.` call.
- **The debug evaluator could not see locals on Lua 5.5.** The sandbox replaces
  `load` to refuse bytecode, and its wrapper silently dropped `load`'s fourth
  argument — the chunk environment. On 5.2+ that is the *only* way to set one
  (5.1's `setfenv`, which LuaJIT still uses, is gone), so watch expressions,
  breakpoint conditions, the REPL and logpoints all compiled against the globals
  instead of the paused frame: `player` read as nil on a line where `player` is
  plainly in scope. Invisible from the sandbox's own tests, which only ever asked
  whether a chunk compiled.
- **Lua syntax highlighting** in the source pane, with search hits painted over
  it. Long brackets are resolved by scanning the file once when it opens, so a
  keyword inside a `[[ ]]` block is not coloured as one.
- **The variables pane takes the middle column when focused**, swapping with the
  source. Reading values is most of what a debugger is for, and a 38-column
  strip was not enough to do it in.
- **A stop no longer gives a file two identities.** The adapter reports frames
  by absolute path and the tree works in relative ones, so a breakpoint set
  before a stop and one set after it landed on different keys: the gutter dot
  vanished on the line you were standing on, and the tree kept a mark for a
  breakpoint you had removed.
- **Logpoint output is no longer drawn as a warning.** `output` events now carry
  a category, so a logpoint doing exactly what it was told reads as a report
  rather than a problem — and the pane shows a useful number of them instead of
  the last two.
- **The cockpit's file pane is a collapsed tree** rather than several hundred
  full paths, with `h`/`l` to close and open and `●` on a folder holding a
  breakpoint. Opening a file expands everything above it, so a stop always shows
  where it landed.
- **The source pane takes the vi motions**: `:` for a line number, `/` to search
  with `n`/`N` to walk the matches, `g`/`G` for the ends. Matches are highlighted
  where they are, not merely jumped to.
- **The cockpit sets logpoints** with `⇧F9`/`^L`, and every step gained a
  `Ctrl`+arrow alias — `^→` over, `^↓` into, `^↑` out, `^G` go. The function keys
  are not ours to take: `F11` toggles full-screen in most terminals and never
  reached the application, so "step into" was simply unavailable to anyone whose
  terminal does that.
- **Logpoints.** A breakpoint with a `logMessage` reports `{expr}` substitutions
  and keeps running, gated by the same condition and hit count. Standard DAP, so
  VS Code sets one natively. This is the answer to "a breakpoint on a combat
  round is a stop per round".

### The game thread runs Lua 5.5 by default

- **A breakpoint no longer freezes the server.** mlua's `VmState::Yield` is Lua 5.3+ only, so under LuaJIT a stop had nowhere to suspend to and was implemented by blocking the one Lua thread — every player froze until the client continued. On 5.5 the hook yields, the engine parks that one command as a coroutine, and everyone else keeps playing. `tests/yield_pause.rs` asserts the difference on both runtimes.
- **The default changed for that reason, not for speed.** On real command dispatch the two runtimes are within a few percent of each other, because `limits.lua_instruction_limit` already called `jit.off()` at boot and the game thread had been interpreted all along. `cargo build --no-default-features --features luajit` still builds the old one. See [Performance](./lua-api/performance.md).
- **Ticks are coroutines too.** They were a direct call at first, on the reasoning that stopping in one would be rare. It is the opposite — combat rounds, regeneration and effect ticks all arrive as `on_timer`, so a tick is the likeliest place to want a breakpoint, and breaking in combat froze every player.
- **A stop that Lua cannot suspend blocks instead.** Connect handlers, GMCP and hot reloads are not coroutines, and anything called *by C* — a `table.sort` comparator, a `gsub` replacement, an `__index` metamethod — cannot yield past that frame. mlua silently *ignores* a yield it cannot honour, which left the debugger believing the VM was stopped for the rest of the process. The hook now asks `lua_isyieldable` and blocks when the answer is no.
- **The parked path gained the auto-continue valve** the blocking path always had, so a crashed editor cannot leave one player at a dead prompt for ever.
- Module-level guards in `trait_d` and `effect_d` are keyed per entity and per scope rather than per process, so one suspended dispatch cannot suppress another entity's regeneration or effect pipeline. `tests/interleaving.rs`.

### Compute runs out of process

- **Workers are `oxigeon-compute` child processes**, built separately with `cargo build --release -p oxigeon-compute`. They link LuaJIT whatever the server was built with, so the compiler stays where it was worth 2.10× — the arithmetic-heavy jobs compute exists for — while the game thread gets the debugger it wanted.
- **A runaway job can now be killed.** Rust cannot kill a thread, so with the compiler on a job that ignored its deadline burned one worker for the life of the *server* and all the pool could do was count it. The watchdog now terminates the worker process and the next job gets a fresh one. `tests/compute_wedge.rs` asserts the recovery, which could not have passed before.
- A worker exits on its own when its stdin closes, so a killed server does not leave orphans spinning.
- Workers start on first use: `server_info().compute.spawned` is 0 on an enabled-but-idle pool.
- **A whole number from a worker is a float.** LuaJIT has one number type, so a job returning `3` returns `3.0`. The wire carries what the worker produced and converts nothing, because promoting integral floats would corrupt a job that meant to return one.
- The value marshaller, the sandbox and the worker VM moved to a shared `oxigeon-lua` crate, compiled once per runtime. There is still exactly one list of what the sandbox removes.

### Also

- `package.searchers` is now cleared as well as `package.loaders`. The old code read only the 5.1 name behind an `if let Ok`, so on any 5.2+ runtime it found nothing, did nothing, and left the C module loader installed — a sandbox that failed open with no error and no failing test.
- Five `set_hook` calls in the debug spike tests were ignoring their `Result` since the mlua 0.11 upgrade, which is precisely the failure `tests/common/mod.rs` exists to prevent.

## Phase 3: Hardening, Performance & Persistence

### Security

- **The sandbox is now applied to the running VM.** `apply_sandbox` was well tested and never called: `io.open`, `io.popen`, `os.execute`, `os.exit` and `package.loadlib` were all reachable from mudlib code, and `io.open` read files outside the mudlib jail. The dead `create_sandboxed_env` was deleted so there is one boundary, not two.
- `jit`, `package.cpath` and the native module loaders removed; `load`/`loadstring` now refuse binary bytecode.
- **Argon2 moved off the game thread.** Every login froze the whole game for ~370 ms, pre-authentication, so spamming attempts was a trivial denial of service. `authenticate` and `create_account` are now asynchronous and answer at `on_auth_result`. Added a bounded queue and a per-address lockout after 5 failed attempts.
- **Timer-dispatched code has an explicit identity.** Gated efuns called from a daemon tick used to fail closed, silently. Engine-internal dispatch now declares itself; `tests/timer_identity.rs` pins that a player session is still refused.

### Correctness

- **`lua_to_json` had five silent failures on the `save_character_data` path**: a table that is both a list and a map lost every string key, cycles exhausted the Rust stack and killed the process, NaN and infinity became `0`, functions became `null`, and unusual keys vanished. All now raise, naming the offending field.
- **Output is no longer silently dropped.** Ten `try_send` sites against a 64-slot channel lost text with no log, counter or marker. Drops are counted, logged, surfaced in `server_info()`, and the player sees a truncation notice.
- **Lock poisoning is survivable.** 42 `.unwrap()` calls on lock acquisition would have turned one panic into a permanently dead game. `read_recover`/`write_recover`/`lock_recover` recover and report.
- **The stat whitelist in `Mobile:new` is gone.** It rebuilt `obj.stats` from a fixed list of nine keys on every load, so any other stat was silently dropped even though `to_save` had faithfully written it — a trait named `wisdom` would have vanished on every login.
- **`Char.Status` reported 0 experience and 0 gold for every character, always**, because `gmcp_d` read `player.stats.xp` and `player.stats.gold`, which have never existed. `death_d`'s XP-loss-on-death was dead code for the same reason.
- **`ticker_d.remove_by_prefix` now exists.** `character_d.unload` had called it since it was written; the function was never there, so the call raised into a pcall that logged at debug level and every per-player timer leaked.
- **`send_lines` accepts a table again.** Both spellings were in use across the mudlib and the table form printed `table: 0x...` to the player — which is what `death_d` had been announcing deaths with.
- **A clean shutdown now saves.** `LuaCommand::Shutdown` broke the engine loop without dispatching anything to Lua, and the Ctrl+C path never joined the Lua thread — `Drop for ScriptEngine` only sent the command and returned. Two failures compounding: nothing asked the mudlib to save, and nothing would have waited if it had. Since `CHARACTER_D` is a write-back cache flushed by the autosave ticker, **every clean restart discarded up to `autosave_seconds` (default 300) of every online player's progress.** The engine now dispatches [`on_shutdown`](./lua-api/events.md) under its own identity before breaking, and the driver waits for the thread — bounded by `game.shutdown_timeout_seconds` (default 30) so a mudlib that wedges in the hook cannot hang the process. `tests/clean_shutdown.rs` pins all of it, ending with a save through the real mudlib.
- Fixed the file jail refusing every legitimate read on a relative mudlib root — `audit_d` could not load its watch list and said nothing.
- SQLite now runs in WAL mode with `synchronous = NORMAL`, removing an fsync from the Lua thread on every write.

### Performance

- **`lua_instruction_limit` is enforced, and on by default.** It was parsed and never read. Enforcing it disables the LuaJIT compiler — LuaJIT dispatches no debug hooks from inside a compiled trace — but measured through the real mudlib that costs 2-7% on commands, because the compiler is worth ~1.00x on command dispatch and 2.10x only on tight arithmetic. See [Performance & the JIT Trade-off](./lua-api/performance.md).
- `lua_memory_mb` is enforced too; it turned out to work on this build after all.
- `cargo bench` (criterion) measures the real mudlib and refuses to run if its own control shows the JIT toggle is broken.

### Features

- **[Traits](./lua-api/traits.md)** — character attributes that are *computed* rather than stored: derived from other traits (Willpower from Wisdom), filtered through active effects, and regenerating from a timestamp rather than a timer. Deliberately no `mod` field on a trait: Evennia's Traits contrib stores one, which makes a buff a write to the thing it buffs, so any path that misses the matching unapply leaves the character permanently wrong. Here nothing is stored, so there is nothing to unapply. Dependencies are declared and *enforced* — reading an undeclared one raises — which is what lets `seal()` report a cycle as a path rather than a shrug.
- **[Effects](./lua-api/effects.md)** — buffs and debuffs as an event pipeline. `run(entity, "damage_taken", ev)` passes the numbers through every effect that cares. Ordering is by declared **phase**, not registration order, so "-15% damage" and "-5 flat damage" on a 30-point hit give 20 rather than depending on which buff landed first. Passive stat modifiers are the same pipeline under the hook family `trait:<id>`, so a +2 ring and a -15% buff are authored identically. Definitions hold functions and live in code; instances are nine plain fields and live in the cache.
- **[State Cache](./lua-api/state-cache.md)** — the write-behind tier `task_list.md` item 3 asked for, plus `DAEMON.cooldown`. Three tiers chosen by how much you would mind losing the data. Measured: 10 changes to one player cost 1.20 ms written through and 0.15 ms written behind; 1000 changes, 1077 ms against 2.3 ms. A flush is one `db_put` of the whole scope rather than a merge patch, because RFC 7396 expresses deletion as a JSON null and a Lua table cannot hold one — a merge flush could never remove an expired effect. Values are checked against `lua_to_json`'s rules when written rather than when flushed, so a bad value is refused at the call site instead of raising inside `on_shutdown`.
- **[Creatures & Combat](./lua-api/combat.md)** — `mob_d` and `combat_d`: templates, instances, room occupancy, respawn, and a minimal round-based fight so the pipeline is visible in numbers a player sees. Combat state is memory-tier and never written.
- **[Compute Bridge](./lua-api/compute.md)** — `compute()` runs a long computation on a worker thread with its own LuaJIT VM and answers at `on_compute_result`. Worker VMs have no efuns at all.
- **[Document Store](./lua-api/document-store.md)** — twelve `db_*` efuns over a generic JSON table. Persisting a new type needs no Rust, no migration and no rebuild, which matters because `embed_migrations!` is compile-time and the game layer can never ship schema.

### Fixed

- `mudstatus` printed "0s" uptime: it read `info.uptime_seconds`, but the field is `uptime_secs` — and `types/oxigeon.lua` declared the wrong name, which is why. The stub also declared two fields that never existed.
- `save_character_data`/`load_character_data` were annotated as taking and returning strings; they take and return tables.
- The "Persistent Store" annotation claimed persistence across restarts. It is a Lua table that survives hot reload only.


## Phase 2: Game World

### Features

#### Object System
- Base `Object` class (`game/lib/object.lua`) — shared fields (`id`, `short`, `description`), `resolve()` (lfun pattern), state access methods
- `Room` inherits from `Object` via metatable chain — exits, contents, actions, items, appearance rendering
- Callable properties (lfun pattern) — any property can be a string or a function returning a string
- `resolve()` returns `<invalid lfun return>` for non-string function returns

#### World Engine
- Two-layer Lua architecture: `mudlib/` (system) + `game/` (content)
- DAEMON service registry (DAEMON global table)
- Data-oriented room definitions — area files are pure data tables with logic separated
- `ROOM_D.from_data()` — creates Room objects from data tables with field mapping and validation
- `ROOM_D.load_area()` — processes area data arrays, extracts `_meta`, registers with world_d
- `ROOM_D.merge()` — combines multiple data arrays for multi-file areas
- Builder pattern (ROOM_D) preserved for dynamic/programmatic room generation
- Area metadata (`_meta`) — stored in area files, queryable via `DAEMON.world.get_area_meta()`
- Multi-file areas — large areas split across sub-files, assembled via `ROOM_D.merge()`
- Virtual room providers — register generators by prefix for infinite/procedural spaces
- Virtual room caching and eviction (`evict_virtual`)
- World daemon (world_d) — room registry with virtual fallback, character locations, movement
- Room actions (add_action) — room-scoped custom commands
- Layered command dispatch: room actions → system commands
- Movement library with room-scoped messaging
- CHARACTER_D — in-memory character state cache with DB persistence

#### Object State
- In-memory key/value state store scoped by object ID (rooms, items, mobs)
- Driver-side efuns: `set_object_state()`, `get_object_state()`, `get_all_object_state()`, `clear_object_state()`
- Survives hot-reloads (Lua VM globals), cleared on restart
- `Object:get_state(key)` / `Object:set_state(key, value)` convenience methods

#### Timer System (TICKER_D)
- Tokio-backed async timers — zero polling, each timer sleeps independently
- `schedule_timer(id, delay)` efun — one-shot timer via `tokio::spawn`
- `schedule_repeating(id, interval)` efun — repeating timer via `tokio::time::interval`
- `cancel_timer(id)` efun — immediate cancellation via `AbortHandle`
- `LuaCommand::TimerFired` — engine dispatches `on_timer(id)` to Lua
- `DAEMON.ticker.after(delay, id, fn)` — one-shot with Lua callback
- `DAEMON.ticker.every(interval, id, fn)` — repeating with Lua callback
- `DAEMON.ticker.remove(id)` — cancel timer and callback
- Input validation, pcall-wrapped callbacks, journal_d error logging

#### Event System (EVENT_D)
- Godot-style signals — named event channels with subscribe/emit
- `DAEMON.event.on(event, id, fn, priority?)` — subscribe with optional priority
- `DAEMON.event.off(event, id)` / `off_all(event)` / `off_by_prefix(prefix)` — flexible unsubscribe
- `DAEMON.event.emit(event, data)` — synchronous dispatch in priority order
- `DAEMON.event.defer(event, data, delay)` — deferred emit via TICKER_D
- pcall-wrapped handlers, sorted listener cache, full introspection API

#### Efuns (new in Phase 2)
- `save_character_data()`, `load_character_data()` — character JSON persistence
- `set_object_state()`, `get_object_state()`, `get_all_object_state()`, `clear_object_state()`
- `schedule_timer()`, `schedule_repeating()`, `cancel_timer()`
- New config keys: `game.command_paths`, `game.start_room`, `game.game_path`

#### Observability
- Structured error logging via journal_d for all critical operations
- `pcall`-wrapped cleanup chains (disconnect, init loading)
- Input validation in all daemons with logged warnings

#### Tests
- 147 tests total — all passing

---

## v0.1.0 (Current)

**Initial release of Oxigeon**

### Features

#### Network
- Telnet server (RFC 854) with full IAC state machine
- RFC 1143 Q Method option negotiation (prevents infinite loops)
- Initial negotiation offers: SGA, GMCP, MCCP2, TTYPE, NAWS
- ECHO option (password masking) — `start_echo()`/`stop_echo()`; sends `IAC WILL ECHO` / `IAC WONT ECHO`
- GMCP support (option 201) — `send_gmcp()` efun, `on_gmcp()` event hook, `Core.Hello` on connect
- CR/LF normalization on send and receive
- Full bidirectional relay loop: TCP read → Telnet parse → Lua on_input; Lua send → session channel → TCP write

#### Sessions
- UUID-based session identifiers
- Full session lifecycle: `Connected → Authenticating → Authenticated → Playing`
- `authenticate_session(session_id, account_id)` — marks session authenticated, enforces multisession policy
- `enter_game_session(session_id, account_id, character_id)` — marks session as playing with character
- Multisession policy: `single`, `shared_character`, `multi_character`, `full_multi`
- Max connections limit; `"Server is full"` message on reject

#### Database
- SQLite via Diesel 2.x + r2d2 connection pool
- Automatic migrations on startup (embedded via `embed_migrations!`)
- Account model with Argon2id password hashing
- Character model with per-account limit enforcement and globally unique name constraint

#### Scripting
- LuaJIT (5.1 API) on a dedicated OS thread
- Full sandbox: `io` and `debug` removed, `os` reduced to its clock functions, binary bytecode loading blocked
- `require()` jailed to mudlib directory — sets `package.path` before loading `init.lua`
  (Windows UNC `\\?\` extended path prefix stripped to avoid Lua `?` substitution conflicts)
- Path traversal prevention (`../` jailing) for all file efuns
- Hot-reload: `reload(module_name)` from Lua, `on_load`/`on_unload` hooks
- `LuaCommand` channel pre-created so `cmd_tx` is available in `EfunContext` for Lua-triggered reloads
- Persistent storage across reloads: `set_persistent()`/`get_persistent()`

#### Architecture
- Three-layer design: **Core** (driver internals) → **Domain** (creator-facing models) → **Mudlib** (Lua game)
- `src/domain/` (previously `src/middle/`) — renamed to reflect DDD terminology
- `AccountStore` and `CharacterStore` traits defined in `src/domain/traits.rs`
- `DieselAccountStore` and `DieselCharacterStore` implement the respective traits

#### Efuns (complete list)
**Output:** `send()`, `send_prompt()`, `broadcast()`, `disconnect()`

**Telnet:** `send_gmcp()`, `start_echo()`, `stop_echo()`

**Session:** `this_session()`, `get_session()`, `all_sessions()`, `set_session_state()`,
`authenticate_session()`, `enter_game_session()`

**Account:** `authenticate()`, `create_account()` (both asynchronous — they answer at `on_auth_result`), `get_account()`

**Character:** `create_character()`, `get_characters()`, `get_character()`

**Utility:** `log()`, `time()`, `config()`

**File I/O (mudlib-jailed):** `read_file()`, `write_file()`, `append_file()`, `file_exists()`,
`list_dir()`, `delete_file()`, `os_time()`, `os_clock()`, `os_date()`

**Hot-reload:** `reload()`, `set_persistent()`, `get_persistent()`

#### Mudlib (Starter)
- `mudlib/init.lua` — event handlers (`on_connect`, `on_input`, `on_disconnect`, `on_gmcp`,
  `on_load`, `on_unload`), command dispatcher (`help`, `who`, `time`, `say`, `quit`)
- `mudlib/login.lua` — full login/registration flow with ECHO masking; calls
  `authenticate_session()` + `enter_game_session()` for proper session state transitions
- `mudlib/lib/strings.lua` — string utilities
- `mudlib/lib/tables.lua` — table utilities

#### Documentation
- mdbook-based documentation served at `docs/` (`mdbook serve docs/ --port 3000`)
- Lua API reference: efuns, events, sandboxing, file access
- Architecture overview with layer diagram and concurrency model
- Configuration reference (driver.toml and server.toml)
- Protocol documentation: Telnet, GMCP, MCCP, ECHO
- Rust API reference: domain models, swappable traits, extension guide

### Tests
- **74 tests total — all passing**
- 43 unit tests:
  - Telnet parser FSM (13 tests)
  - Option negotiation Q Method (8 tests)
  - Codec encode/decode (8 tests)
  - Session handler + multisession policy (6 tests)
  - Lua sandbox: io/os/debug blocked, path traversal prevented, bytecode rejected (8 tests)
- 31 integration tests:
  - Account store CRUD + Argon2 authentication (9 tests)
  - Character store CRUD + per-account limits (6 tests)
  - Hot-reload: module update, error resilience, multiple cycles, event dispatch (4 tests)
  - Sandbox: io module, os.execute, loadfile, dofile, debug module, pcall, coroutine, table (12 tests)

### Known Limitations / Upcoming
- MCCP2 zlib compression negotiated but not yet applied to the write stream
- PostgreSQL backend declared in config but not fully wired (requires libpq)
- WebSocket and TLS listeners not yet implemented
- `set_persistent()`/`get_persistent()` live in VM memory only — not persisted across server restarts
- Object state (`set_object_state`) lives in VM memory only — not persisted across server restarts
