# Before You Start

## Getting in

```bash
cargo run
```

```
telnet localhost 4000
```

The first thing you are asked is a username. Type `new` to make an account, then
a name, then a password of at least six characters. A character is created
automatically with the same name, and you arrive in the Wizard's Workshop.

> [!NOTE]
> **The first account created is the superuser.** It bypasses every permission
> check, which means the admin commands in this guide will work for you and not
> for the second person who connects. That is deliberate — see
> [Admin Tour](./admin-tour.md).

## The commands you will actually use

```
look  l                    look at the room
north south east west      and up down in out
n s e w ne nw se sw

inventory  i               what you are carrying
get  drop  put  give       moving things about
wear  wield  remove  eq    equipment
examine  x                 anything, closely
use                        whatever the thing does

score  sc                  your attributes
skills                     what you have learned
effects                    what is on you
quests                     what you are doing

talk <someone>             their greeting
ask <someone> about <x>    one subject
say  '  emote  :           other people

help                       everything, generated from the registry
```

`help <command>` gives detail on any one of them. The list is built from the
command modules themselves, so it cannot go stale.

## Two things worth knowing early

**A room may claim a verb.** Standing at the well in Thornhollow Square, `drink`
is the well rather than a potion. Room actions are checked before system
commands, deliberately — you are somewhere, and where you are should win. The
room tells you when it has claimed something:

```
You could try: read the notices, drink from the well
```

**Colour can be turned off.**

```
> color off
```

Everything the game sends is written as markup (`{red}...{/}`) rather than raw
escape codes, precisely so it can be stripped. If you are using a screen reader,
do this first; nothing in this guide depends on colour.

## If you get stuck

`help` lists every command you have permission for. `help all` lists the ones
you do not, so you can see what a builder or a staff member would have.

Nothing in the world can permanently ruin your character. Dying costs you
nothing but the walk back — you reappear in the workshop with a quarter of your
health, and your effects are cleared except for the two that say otherwise.
