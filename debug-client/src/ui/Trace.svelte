<script>
  // Drives the in-game `trace` command on your session and lifts the block it
  // prints.
  //
  // This is text, not data, and is labelled as such: the trace rings live in a
  // thread-local on the Lua thread and are exposed only as pre-rendered strings
  // through the `trace_*` efuns, with no path out of the process. Structured
  // `trace_*_data` efuns would make this a real pane.

  import { css } from '../lib/ansi.js'

  let { app } = $props()

  const COMMANDS = [
    { key: 't', arg: 'time', label: 'time', hint: 'wall-clock per dispatch' },
    { key: 'c', arg: 'calls', label: 'calls', hint: 'record the dispatch chain' },
    { key: 'o', arg: 'off', label: 'off', hint: 'stop recording' },
    { key: 'r', arg: 'timings 20', label: 'timings', hint: 'the slowest 20' },
    { key: 's', arg: 'show 40', label: 'show', hint: 'the last 40 entries' },
    { key: 'x', arg: 'clear', label: 'clear', hint: 'empty the rings' },
  ]

  let body = $state(null)

  // The output arrives in the game stream, because that is where the command
  // printed it. Showing the tail here is the whole of the lift.
  const tail = $derived(app.scrollback.slice(-400))

  $effect(() => {
    tail.length
    if (body) body.scrollTop = body.scrollHeight
  })

  function onKeydown(event) {
    if (app.tab !== 'Trace') return
    if (event.target instanceof HTMLInputElement) return
    if (event.ctrlKey || event.altKey || event.metaKey) return
    const hit = COMMANDS.find((c) => c.key === event.key)
    if (!hit) return
    event.preventDefault()
    app.trace(hit.arg)
  }
</script>

<svelte:window on:keydown={onKeydown} />

<div class="trace">
  <section class="pane">
    <header>
      trace
      <span class="spacer"></span>
      <span class="faint">needs a character holding <code>admin</code> / <code>efun.trace</code></span>
    </header>
    <div class="commands">
      {#each COMMANDS as c (c.arg)}
        <button onclick={() => app.trace(c.arg)} title={c.hint}>
          <kbd>{c.key}</kbd>
          {c.label}
        </button>
      {/each}
    </div>
  </section>

  <section class="pane">
    <header>
      output
      <span class="spacer"></span>
      <!-- Said out loud rather than implied: this pane is the game's own text,
           not a structured feed. -->
      <span class="faint">the game stream — `trace` prints into it</span>
    </header>
    <div class="body" bind:this={body}>
      {#each tail as line, i (i)}
        <div class="line">
          {#each line as span}<span style={css(span.style)}>{span.text}</span>{/each}
        </div>
      {/each}
    </div>
  </section>
</div>

<style>
  .trace {
    flex: 1 1 auto;
    min-height: 0;
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    gap: 4px;
  }

  .commands {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    padding: 6px 8px;
  }

  .line {
    padding: 0 8px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  code {
    color: var(--fg-dim);
  }
</style>
