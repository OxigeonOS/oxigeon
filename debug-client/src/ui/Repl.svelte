<script>
  // The REPL over `evaluate`, and the console.
  //
  // Logpoint lines and breakpoint conditions that raised both arrive here. They
  // are drawn differently on purpose: a logpoint reporting is `·` in green, and
  // a `⚠` in yellow means something needs looking at — a condition that failed
  // to evaluate, a request the adapter refused, or a logpoint that hit its
  // per-dispatch limit.

  let { app } = $props()
  const dbg = $derived((app.dbgVersion, app.dbg))

  let field = $state(null)
  let body = $state(null)
  let history = []
  let historyPos = null

  $effect(() => {
    dbg.replLog.length
    dbg.output.length
    if (body) body.scrollTop = body.scrollHeight
  })

  $effect(() => {
    if (dbg.focus === 'repl') field?.focus()
  })

  function onKeydown(event) {
    event.stopPropagation()
    if (event.key === 'Enter') {
      const expr = dbg.replInput
      if (expr !== '') history.push(expr)
      historyPos = null
      dbg.submitRepl()
    } else if (event.key === 'ArrowUp') {
      event.preventDefault()
      if (!history.length) return
      historyPos = historyPos === null ? history.length - 1 : Math.max(0, historyPos - 1)
      dbg.replInput = history[historyPos]
    } else if (event.key === 'ArrowDown') {
      event.preventDefault()
      if (historyPos === null) return
      historyPos = historyPos >= history.length - 1 ? null : historyPos + 1
      dbg.replInput = historyPos === null ? '' : history[historyPos]
    }
  }
</script>

<section
  class="pane"
  class:focused={dbg.focus === 'repl'}
  style="grid-area: repl"
  onclick={() => {
    dbg.focus = 'repl'
    dbg.changed()
  }}
  role="presentation"
>
  <header>
    repl
    <span class="spacer"></span>
    {#if dbg.output.length}
      <button
        onclick={(e) => {
          e.stopPropagation()
          dbg.output.length = 0
          dbg.changed()
        }}>clear console</button
      >
    {/if}
  </header>

  <div class="body" bind:this={body}>
    {#each dbg.output as line, i (i)}
      <div class="row out" class:problem={line.important}>
        <span class="sigil">{line.important ? '⚠' : '·'}</span>
        <span class="text">{line.text}</span>
      </div>
    {/each}
    {#each dbg.replLog as entry, i (i)}
      <div class="row q"><span class="sigil">›</span><span class="text">{entry.expr}</span></div>
      <div class="row a" class:problem={entry.answer.startsWith('!')}>
        <span class="sigil"> </span><span class="text">{entry.answer}</span>
      </div>
    {/each}
  </div>

  <footer class="entry">
    <span class="caret" class:live={dbg.stopped}>›</span>
    <input
      bind:this={field}
      bind:value={dbg.replInput}
      onkeydown={onKeydown}
      placeholder={dbg.stopped ? 'a Lua expression in the selected frame' : 'evaluate needs a paused frame'}
      spellcheck="false"
      autocomplete="off"
    />
  </footer>
</section>

<style>
  .row {
    align-items: baseline;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .sigil {
    width: 1ch;
    flex: 0 0 auto;
  }

  .out .sigil {
    color: var(--green);
  }

  .out.problem .sigil,
  .out.problem .text {
    color: var(--yellow);
  }

  .q .sigil {
    color: var(--accent);
  }

  .q .text {
    color: var(--fg);
  }

  .a .text {
    color: #a5d6ff;
  }

  .a.problem .text {
    color: var(--red);
  }

  .entry {
    display: flex;
    align-items: baseline;
    gap: 6px;
    padding: 3px 8px;
  }

  .caret {
    color: var(--fg-faint);
  }

  .caret.live {
    color: var(--accent);
  }
</style>
