<script>
  // Source with a breakpoint gutter, Lua syntax highlighting, and the vi
  // motions — because that is what your hands already do.

  import { tokenize, withMatches } from '../lib/lua.js'

  let { app } = $props()
  const dbg = $derived((app.dbgVersion, app.dbg))

  let body = $state(null)
  let promptField = $state(null)
  let logpointField = $state(null)

  /// The line the current stop is on, in the file being shown. Only the frame
  /// under the cursor in the stack pane counts — the others are context.
  const stopLine = $derived.by(() => {
    const frame = dbg.frames[dbg.frameSel]
    if (!frame?.path || !dbg.stopped) return null
    return dbg.knownPath(frame.path) === dbg.open ? frame.line : null
  })

  const marks = $derived((app.dbgVersion, dbg.open ? dbg.linesFor(dbg.open) : new Map()))

  const rendered = $derived.by(() =>
    dbg.source.map((text, i) => ({
      n: i + 1,
      runs: withMatches(tokenize(text, dbg.blocks[i] ?? null), dbg.highlight ? dbg.search : ''),
    }))
  )

  function onKeydown(event) {
    if (dbg.focus !== 'source') return
    if (event.target instanceof HTMLInputElement) return
    const { key } = event
    const last = Math.max(0, dbg.source.length - 1)

    if (key === 'ArrowUp' || key === 'k') dbg.cursor = Math.max(0, dbg.cursor - 1)
    else if (key === 'ArrowDown' || key === 'j') dbg.cursor = Math.min(last, dbg.cursor + 1)
    else if (key === 'PageUp') dbg.cursor = Math.max(0, dbg.cursor - 20)
    else if (key === 'PageDown') dbg.cursor = Math.min(last, dbg.cursor + 20)
    else if (key === 'Home' || key === 'g') dbg.cursor = 0
    else if (key === 'End' || key === 'G') dbg.cursor = last
    // The two vi prompts. Both edit in the footer row, where the logpoint
    // editor already lives.
    else if (key === ':') dbg.sourcePrompt = { kind: 'goto', text: '' }
    else if (key === '/') dbg.sourcePrompt = { kind: 'search', text: '' }
    else if (key === 'n') dbg.find(true)
    else if (key === 'N') dbg.find(false)
    else return

    event.preventDefault()
    dbg.changed()
  }

  function promptKey(event) {
    if (event.key === 'Escape') {
      event.preventDefault()
      dbg.sourcePrompt = null
      dbg.changed()
    } else if (event.key === 'Enter') {
      event.preventDefault()
      dbg.commitSourcePrompt()
    }
    event.stopPropagation()
  }

  function logpointKey(event) {
    if (event.key === 'Escape') {
      event.preventDefault()
      dbg.cancelLogpoint()
    } else if (event.key === 'Enter') {
      event.preventDefault()
      dbg.commitLogpoint()
    }
    event.stopPropagation()
  }

  // Both editors take the keyboard the moment they open.
  $effect(() => {
    if (dbg.sourcePrompt) promptField?.focus()
  })
  $effect(() => {
    if (dbg.logpointEdit) logpointField?.focus()
  })

  $effect(() => {
    dbg.cursor
    dbg.open
    body?.querySelector('.cursor')?.scrollIntoView({ block: 'center' })
  })
</script>

<svelte:window on:keydown={onKeydown} />

<section class="pane" class:focused={dbg.focus === 'source'} style="grid-area: source">
  <header>
    <span class="path">{dbg.open ?? 'no file open'}</span>
    <span class="spacer"></span>
    {#if dbg.search && dbg.highlight}<span class="dim">/{dbg.search}</span>{/if}
    <span class="dim">{dbg.source.length ? `${dbg.cursor + 1}/${dbg.source.length}` : ''}</span>
  </header>

  <div class="body" bind:this={body}>
    {#each rendered as line (line.n)}
      {@const mark = marks.get(line.n)}
      {@const hasMark = marks.has(line.n)}
      <div class="line" class:cursor={line.n === dbg.cursor + 1} class:stop={line.n === stopLine}>
        <!-- The gutter marks a logpoint ◆ in cyan rather than ● in red, because
             it will never stop and a gutter that promised otherwise would be
             lying. -->
        <button
          class="gutter"
          class:bp={hasMark && mark === null}
          class:lp={hasMark && mark !== null}
          onclick={() => {
            dbg.cursor = line.n - 1
            dbg.toggleBreakpoint(line.n)
          }}
          oncontextmenu={(e) => {
            e.preventDefault()
            dbg.cursor = line.n - 1
            dbg.beginLogpoint(line.n)
          }}
          title={mark ? `logpoint: ${mark}` : 'click to break, right-click for a logpoint'}
        >
          {hasMark ? (mark === null ? '●' : '◆') : line.n}
        </button>
        <span
          class="text"
          onclick={() => {
            dbg.cursor = line.n - 1
            dbg.focus = 'source'
            dbg.changed()
          }}
          role="presentation"
        >
          {#each line.runs as run}<span
              class={run.kind}
              class:match={run.match}>{run.text}</span
            >{/each}
        </span>
      </div>
    {:else}
      <div class="empty faint">
        {dbg.pendingFile ? 'reading…' : 'pick a file, or break somewhere'}
      </div>
    {/each}
  </div>

  <footer>
    {#if dbg.logpointEdit}
      <!-- Whatever you type becomes a breakpoint that reports instead of
           stopping. The message is a **template**, not an expression: plain
           text is printed as written, only `{...}` is evaluated. An empty
           message removes it. -->
      <div class="editor">
        <span class="lp">logpoint {dbg.logpointEdit.line} ›</span>
        <input
          bind:this={logpointField}
          bind:value={dbg.logpointEdit.text}
          onkeydown={logpointKey}
          placeholder="{'{attacker.name}'} hits {'{target.name}'} for {'{raw}'} — empty removes it"
          spellcheck="false"
        />
      </div>
    {:else if dbg.sourcePrompt}
      <div class="editor">
        <span class="sigil">{dbg.sourcePrompt.kind === 'goto' ? ':' : '/'}</span>
        <input
          bind:this={promptField}
          bind:value={dbg.sourcePrompt.text}
          onkeydown={promptKey}
          spellcheck="false"
        />
        <span class="faint">
          {dbg.sourcePrompt.kind === 'goto' ? 'line number, or noh' : 'Enter finds, Esc abandons'}
        </span>
      </div>
    {:else}
      <kbd>:</kbd> line <kbd>/</kbd> search <kbd>n</kbd><kbd>N</kbd> next/prev
      <kbd>F9</kbd> break <kbd>^L</kbd> logpoint
    {/if}
  </footer>
</section>

<style>
  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    direction: rtl;
    text-align: left;
  }

  .line {
    display: flex;
    align-items: baseline;
    white-space: pre;
    line-height: var(--row);
  }

  .line.cursor {
    background: #ffffff0e;
  }

  .line.stop {
    background: #d2992244;
  }

  .gutter {
    flex: 0 0 auto;
    width: 5ch;
    text-align: right;
    padding: 0 8px 0 0;
    margin: 0;
    border: 0;
    border-radius: 0;
    color: var(--fg-faint);
    background: var(--bg-sunken);
    font-size: 11px;
  }

  .gutter:hover {
    color: var(--red);
    background: #ffffff10;
  }

  .gutter.bp {
    color: var(--red);
  }

  .gutter.lp {
    color: var(--cyan);
  }

  .text {
    padding: 0 8px;
    flex: 1 1 auto;
  }

  .empty {
    padding: 8px;
  }

  .keyword {
    color: #ff7b72;
  }
  .literal {
    color: #79c0ff;
  }
  .string {
    color: #a5d6ff;
  }
  .comment {
    color: #6a7580;
    font-style: italic;
  }
  .ident {
    color: #d2a8ff;
  }
  .plain {
    color: var(--fg);
  }

  /* Painted on top of the syntax colour, including inside a string or a
     comment — which is usually where you were looking. */
  .match {
    background: #d29922;
    color: #0e1116;
    border-radius: 2px;
  }

  .editor {
    display: flex;
    align-items: baseline;
    gap: 6px;
  }

  .sigil {
    color: var(--accent);
  }

  .lp {
    color: var(--cyan);
  }
</style>
