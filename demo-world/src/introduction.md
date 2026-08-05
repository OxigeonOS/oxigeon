# Thornhollow — A Guided Tour

A frontier town at the mouth of a collapsed mine, on the edge of a drowned
marsh. Five areas, about forty rooms, and a walk of maybe an hour if you read
everything.

It is also an **instrument**. Every room in it exists because some part of the
driver needed something to act on, and the honest way to find out whether a
feature works is to make a player use it. This guide walks the world twice at
once: once as somebody playing it, and once as somebody asking *what is this
proving*.

## How to read this

Each area chapter has the same shape:

> **The walk** — what to type, in order, and what comes back.
>
> **What it proves** — the driver features that room exercises, and why they
> needed content rather than a unit test.
>
> **Look closer** — the interesting bit. Usually something that was subtly
> wrong until this content made it visible.

Boxes like this one are for the *why*:

> [!NOTE]
> **The rule the whole world is built on.** `mudlib/` is anything a second game
> would want unchanged. `game/` is this game — the rooms, but also the
> *decisions*: whether an aggressive creature attacks, whether it rains, what a
> quest is. The test is never size or subject. It is: would another game want
> this file as it stands, or would it want a different one?

## What is here

| Area | Rooms | Exists to prove |
|---|---:|---|
| [The Wizard's Workshop](./workshop.md) | 6 | room actions, object state, an lfun description, the original fixture |
| [Thornhollow](./thornhollow.md) | 10 | a multi-file area, shops, dialogue, factions, containers, the notice board |
| [Greywater Marsh](./marsh.md) | 5 | weather-driven descriptions, aggression, damage types, a daily gate |
| [The Collapsed Mine](./mine.md) | 6 | darkness, a locked door, a puzzle, a boss, an area reset |
| [The Drowned Reach](./reach.md) | ∞ | virtual rooms, eviction, off-thread pathfinding |

Plus five quests, four spells, three shops, twenty-one creatures and a notice
board nobody has posted to yet.

## The shortest possible tour

If you have ten minutes and want to see the most:

```
east / up / up                 out of the workshop into town
north                          the smithy — talk smith, list, quest
south / east                   the market — buy a lantern at Hobb's
west / west / west             the causeway — watch the weather in the prose
east / east / north / down     the mine — dark, and you need that lantern
```

Everything else in this book is that walk, slower, with the reasons.

## A note on honesty

Several things in this guide are described as having been *broken* until the
content found them — the sealed workshop, the missing `up` command, a marsh
that could not be entered, experience that never became a level. Those are not
war stories. They are the argument for building a demo world at all: each one
survived a green test suite, because a suite that moves with `goto` never walks
through a door.
