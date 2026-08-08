# Help & Authored Documentation

`help` is derived, never maintained. It reads the command registry and the
`game/docs/` tree on every invocation, so a list that has drifted out of date is
not a state it can be in — which is the only interesting property a help file
has.

It browses in two levels:

```
help                      the categories
help <category>           the commands and topics in one
help <command>            one command in detail
help <topic>              an authored page
help <category>/<topic>   when two categories hold the same topic name
help all                  every command, including ones you cannot use
```

---

## Where a category comes from

Two places, and they merge by name.

**A command declares one** with `M.category`, as it always has. The path on disk
is irrelevant: `cmds/admin/spawn.lua` is in `admin` because it says so.

**A game contributes one** by making a directory under `game/docs/`. Every file
in that directory is a topic:

```
game/docs/
  combat/
    stances.md        -->  help combat  -->  help stances
    parrying               (no extension: shown as written)
  lore/
    thornhollow.md    -->  help lore
    the-marsh.md
  README.md           -->  ignored; a topic needs a category
```

So `game/docs/combat/` puts `stances` beside `attack` and `flee` under one
**Combat** heading. The player is never shown which half came from where.

Three rules fall out of that, and each of them is a decision:

- **One level.** Directories directly under `docs/` are categories; files in
  them are topics. A file sitting loose in `docs/` is not reachable and is
  logged once per boot to `journal_d`, because a page nobody can open is
  indistinguishable from a page nobody wrote.
- **A command beats a topic of the same name.** `help attack` has to keep
  describing the verb you would type. The page is not lost — the command's
  detail ends with `See also: help combat/attack`.
- **An ambiguous bare topic name refuses**, and prints the categories holding
  it. Same rule as `2.rat` in `lib/matching.lua`, for the same reason: guessing
  wrong costs the player their next command.

### The `game:` prefix is the whole rule

`help` reads `list_dir("game:docs")`, never `list_dir("docs")`. Unprefixed,
every read efun searches **both roots**, game layer first — see
[File & System Access](./lua-api/file-access.md) — so the unprefixed spelling
would sweep a creator's `mudlib/docs/` into the player's help. That tree is the
system layer's own documentation and a different thing entirely.

### Permissions

There are none by default, and that is correct for a help system: `read_file`
and `list_dir` are ungated efuns and the shipped `config/permissions.toml` has
no rule for `/game/docs`.

A creator who wants a staff-only category adds one:

```toml
"/game/docs/staff" = { read = "docs.read.staff" }
```

`help` asks `dir_permission` and hides what you cannot read, rather than listing
it and then refusing — the pattern `file-access.md` documents.

---

## What Markdown becomes

`lib/markdown.lua` handles four constructs. A MUD screen is eighty columns of
monospace with no tables, no images and no hyperlinks; what is left of Markdown
is the part that carries structure.

| Source | On screen |
|---|---|
| `# Title` | `===[ Title ]===`, bold white |
| `## Section` | `=== Section ===`, bold white |
| `### Sub` (and `####`+) | `Sub`, bold white, unboxed |
| `- item` | an indented bullet, wrapped with a hanging indent |
| blank line | a blank line; runs of them collapse |
| anything else | paragraph text |

Consecutive non-blank lines are **one paragraph**, joined and re-wrapped to the
reader's terminal. That is what makes an eighty-column source file readable at
sixty columns instead of coming out as alternating long and stub lines.

Two things it deliberately does not do:

- **Nothing is eaten.** Code fences, `*emphasis*`, `[links](x)` and pipe tables
  arrive as ordinary text. A visible ` ``` ` is a typo an author can see and
  fix; a block that silently vanished is a bug they will stare at the source of.
  Same rule as an unknown `$token` in [Messages](./lua-api/messages.md).
- **`{colour}` tags pass through.** A help page is authored *content*, so
  `{red}Danger.{/}` is a feature. This is the opposite of `cat`, which pages
  files `literal` precisely because a source file's tags must not be rendered.
  The difference is whether the thing on screen is prose or code.

A topic file with **no extension** is not parsed at all — only wrapped. That is
for ASCII diagrams and tables, where reflowing a paragraph would leave the
arrows pointing at nothing.

### Colour and width

`markdown.lua` returns a string carrying `{tag}` markup, never ANSI, and `help`
sends it through `Player:send_paged`. That single path is what makes the output
honour the player's settings: `color off` strips the tags instead of rendering
them, `pagesize` chunks it, `pagesize 0` turns paging off, and the wrap width
comes from NAWS.

Wrapping uses `strings.wrap_tagged` rather than `strings.wrap`, because a
`{tag}` occupies no screen column. `wrap` counts them — a heading carrying
`{bold}{white}…{/}` would fold seventeen characters early and stop lining up
with the paragraph beneath it. `wrap` itself is unchanged; `wrap_tagged` sits
beside it, and also preserves leading whitespace and indents continuation lines,
which is what makes a wrapped bullet hang under its own text.

---

## Writing a page

Nothing to register. Make the directory, write the file, and it is in `help` on
the next invocation — there is no cache to flush and no reload to run.

```markdown
# Stances

A stance is how you are standing, not what you are doing.

## Choosing one

- **Balanced** — no modifier in either direction.
- **Aggressive** — you hit harder, and so does everything swinging at you.

{yellow}See also:{/} `help attack`, `help flee`.
```

`game.example/docs/` ships four such pages, including one with no extension.
