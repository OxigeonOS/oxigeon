# Interface — Prompt, Colour, Pager, Channels, Snoop

The five daemons a player meets without ever thinking about them. Each was a
single row in [Daemons](./daemons.md) until now.

## `prompt_d` — the line before the cursor

```
> prompt %h/%H %m/%M >
[95/100 42/50 >]
```

A prompt is a template rendered after **every** command, so it has to be cheap.
Values are read through `player:trait(id)` rather than `player.stats[id]` —
`max_hp` is derived and is not stored at all, and a buffed attribute stored is
the wrong number. TRAIT_D's memo is what makes that affordable: a repeat read is
two integer comparisons.

| Token | |
|---|---|
| `%h` / `%H` | current / maximum health |
| `%m` / `%M` | current / maximum mana |
| `%g` | gold |
| `%x` | experience |
| `%l` | level |
| `%r` | the room you are in |
| `%n` | your name |
| `%%` | a literal `%` |

```lua
DAEMON.prompt.set(char_id, "%h/%H %m/%M %r> ")
DAEMON.prompt.render(session_id)   -- what commands.lua calls
```

A player with no prompt set gets the default. An unknown token renders as
itself rather than as an error, because a prompt that refuses to render leaves
somebody unable to see what they are typing.

## `lib/color.lua` — markup, not escape codes

```lua
player:send("{red}You are bleeding.{/}")
player:send("{fg:214}A warning{/} in 256-colour.")
```

| Tag | |
|---|---|
| `{red}` `{green}` `{yellow}` `{blue}` `{magenta}` `{cyan}` `{white}` | the eight |
| `{bright_red}` … | the bright eight |
| `{fg:N}` / `{bg:N}` | 256-colour, 0–255 |
| `{bold}` `{dim}` `{underline}` | attributes |
| `{/}` | reset |

Markup rather than raw escapes for one reason: **it can be removed**. `color
off` strips the tags on the way out, which is what a screen reader needs, and a
mudlib full of `\27[31m` cannot offer that at all. The stripping happens in
`Player:_process_output`, so it covers every path text can take to a player.

```
> color off
Colour is now off.
```

The setting is a SAVE_FIELD (`color_enabled`), because "I cannot read this" is
not something anybody should have to say twice.

## `pager_d` — long output, a screenful at a time

```
> help
... 40 lines ...
[More] (return, q to quit)
```

The pager **intercepts input**: while a player is paging, `commands.dispatch`
hands the line to `pager_d` instead of parsing it as a command. That is checked
first, before room actions and before the channel shortcut, so pressing return
at a `[More]` prompt cannot accidentally be a verb.

```lua
DAEMON.pager.send(session_id, lines)        -- page it if it is long enough
DAEMON.pager.is_paging(session_id)
DAEMON.pager.handle_input(session_id, text)
```

`pagesize` sets the height; `pagesize 0` turns paging off for people whose
client scrolls perfectly well on its own. It is saved as `page_length`.

## `channel_d` — chat

```lua
DAEMON.channel.create("chat", { title = "Chat", colour = "cyan" })
DAEMON.channel.create("staff", { title = "Staff", colour = "red",
                                 permission = "channel.staff" })
```

| | |
|---|---|
| `create(name, config)` / `destroy(name)` | |
| `join(name, char_id)` / `leave(name, char_id)` | |
| `send(name, char_id, message)` | |
| `list()` / `get_subscribers(name)` / `is_subscribed(name, char_id)` | |
| `restore_channels(char_id, list)` | on login, from `SAVE_FIELDS` |
| `leave_all(char_id)` | on disconnect — in-memory only; the saved list survives |

### The channel-name shortcut

A subscribed channel's name works as a verb:

```
> chat hello
[Chat] Alice: hello
```

Handled in `commands.dispatch` **after** room actions and **before** system
commands. That ordering matters: a room with a `search` action and a channel
called `search` should give you the room's, because the room is where you are.

A `permission` on a channel gates both joining and sending, which is what makes
a `staff` channel a boundary rather than a convention.

## `snoop_d` — watching a session

```
> snoop alice
You are now snooping Alice.
```

Everything sent to the snooped session is mirrored to the snooper, prefixed so
it cannot be mistaken for their own output.

Two refusals, both of them structural rather than polite:

- **You cannot snoop yourself.** The mirror would feed itself.
- **You cannot make a chain.** If A snoops B, B cannot snoop A — nor can B snoop
  C while C snoops A. `snoop_d` walks the chain before allowing one, because a
  cycle is an infinite loop on the game thread rather than a confusing display.

Snooping is audited, because the question it answers is "who did this".

```lua
DAEMON.snoop.start(snooper_session, target_session)
DAEMON.snoop.stop(snooper_session)
DAEMON.snoop.snoopers_of(session_id)
```

Cleaned up on disconnect from either end.

## NAWS — the client's real width

`get_session().window_width` is negotiated over telnet and read by
`Player:get_width()`, which every `send` and `send_lines` wraps to. A client
that never sent one gets `DEFAULT_WRAP_WIDTH`.

`get_width` is **protected**: `get_session` raises on a malformed id rather than
returning nil, and this sits under every line of output the game sends. A Player
holding a stale session id would otherwise take down whatever was trying to talk
to them, at the worst possible moment.

`send_raw` skips wrapping, for pre-formatted content — a room description that
has already been laid out, a table, ASCII art. Colour is still processed.
