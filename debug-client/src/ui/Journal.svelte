<script>
  import { clock } from '../lib/journalfmt.js'

  let { app } = $props()

  let body = $state(null)
  let pinned = true

  /// The strip holds 2000 entries and is eight rows tall. Rendering all of them
  /// is four figures of DOM nodes rebuilt every time a line arrives — which for
  /// a server under load is several times a second — for the sake of scrollback
  /// nobody reaches by dragging a 9em-tall pane. The tail is what this is.
  const RENDERED = 300

  const shown = $derived.by(() => {
    const needle = app.journalFilter.trim().toLowerCase()
    const matched =
      needle === ''
        ? app.journal
        : app.journal.filter((e) => `${e.level} ${e.source} ${e.msg}`.toLowerCase().includes(needle))
    return matched.slice(-RENDERED)
  })

  // Follow the tail only while the reader is already at it.
  $effect(() => {
    shown.length
    if (pinned && body) body.scrollTop = body.scrollHeight
  })

  function onScroll() {
    if (!body) return
    pinned = body.scrollHeight - body.scrollTop - body.clientHeight < 20
  }
</script>

<section class="pane">
  <header>
    journal
    <span class="dim">{app.info?.journal ?? ''}</span>
    <span class="spacer"></span>
    <span class="dim">/</span>
    <input placeholder="filter" bind:value={app.journalFilter} />
    <span class="dim">{shown.length}</span>
  </header>
  <div class="body" bind:this={body} onscroll={onScroll}>
    {#each shown as entry, i (i)}
      <!-- A traceback arrives as one JSON line with embedded newlines; the strip
           shows the first line, and the Lua error is the first line. -->
      <div class="row" title={entry.msg}>
        <span class="faint">{clock(entry)}</span>
        <span class="level {entry.level}">{entry.level.padEnd(5)}</span>
        <span class="faint src">{entry.source}</span>
        <span class="msg">{entry.msg.split('\n')[0]}</span>
      </div>
    {/each}
  </div>
</section>

<style>
  .pane {
    height: 9.5em;
    flex: 0 0 auto;
  }

  .level {
    text-transform: lowercase;
  }
  .level.error {
    color: var(--red);
  }
  .level.warn {
    color: var(--yellow);
  }
  .level.info {
    color: var(--fg-dim);
  }
  .level.debug,
  .level.trace {
    color: var(--fg-faint);
  }
  .level.raw {
    color: var(--magenta);
  }

  .src {
    display: inline-block;
    width: 22ch;
    overflow: hidden;
    text-overflow: ellipsis;
    direction: rtl;
    text-align: left;
  }

  .msg {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  input {
    max-width: 18ch;
  }
</style>
